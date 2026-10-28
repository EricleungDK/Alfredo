#!/usr/bin/env python3
"""Reproducible harness eval: run fixed tasks through Alfredo or Claude Code and emit metrics.

    python3 alfredo-tui/eval/run_eval.py --agent alfredo --backend fixture --trials 3 --out out.json

Every trial starts from a fresh temp git repo (copied from tasks/<id>/repo) and, for Alfredo, a
fresh --state-dir. The verdict never comes from the screen: the candidate (Alfredo's
`alfredo/go-*` branch, or Claude Code's final working tree) is exported with `git archive` and
graded by the task's `check.sh`. See README.md in this directory. Standard library only.
"""
import argparse
import datetime
import http.client
import http.server
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import urllib.parse

HERE = Path(__file__).resolve().parent
TUI = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(TUI / 'tests'))

import code_health  # noqa: E402
import probes  # noqa: E402
import schema_check  # noqa: E402

SCHEMA_VERSION = 1
# No Bash: python3 can write anywhere the user can, so shell access would defeat "edits only in the
# temp repo". Read/Glob/Grep are read-only; Edit/Write are confined to the cwd by acceptEdits.
CLAUDE_TOOLS = 'Read,Edit,Write,Glob,Grep'
CLAUDE_ENV_KEYS = ('PATH', 'HOME', 'LANG', 'LC_ALL', 'TERM', 'TMPDIR', 'USER', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME',
                   'XDG_CACHE_HOME', 'CLAUDE_CONFIG_DIR', 'ANTHROPIC_API_KEY', 'ANTHROPIC_AUTH_TOKEN',
                   'ANTHROPIC_BASE_URL', 'CLAUDE_CODE_OAUTH_TOKEN', 'HTTPS_PROXY', 'HTTP_PROXY', 'NO_PROXY',
                   'https_proxy', 'http_proxy', 'no_proxy', 'SSL_CERT_FILE', 'NODE_EXTRA_CA_CERTS')
GIT_ENV = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
MALFORMED = re.compile(r'no FILE blocks|valid file plan|FILE marker|ended inside FILE block|more than once')
REFUSED = re.compile(r'unapproved file|outside approved paths')
SANDBOX_CODE = 'sandbox-unavailable'
# check.sh imports model-written code, so grading runs it under the same bwrap profile the worker uses
# (alfredo-tui/src/worker.rs check_request). The check script is bound read-only at GUEST_CHECK.
GUEST_CHECK = '/grader/check.sh'
GRADE_LIMITS = {'as': 8 * 1024 * 1024 * 1024, 'fsize': 32 * 1024 * 1024, 'nofile': 1024, 'nproc': 128}
SANDBOX_MISSING = ('grading sandbox unavailable: grading runs model-written code and needs bubblewrap and '
                   'util-linux (sudo apt install bubblewrap util-linux); refusing to run it unsandboxed. '
                   'Pass --unsafe-unsandboxed-grading only on a disposable machine.')
LOOPBACK = {'127.0.0.1', 'localhost', '::1'}
# Chat tasks (task.json "mode": "chat", issue #240) run one `--chat GOAL --headless` turn; the scripted
# approval policy approves edit_file/write_file inside the task workspace and denies everything else.
CHAT_POLICY = 'workspace-edits'
CHAT_READ_TOOLS = ('read_file', 'list_files', 'grep', 'git_status', 'git_branch', 'git_log', 'git_diff')
CHAT_EDIT_TOOLS = ('edit_file', 'write_file')

METRIC_NAMES = {
    'M1': ('pass@1', 'fraction'), 'M2': ('pass^3', 'fraction'),
    'M3': ('repair recovery rate', 'fraction of repairs accepted'),
    'M4': ('turns per task', 'model requests per trial (median, /go tasks; chat in secondary.chat)'),
    'M5': ('tool/command calls per task', 'worker runs + check runs per trial (median, /go tasks)'),
    'M6': ('tokens per task', 'prompt+completion tokens per trial (median, /go tasks)'),
    'M7': ('tool-call error rate', 'errors per 100 calls (/go tasks)'),
    'M8': ('harness overhead per turn', 'ms p50'),
    'M9': ('keystroke echo', 'ms p50/p95'), 'M10': ('frame render cost', 'ms p50/p95'),
    'M11': ('cold start to first frame', 'ms (median)'),
    'M12': ('panics/crashes', 'count'), 'M13': ('flaky tests', 'count'),
    'M14': ('crash recovery', 'fraction'), 'M15': ('code health', 'clippy::pedantic warnings (lib+bin); see secondary'),
    'M16': ('peak RSS during /go', 'kB, main process, median'),
}
FIXED_NULL = {
    'M13': 'flakiness needs repeated identical runs of one build; --trials only samples task outcomes (see M2)',
}
CODE_HEALTH_OFF = 'not measured: pass --code-health to compute clippy pedantic, unwrap/expect, module size and test counts'
PROBE_OFF = 'not measured: pass --probes (alfredo only) to run the harness probes'


class EvalError(Exception):
    """A usage or environment problem reported to the user without a traceback."""


# ----------------------------------------------------------------------------- tasks

def load_tasks(root, selector=None):
    """Load tasks/<id>/ directories; `selector` is a list of ids or id prefixes like T1."""
    found = {}
    for path in sorted((root / 'tasks').iterdir()):
        if not (path / 'task.json').is_file():
            continue
        meta = json.loads((path / 'task.json').read_text(encoding='utf-8'))
        found[path.name] = {
            'id': path.name, 'dir': path, 'title': meta['title'], 'kind': meta['kind'],
            'scripted_only': bool(meta.get('scripted_only')), 'mode': meta.get('mode', 'go'),
            'goal': (path / 'goal.md').read_text(encoding='utf-8').strip(),
            'answers': json.loads((path / 'answers.json').read_text(encoding='utf-8')),
            'check': path / 'check.sh',
        }
    if not selector:
        return list(found.values())
    chosen = []
    for want in selector:
        hits = [t for name, t in found.items() if name == want or name.split('-')[0] == want]
        if not hits:
            raise EvalError(f'Unknown task {want!r}; available: {", ".join(found)}')
        chosen += [t for t in hits if t not in chosen]
    return chosen


def git(repo, *args):
    return subprocess.run(['git', '-C', str(repo), *args], env=GIT_ENV, check=True, timeout=30,
                          capture_output=True, text=True).stdout.strip()


def make_repo(task, destination):
    """Fresh git repo containing exactly tasks/<id>/repo, committed on `main`."""
    shutil.copytree(task['dir'] / 'repo', destination)
    for args in (['init', '-q', '--template=', '--initial-branch=main'],
                 ['config', 'user.name', 'Eval'], ['config', 'user.email', 'eval@example.invalid'],
                 ['add', '-A'],
                 ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgSign=false', 'commit', '-qm', 'task baseline']):
        git(destination, *args)


def sandbox_tools():
    """(bwrap, prlimit) when grading can run sandboxed, else None.

    Only the trusted absolute installs count. PATH is never consulted: it can be attacker-influenced, and the
    worker runs the same two binaries from these exact paths (alfredo-tui/src/worker.rs).
    """
    bwrap, prlimit = '/usr/bin/bwrap', '/usr/bin/prlimit'
    return (bwrap, prlimit) if Path(bwrap).is_file() and Path(prlimit).is_file() else None


def grading_refusal(unsafe_unsandboxed=False):
    """SANDBOX_MISSING when grading must refuse to run model-written code, else None."""
    if sandbox_tools() is None and not unsafe_unsandboxed:
        return SANDBOX_MISSING
    return None


def sandbox_argv(export, check, bwrap, prlimit):
    """The bwrap argv that runs one check.sh: only `export` is writable, the host home and network are gone."""
    export = str(export)
    argv = [bwrap, '--die-with-parent', '--new-session', '--unshare-all', '--unshare-net',
            '--tmpfs', '/', '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp']
    for system in ('/usr', '/bin', '/sbin', '/lib', '/lib64', '/etc'):
        if Path(system).exists():
            argv += ['--ro-bind', system, system]
    argv += ['--bind', export, export, '--chdir', export, '--ro-bind', str(check), GUEST_CHECK, '--',
             prlimit, f'--as={GRADE_LIMITS["as"]}', f'--fsize={GRADE_LIMITS["fsize"]}',
             f'--nofile={GRADE_LIMITS["nofile"]}', f'--nproc={GRADE_LIMITS["nproc"]}', '--', 'sh', GUEST_CHECK]
    return argv


def _relative_inside(path):
    """True for a relative path that stays inside its root after lexical normalisation."""
    if path.startswith('/'):
        return False
    normal = os.path.normpath(path)
    return normal != '..' and not normal.startswith('../')


def check_archive(data):
    """Refuse archive members that could name, link or write outside the export (#207).

    Raises ValueError for absolute or `..` member names, symlinks whose target leaves the export,
    hardlinks to anything outside the archive, and device or fifo members. Only files, directories,
    symlinks and hardlinks are allowed.
    """
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        for member in archive.getmembers():
            name = member.name
            if name.startswith('/') or '..' in PurePosixPath(name).parts:
                raise ValueError(f'member {name!r} is not a relative path inside the export')
            if member.issym():
                if not _relative_inside(os.path.join(os.path.dirname(name), member.linkname)):
                    raise ValueError(f'symlink {name!r} points outside the export: {member.linkname!r}')
            elif member.islnk():
                if member.linkname.startswith('/') or '..' in PurePosixPath(member.linkname).parts:
                    raise ValueError(f'hardlink {name!r} points outside the export: {member.linkname!r}')
            elif not (member.isfile() or member.isdir()):
                raise ValueError(f'member {name!r} is a device or fifo, not allowed in an export')


def _links_escaping(export):
    """Symlinks in the extracted export whose real target lies outside it (checks chained links)."""
    base = os.path.realpath(export)
    for dirpath, dirnames, filenames in os.walk(export):
        for name in dirnames + filenames:
            path = os.path.join(dirpath, name)
            if os.path.islink(path):
                real = os.path.realpath(path)
                if real != base and not real.startswith(base + os.sep):
                    yield path


def grade(task, repo, ref, *, unsafe_unsandboxed=False):
    """Run the task's check.sh on `git archive ref` under the grading sandbox; never looks at a screen or a log.

    The candidate is model-written code, so check.sh runs under bwrap with only the export writable.
    Without bwrap grading refuses, unless `unsafe_unsandboxed` (--unsafe-unsandboxed-grading) is set.
    """
    if ref is None:
        return False, 'no candidate to grade'
    refusal = grading_refusal(unsafe_unsandboxed)
    if refusal:
        return False, refusal
    tools = sandbox_tools()
    export = Path(tempfile.mkdtemp(prefix='alfredo-eval-grade-', dir=repo.parent))
    try:
        archive = subprocess.run(['git', '-C', str(repo), 'archive', '--format=tar', ref], env=GIT_ENV,
                                 check=True, timeout=30, capture_output=True).stdout
        try:
            check_archive(archive)
        except (ValueError, tarfile.TarError) as error:
            return False, f'refused: {error}'
        subprocess.run(['tar', '-x', '-C', str(export)], input=archive, check=True, timeout=30)
        escaping = next(_links_escaping(export), None)
        if escaping:
            return False, f'refused: symlink {escaping} resolves outside the export'
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(export), 'LC_ALL': 'C.UTF-8',
               'PYTHONDONTWRITEBYTECODE': '1'}
        if tools:
            argv = sandbox_argv(export, task['check'], *tools)
        else:
            argv = ['sh', str(task['check'])]
        run = subprocess.run(argv, cwd=export, env=env, timeout=60, capture_output=True, text=True, errors='replace')
        tail = (run.stdout + run.stderr).strip().splitlines()[-6:]
        return run.returncode == 0, '\n'.join(tail)[:800]
    except subprocess.TimeoutExpired:
        return False, 'check.sh timed out'
    except subprocess.CalledProcessError as error:
        return False, f'export failed: {error}'
    finally:
        shutil.rmtree(export, ignore_errors=True)


def validate_tasks(tasks, *, unsafe_unsandboxed=False):
    """Every check.sh must fail on the untouched repo, or the grader would pass an empty answer."""
    refusal = grading_refusal(unsafe_unsandboxed)
    if refusal:
        return [refusal]
    problems = []
    for task in tasks:
        root = Path(tempfile.mkdtemp(prefix='alfredo-eval-validate-'))
        try:
            repo = root / 'repo'
            make_repo(task, repo)
            ok, output = grade(task, repo, 'HEAD', unsafe_unsandboxed=unsafe_unsandboxed)
            if ok:
                problems.append(f'{task["id"]}: check.sh passes on the untouched repo')
        finally:
            shutil.rmtree(root, ignore_errors=True)
    return problems


# ----------------------------------------------------------------------------- model server

class ModelServer:
    """Loopback Ollama-compatible endpoint that records every request Alfredo makes.

    Fixture mode answers from the task's answers.json. Proxy mode (`upstream`) forwards to a
    real Ollama and records the same facts, so both backends produce the same trial metrics.
    """

    def __init__(self, answers=None, upstream=None):
        self.answers, self.upstream = answers, upstream
        self.lock = threading.Lock()
        self.records, self.errors, self.used = [], [], {}
        self.hold_workers, self.worker_requests = None, 0  # M14 probe: park worker requests until released
        server = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                if server.upstream:
                    return server.passthrough(self, 'GET')
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(b'{"models":[{"name":"fixture"}]}')

            def do_POST(self):
                if self.path == '/api/generate':  # model preload: loads, generates nothing
                    if server.upstream:
                        return server.passthrough(self, 'POST')
                    self.rfile.read(int(self.headers.get('Content-Length', 0)))
                    body = b'{"done":true,"done_reason":"load"}\n'
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
                server.chat(self)

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'

    def passthrough(self, handler, method):
        length = int(handler.headers.get('Content-Length', 0))
        body = handler.rfile.read(length) if length else None
        conn = http.client.HTTPConnection(*self.upstream, timeout=1800)
        try:
            conn.request(method, handler.path, body, {'Content-Type': 'application/json'})
            response = conn.getresponse()
            data = response.read()
            handler.send_response(response.status)
            handler.send_header('Content-Type', response.getheader('Content-Type', 'application/json'))
            handler.send_header('Content-Length', str(len(data)))
            handler.end_headers()
            handler.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception as error:
            self.note_error(f'passthrough {handler.path}: {error!r}')
        finally:
            conn.close()

    def note_error(self, text):
        with self.lock:
            self.errors.append(text)

    def fixture_reply(self, kind, request, last):
        if kind == 'chat':
            # Round n of the tool loop = the number of tool-call rounds already in the request.
            rounds = sum(1 for m in request['messages'] if m.get('role') == 'assistant' and m.get('tool_calls'))
            script = self.answers['chat']
            return script[min(rounds, len(script) - 1)]
        if kind == 'planner':
            tasks = [dict(task, model=request.get('model', 'fixture')) for task in self.answers['plan']['tasks']]
            return json.dumps({'tasks': tasks})
        for index, entry in enumerate(self.answers['workers']):
            if last.startswith(entry['prefix']):
                with self.lock:
                    count = self.used.get(index, 0)
                    self.used[index] = count + 1
                reply = entry['replies'][min(count, len(entry['replies']) - 1)]
                return reply['prose'] if 'prose' in reply else json.dumps({'files': reply['files']})
        raise AssertionError(f'No scripted worker answer for prompt {last[:100]!r}')

    def chat(self, handler):
        record = {'kind': None, 'start': time.monotonic(), 'ttft': None, 'end': None, 'prompt': None, 'eval': None}
        try:
            request = json.loads(handler.rfile.read(int(handler.headers['Content-Length'])))
            last = request['messages'][-1]['content']
            if request.get('tools'):
                record['kind'] = 'chat'
            else:
                record['kind'] = 'worker' if last.startswith('Implement this task:') else 'planner'
            if record['kind'] == 'worker':
                with self.lock:
                    self.worker_requests += 1
                if self.hold_workers is not None:
                    self.hold_workers.wait(120)
            if self.upstream:
                self.forward(handler, request, record)
            else:
                content = self.fixture_reply(record['kind'], request, last)
                message = {'content': content}
                if isinstance(content, dict):  # a scripted chat round
                    calls = [{'function': {'name': c['name'], 'arguments': c['arguments']}}
                             for c in content.get('tool_calls', [])]
                    message = {'role': 'assistant', 'content': content.get('content', '')}
                    if calls:
                        message['tool_calls'] = calls
                    content = message['content'] + json.dumps(calls if calls else '')
                handler.send_response(200)
                handler.send_header('Content-Type', 'application/x-ndjson')
                handler.end_headers()
                # Deterministic synthetic counts (ceil(chars / 4)); a real server reports its own.
                chars = sum(len(m.get('content') or '') for m in request['messages'])
                done = {'message': message, 'done': True,
                        'prompt_eval_count': -(-chars // 4), 'eval_count': -(-len(content) // 4)}
                record['prompt'], record['eval'] = done['prompt_eval_count'], done['eval_count']
                handler.wfile.write((json.dumps(done) + '\n').encode())
                handler.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception as error:  # surfaced by the trial loop
            self.note_error(repr(error))
        finally:
            record['end'] = time.monotonic()
            if record['kind']:
                with self.lock:
                    self.records.append(record)

    def forward(self, handler, request, record):
        conn = http.client.HTTPConnection(*self.upstream, timeout=1800)
        try:
            conn.request('POST', handler.path, json.dumps(request).encode(), {'Content-Type': 'application/json'})
            response = conn.getresponse()
            handler.send_response(response.status)
            handler.send_header('Content-Type', response.getheader('Content-Type', 'application/x-ndjson'))
            handler.end_headers()
            while True:
                line = response.readline()
                if not line:
                    break
                if record['ttft'] is None:
                    record['ttft'] = time.monotonic() - record['start']
                handler.wfile.write(line)
                handler.wfile.flush()
                try:
                    final = json.loads(line)
                except ValueError:
                    continue
                if isinstance(final, dict) and final.get('done'):
                    record['prompt'] = final.get('prompt_eval_count')
                    record['eval'] = final.get('eval_count')
        finally:
            conn.close()

    def snapshot(self):
        with self.lock:
            return list(self.records)

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)


PLANNER_MISMATCH = 'planner_token_mismatch'


def model_stats(records, backend):
    """Turns, tokens, ttft and model seconds from recorded requests, with reasons for gaps."""
    unavailable = {}
    counted = [r for r in records if isinstance(r['prompt'], int) and isinstance(r['eval'], int)]
    tokens = {'prompt': None, 'eval': None, 'planner': None, 'peak_ctx': None}
    if records and len(counted) == len(records):
        tokens = {
            'prompt': sum(r['prompt'] for r in records), 'eval': sum(r['eval'] for r in records),
            'planner': sum(r['prompt'] + r['eval'] for r in records if r['kind'] == 'planner'),
            'peak_ctx': max(r['prompt'] + r['eval'] for r in records),
        }
    else:
        unavailable['tokens'] = f'the model server reported token counts for {len(counted)} of {len(records)} requests'
    ttfts = [r['ttft'] * 1000 for r in records if r['ttft'] is not None]
    ttft = round(statistics.median(ttfts), 1) if ttfts else None
    if ttft is None:
        unavailable['ttft_ms'] = ('the fixture replies instantly, so its first-byte time is not a model latency'
                                  if backend == 'fixture' else 'no streamed response was timed')
    model_s = sum(r['end'] - r['start'] for r in records)
    return {'turns': len(records), 'tokens': tokens, 'ttft_ms': ttft, 'model_s': model_s}, unavailable


# ----------------------------------------------------------------------------- state dir

def read_state(state):
    """Repairs, check executions and error classes from the on-disk Alfredo state."""
    repairs = repairs_accepted = tasks_accepted = 0
    for path in state.glob('rust-tasks-v1/*/tasks.json'):
        tasks = json.loads(path.read_text(encoding='utf-8')).get('tasks', [])
        fixes = [t for t in tasks if t.get('repair_of') is not None]
        recovered = {t['repair_of'] for t in fixes if t.get('status') == 'accepted'}
        repairs += len(fixes)
        repairs_accepted += sum(1 for t in fixes if t.get('status') == 'accepted')
        tasks_accepted += sum(1 for t in tasks if t.get('repair_of') is None
                              and (t.get('status') == 'accepted' or t['id'] in recovered))
    runs = len([p for p in state.glob('rust-tasks-v1/*/runs/*') if p.is_dir()])
    checks = len(list(state.glob('rust-tasks-v1/*/runs/*/check-result.json')))
    errors = {'malformed': 0, 'refused': 0, 'sandbox': 0}
    for path in state.glob('rust-tasks-v1/*/runs/*/evidence.json'):
        evidence = json.loads(path.read_text(encoding='utf-8'))
        detail = evidence.get('detail') or ''
        if evidence.get('failure_code') == SANDBOX_CODE:
            errors['sandbox'] += 1
        if MALFORMED.search(detail):
            errors['malformed'] += 1
        if REFUSED.search(detail):
            errors['refused'] += 1
    return {'repairs': repairs, 'repairs_accepted': repairs_accepted, 'tasks_accepted': tasks_accepted,
            'tool_calls': runs + checks, 'errors': errors, 'planner_tokens': planner_tokens(state)}


def planner_tokens(state):
    """Prompt + completion tokens Alfredo saved in its plan receipts (`usage`); None if it saved none."""
    total = None
    for path in state.glob('rust-tasks-v1/*/tasks.json'):
        for receipt in json.loads(path.read_text(encoding='utf-8')).get('receipts', []):
            action = receipt.get('request', {}).get('action', {})
            usage = (action.get('plan') or {}).get('usage') if action.get('kind') == 'plan' else None
            if usage:
                total = (total or 0) + usage.get('prompt_tokens', 0) + usage.get('completion_tokens', 0)
    return total


def planner_token_mismatch(server, saved):
    """Warning text when the planner tokens in state differ from what the server counted, else None."""
    if server is None or server == saved:
        return None
    return f'planner tokens: the server counted {server}, Alfredo saved {saved if saved is not None else "none"}'


def peak_rss_kb(pid):
    """Peak resident set (VmHWM) of one process; None when /proc cannot be read."""
    try:
        for line in Path(f'/proc/{pid}/status').read_text().splitlines():
            if line.startswith('VmHWM:'):
                return int(line.split()[1])
    except (OSError, ValueError):
        pass
    return None


# ----------------------------------------------------------------------------- trials

def blank_trial(task, index):
    return {
        'task': task['id'], 'kind': task['kind'], 'mode': task.get('mode', 'go'), 'trial': index, 'pass': False, 'outcome': 'harness_error',
        'graded_ref': None, 'grade_output': '', 'turns': None, 'tool_calls': None,
        'tokens': {'prompt': None, 'eval': None, 'planner': None, 'peak_ctx': None},
        'errors': {'malformed': None, 'refused': None, 'sandbox': None}, 'repairs': None, 'repairs_accepted': None,
        'tasks_accepted': None, 'enter_to_request_ms': None, 'wall_s': 0.0,
        'cold_start_ms': None, 'overhead_s': None, 'ttft_ms': None, 'rss_kb': None, 'exit_code': None,
        'panic': False, 'unavailable': {}, 'notes': [],
    }


def remove_tree(path):
    def writable(function, target, _info):
        # Unlinking an entry needs a writable parent; listing a directory needs it readable.
        os.chmod(os.path.dirname(target), 0o700)
        if os.path.isdir(target) and not os.path.islink(target):
            os.chmod(target, 0o700)
        function(target)
    shutil.rmtree(path, onerror=writable)


def alfredo_trial(task, index, options):
    from inference_terminal_smoke import Terminal  # reused PTY driver

    trial = blank_trial(task, index)
    root = Path(tempfile.mkdtemp(prefix='alfredo-eval-', dir=options.tmp_root))
    server = terminal = None
    try:
        workspace, state = root / 'workspace', root / 'state'
        make_repo(task, workspace)
        upstream = None
        if options.backend == 'ollama':
            parsed = urllib.parse.urlparse(options.ollama_endpoint)
            upstream = (parsed.hostname, parsed.port or 11434)
        server = ModelServer(answers=task['answers'], upstream=upstream)
        spawned = time.monotonic()
        terminal = Terminal(options.binary, server.endpoint, workspace, state, 'eval', model=options.model)
        deadline = spawned + 30
        while 'ALFREDO' not in terminal.screen():
            terminal.pump()
            if time.monotonic() > deadline:
                raise EvalError('Alfredo never drew its header within 30 s')
            time.sleep(0.0005)
        trial['cold_start_ms'] = round((time.monotonic() - spawned) * 1000, 1)

        goal = ' '.join(task['goal'].split())
        started = time.monotonic()
        terminal.send(f'/go {goal}\r')  # Enter is the last byte sent
        deadline = started + options.timeout
        rss = None
        while True:
            terminal.pump()
            rss = peak_rss_kb(terminal.process.pid) or rss
            screen = terminal.screen()
            if 'git switch alfredo/go-' in screen:
                trial['outcome'] = 'done'
                break
            if 'Autopilot ✗ failed' in screen:
                trial['outcome'] = 'failed'
                break
            if server.errors:
                trial['outcome'] = 'harness_error'
                trial['notes'].append(f'fixture error: {server.errors[0]}')
                break
            if time.monotonic() > deadline:
                trial['outcome'] = 'timeout'
                break
            time.sleep(0.03)
        trial['wall_s'] = round(time.monotonic() - started, 3)
        trial['rss_kb'] = rss
        if rss is None:
            trial['unavailable']['rss_kb'] = '/proc/PID/status was not readable'

        branches = git(workspace, 'for-each-ref', '--format=%(refname:short)', 'refs/heads/alfredo/').split()
        ref = branches[0] if trial['outcome'] == 'done' and branches else None
        trial['graded_ref'] = ref
        trial['pass'], trial['grade_output'] = grade(task, workspace, ref,
                                                     unsafe_unsandboxed=options.unsafe_unsandboxed_grading)

        terminal.expected_exit = True
        terminal.send(b'\x11')
        try:
            trial['exit_code'] = terminal.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            trial['notes'].append('did not exit on Ctrl+Q within 10 s; killed')
            terminal.kill()
            trial['exit_code'] = terminal.process.returncode
        terminal.pump()
        trial['panic'] = 'panicked at' in terminal.output.decode(errors='replace') or trial['exit_code'] == 101
        stats = read_state(state)
        model, unavailable = model_stats(server.snapshot(), options.backend)
        trial.update(turns=model['turns'], tokens=model['tokens'], ttft_ms=model['ttft_ms'],
                     tool_calls=stats['tool_calls'], errors=stats['errors'], repairs=stats['repairs'],
                     repairs_accepted=stats['repairs_accepted'], tasks_accepted=stats['tasks_accepted'],
                     overhead_s=round(max(0.0, trial['wall_s'] - model['model_s']), 3))
        trial['unavailable'].update(unavailable)
        mismatch = planner_token_mismatch(model['tokens']['planner'], stats['planner_tokens'])
        if mismatch:
            trial['notes'].append(f'{PLANNER_MISMATCH}: {mismatch}')
        firsts = [r['start'] for r in server.snapshot()]
        if firsts:
            trial['enter_to_request_ms'] = round((min(firsts) - started) * 1000, 1)
        else:
            trial['unavailable']['enter_to_request_ms'] = 'no model request reached the server'
    except AssertionError as error:  # Terminal.pump: the TUI exited by itself
        trial['outcome'] = 'crashed'
        trial['notes'].append(str(error)[:400])
        trial['exit_code'] = terminal.process.poll() if terminal else None
        trial['panic'] = bool(terminal and 'panicked at' in terminal.output.decode(errors='replace'))
    except Exception as error:  # one broken trial must not abort the whole eval
        trial['outcome'] = 'harness_error'
        trial['notes'].append(f'{type(error).__name__}: {error}'[:400])
    finally:
        if terminal:
            terminal.close()
        if server:
            server.close()
        if options.keep:
            print(f'kept {root}', file=sys.stderr)
        else:
            remove_tree(root)
    return trial


def chat_command(binary, endpoint, workspace, state, model, goal):
    """argv of one headless Chat Agent turn under the scripted, workspace-scoped approval policy."""
    return [str(binary), '--model', model, '--endpoint', endpoint, '--workspace', str(workspace),
            '--state-dir', str(state), '--chat', goal, '--headless', '--approve', CHAT_POLICY]


def _covers(read, edited):
    """True when a successful read_file/grep call looked at `edited` (grep without a path searches the root)."""
    path = (read.get('path') or '').strip().strip('/')
    if read['tool'] == 'read_file':
        return path == edited
    return path in ('', '.') or edited == path or edited.startswith(path + '/')


def chat_facts(result):
    """Trial facts from the `--chat --headless` JSON: calls, error classes, approvals and the read -> edit flow.

    `errors.malformed` counts only malformed calls (unknown tool, bad arguments), `errors.refused` denied
    approvals; well-formed calls a tool refused (text not found, missing or excluded file) and approved edits
    that failed are `tool_errors`. `read_before_edit`: a successful read_file or grep of the edited file came
    before the first applied edit.
    """
    calls = result.get('calls') or []
    approvals = result.get('approvals') or []
    names = [call['tool'] for call in calls]
    kinds = [call.get('result') or ('error' if call.get('error') else 'ok') for call in calls]
    first = next((i for i, call in enumerate(calls) if call['tool'] in CHAT_EDIT_TOOLS and kinds[i] == 'ok'), None)
    read_before_edit = first is not None and any(
        call['tool'] in ('read_file', 'grep') and kinds[i] == 'ok'
        and _covers(call, (calls[first].get('path') or '').strip().strip('/'))
        for i, call in enumerate(calls[:first]))
    return {
        'tool_calls': len(calls),
        'errors': {'malformed': kinds.count('malformed'), 'refused': kinds.count('denied'), 'sandbox': 0},
        'tool_errors': kinds.count('error'),
        'approvals': approvals,
        'chat': {'policy': result.get('policy'), 'reply': str(result.get('reply') or '')[:400],
                 'steps': result.get('steps'), 'calls': names, 'read_before_edit': read_before_edit,
                 'applied': sum(1 for a in approvals if a['outcome'] == 'applied')},
    }


def chat_verdict(passed, output, facts):
    """A chat trial passes only when check.sh passes AND the edited file was read before the first applied edit."""
    if passed and not (facts and facts['chat']['read_before_edit']):
        return False, ('check.sh passed, but read_before_edit is false: no successful read_file/grep of the '
                       'edited file before the first applied edit\n' + output)[:800]
    return passed, output


def chat_trial(task, index, options):
    """One `--chat GOAL --headless` turn; the working tree it leaves is committed and graded by check.sh."""
    trial = blank_trial(task, index)
    root = Path(tempfile.mkdtemp(prefix='alfredo-eval-', dir=options.tmp_root))
    server = process = None
    try:
        workspace, state = root / 'workspace', root / 'state'
        make_repo(task, workspace)
        upstream = None
        if options.backend == 'ollama':
            parsed = urllib.parse.urlparse(options.ollama_endpoint)
            upstream = (parsed.hostname, parsed.port or 11434)
        server = ModelServer(answers=task['answers'], upstream=upstream)
        argv = chat_command(options.binary, server.endpoint, workspace, state, options.model,
                            ' '.join(task['goal'].split()))
        started = time.monotonic()
        process = subprocess.Popen(argv, cwd=workspace, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True)
        rss = None
        out = err = ''
        while True:
            rss = peak_rss_kb(process.pid) or rss
            try:
                out, err = process.communicate(timeout=0.1)
                break
            except subprocess.TimeoutExpired:
                if time.monotonic() - started > options.timeout:
                    process.kill()
                    out, err = process.communicate()
                    trial['outcome'] = 'timeout'
                    break
        trial['wall_s'] = round(time.monotonic() - started, 3)
        trial['rss_kb'] = rss
        trial['exit_code'] = process.returncode
        trial['panic'] = 'panicked at' in err or process.returncode == 101
        if rss is None:
            trial['unavailable']['rss_kb'] = '/proc/PID/status was not readable'
        try:
            result = json.loads(out.strip().splitlines()[-1]) if out.strip() else None
        except ValueError:
            result = None
        if trial['outcome'] != 'timeout':
            if server.errors:
                trial['notes'].append(f'fixture error: {server.errors[0]}')
            elif result is None:
                trial['outcome'] = 'crashed' if trial['panic'] else 'harness_error'
                trial['notes'].append(f'no JSON result (exit {process.returncode}): {err.strip()[-300:]}')
            else:
                trial['outcome'] = 'done' if result.get('state') == 'done' else 'failed'
                if result.get('error'):
                    trial['notes'].append(f'chat: {result["error"]}'[:400])
        if result is not None:
            facts = chat_facts(result)
            trial.update(tool_calls=facts['tool_calls'], errors=facts['errors'], approvals=facts['approvals'],
                         chat=facts['chat'], tool_errors=facts['tool_errors'])
        model, unavailable = model_stats(server.snapshot(), options.backend)
        trial.update(turns=model['turns'], tokens=model['tokens'], ttft_ms=model['ttft_ms'],
                     overhead_s=round(max(0.0, trial['wall_s'] - model['model_s']), 3))
        trial['unavailable'].update(unavailable)
        trial['unavailable'].update({
            'repairs': 'a chat turn has no automatic-repair stage',
            'cold_start_ms': 'a headless chat turn draws no frame',
            'enter_to_request_ms': 'a headless chat turn has no Enter key',
        })
        # Same grader as every task: commit whatever the approved edits left in the tree, archive that commit.
        git(workspace, 'add', '-A')
        git(workspace, '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgSign=false', 'commit', '-qm',
            'chat result', '--allow-empty')
        trial['graded_ref'] = 'HEAD'
        passed, output = grade(task, workspace, 'HEAD', unsafe_unsandboxed=options.unsafe_unsandboxed_grading)
        trial['pass'], trial['grade_output'] = chat_verdict(passed, output, facts if result is not None else None)
        if passed and not trial['pass']:
            trial['notes'].append('flow: check.sh passed but the edited file was not read before the edit')
    except Exception as error:  # one broken trial must not abort the whole eval
        trial['outcome'] = 'harness_error'
        trial['notes'].append(f'{type(error).__name__}: {error}'[:400])
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        if server:
            server.close()
        if options.keep:
            print(f'kept {root}', file=sys.stderr)
        else:
            remove_tree(root)
    return trial


def recovery_trial(task, options):
    """M14: kill -9 mid-generation, restart (must come back paused), /resume must accept and grade pass."""
    from inference_terminal_smoke import Terminal

    root = Path(tempfile.mkdtemp(prefix='alfredo-eval-recovery-', dir=options.tmp_root))
    server = terminal = None
    try:
        workspace, state = root / 'workspace', root / 'state'
        make_repo(task, workspace)
        server = ModelServer(answers=task['answers'])
        server.hold_workers = threading.Event()

        def until(predicate, timeout, label):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                terminal.pump()
                if server.errors:
                    raise EvalError(f'fixture error: {server.errors[0]}')
                if predicate():
                    return
                time.sleep(0.02)
            raise EvalError(f'timed out waiting for {label}')

        terminal = Terminal(options.binary, server.endpoint, workspace, state, 'eval', model=options.model)
        until(lambda: 'ALFREDO' in terminal.screen(), 30, 'the header')
        terminal.send(f'/go {" ".join(task["goal"].split())}\r')
        until(lambda: server.worker_requests >= 1, 60, 'the worker request')
        terminal.kill()  # kill -9: no cleanup, no evidence
        terminal.close()
        server.hold_workers.set()
        terminal = Terminal(options.binary, server.endpoint, workspace, state, 'eval', resume=True,
                            model=options.model)
        until(lambda: 'Autopilot ‖ paused' in terminal.screen() and 'interrupted' in terminal.screen(), 30,
              'the restarted TUI to show the run paused and interrupted')
        terminal.send('/resume\r')
        until(lambda: 'git switch alfredo/go-' in terminal.screen(), 60, '/resume to accept the task')
        branches = git(workspace, 'for-each-ref', '--format=%(refname:short)', 'refs/heads/alfredo/').split()
        return bool(branches) and grade(task, workspace, branches[0],
                                        unsafe_unsandboxed=options.unsafe_unsandboxed_grading)[0]
    except (EvalError, AssertionError) as error:
        print(f'recovery probe: {str(error)[:200]}', file=sys.stderr)
        return False
    finally:
        if terminal:
            terminal.close()
        if server:
            server.hold_workers.set()
            server.close()
        remove_tree(root)


def run_probes(options, tasks):
    """All `--probes` measurements; each key is a result dict or {'error': reason}."""
    binary, results = options.binary, {}
    task = next((t for t in tasks if t['kind'] == 'positive' and t.get('mode') != 'chat'), None)

    def recovery():
        if task is None:
            return {'error': 'no positive task selected for the recovery probe'}
        return {'recovered': [recovery_trial(task, options) for _ in range(options.probe_recovery_trials)]}

    steps = [('chat', 'chat latency', lambda: probes.run_chat(binary, options.probe_chats, options.model,
                                                              options.tmp_root)),
             ('echo', 'keystroke echo', lambda: probes.run_echo(binary, options.probe_keys, options.model,
                                                                options.tmp_root)),
             ('recovery', 'crash recovery', recovery)]
    if options.render_probe:
        steps.append(('render', 'render cost (release cargo test; slow on a cold build)',
                      lambda: probes.run_render(TUI)))
    for key, label, run in steps:
        print(f'probe: {label}', file=sys.stderr)
        try:
            results[key] = run()
        except Exception as error:  # one broken probe must not lose the whole eval
            results[key] = {'error': repr(error)[:200]}
    return results


def parse_claude_stream(lines):
    """Facts from Claude Code's stream-json events (testable without running Claude)."""
    facts = {'turns': None, 'tool_calls': 0, 'tokens': None, 'refused': 0, 'malformed': 0, 'error': None,
             'version': None}
    for line in lines:
        try:
            event = json.loads(line)
        except ValueError:
            continue
        kind = event.get('type')
        if kind == 'system' and event.get('subtype') == 'init':
            facts['version'] = event.get('claude_code_version')
        elif kind == 'assistant':
            for block in event.get('message', {}).get('content', []):
                if isinstance(block, dict) and block.get('type') == 'tool_use':
                    facts['tool_calls'] += 1
        elif kind == 'user':
            content = event.get('message', {}).get('content', [])
            for block in content if isinstance(content, list) else []:
                if isinstance(block, dict) and block.get('type') == 'tool_result' and block.get('is_error'):
                    text = json.dumps(block.get('content'))
                    if 'requested permissions' in text or 'permission' in text.lower():
                        facts['refused'] += 1
                    elif 'InputValidationError' in text or 'String to replace not found' in text:
                        facts['malformed'] += 1
        elif kind == 'result':
            facts['turns'] = event.get('num_turns')
            usage = event.get('usage') or {}
            prompt = sum(usage.get(key, 0) or 0 for key in
                         ('input_tokens', 'cache_creation_input_tokens', 'cache_read_input_tokens'))
            facts['tokens'] = {'prompt': prompt, 'eval': usage.get('output_tokens', 0) or 0}
            if event.get('is_error'):
                facts['error'] = str(event.get('result') or event.get('subtype'))[:300]
    return facts


def claude_trial(task, index, options):
    trial = blank_trial(task, index)
    root = Path(tempfile.mkdtemp(prefix='alfredo-eval-', dir=options.tmp_root))
    process = None
    try:
        workspace = root / 'workspace'
        make_repo(task, workspace)
        command = ['claude', '-p', '--model', options.model, '--output-format', 'stream-json', '--verbose',
                   '--max-turns', '30', '--permission-mode', 'acceptEdits', '--allowedTools', CLAUDE_TOOLS,
                   '--disallowedTools', 'WebFetch,WebSearch', '--no-session-persistence',
                   '--setting-sources', 'project', '--strict-mcp-config']
        env = {key: os.environ[key] for key in CLAUDE_ENV_KEYS if key in os.environ}
        spawned = time.monotonic()
        process = subprocess.Popen(command, cwd=workspace, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, env=env)
        out, err, first = [], [], []

        def drain(stream, sink, stamp):
            for line in stream:
                if stamp and not first:
                    first.append(time.monotonic())
                sink.append(line)

        threads = [threading.Thread(target=drain, args=(process.stdout, out, True), daemon=True),
                   threading.Thread(target=drain, args=(process.stderr, err, False), daemon=True)]
        for thread in threads:
            thread.start()
        process.stdin.write(task['goal'] + '\n')
        process.stdin.close()
        rss, timed_out = None, False
        while process.poll() is None:
            rss = peak_rss_kb(process.pid) or rss
            if time.monotonic() - spawned > options.timeout:
                timed_out = True
                process.kill()
                break
            time.sleep(0.1)
        process.wait()
        for thread in threads:
            thread.join(timeout=5)
        trial['wall_s'] = round(time.monotonic() - spawned, 3)
        trial['rss_kb'] = rss
        trial['exit_code'] = process.returncode
        if first:
            trial['cold_start_ms'] = round((first[0] - spawned) * 1000, 1)
        facts = parse_claude_stream(out)
        trial['outcome'] = 'timeout' if timed_out else ('done' if process.returncode == 0 else 'failed')
        if facts['error'] or process.returncode:
            trial['notes'].append(f'claude: {facts["error"] or "".join(err)[-300:].strip()}')
        trial['turns'], trial['tool_calls'] = facts['turns'], facts['tool_calls']
        trial['errors'] = {'malformed': facts['malformed'], 'refused': facts['refused'], 'sandbox': 0}
        if facts['tokens']:
            trial['tokens'].update(prompt=facts['tokens']['prompt'], eval=facts['tokens']['eval'])
        trial['unavailable'] = {
            'repairs': 'Claude Code has no automatic-repair stage',
            'overhead_s': 'harness overhead cannot be separated from model time in Claude Code',
            'ttft_ms': 'stream-json does not expose time to first token',
            'tokens.planner,tokens.peak_ctx': 'Claude Code has no planner phase and reports no per-request context '
                                              'size (tokens.prompt and tokens.eval are totals from the result event)',
        }
        # Same grader as Alfredo: commit whatever is in the working tree, then archive that commit.
        git(workspace, 'add', '-A')
        git(workspace, '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgSign=false', 'commit', '-qm',
            'claude-code result', '--allow-empty')
        trial['graded_ref'] = 'HEAD'
        trial['pass'], trial['grade_output'] = grade(task, workspace, 'HEAD',
                                                     unsafe_unsandboxed=options.unsafe_unsandboxed_grading)
    except Exception as error:  # one broken trial must not abort the whole eval
        trial['outcome'] = 'harness_error'
        trial['notes'].append(f'{type(error).__name__}: {error}'[:400])
    finally:
        if process is not None and process.poll() is None:  # also on KeyboardInterrupt
            process.kill()
            process.wait()
        if options.keep:
            print(f'kept {root}', file=sys.stderr)
        else:
            remove_tree(root)
    return trial


# ----------------------------------------------------------------------------- aggregation

def pass_hat_k(per_task, k):
    """Mean over tasks of P(k random distinct trials all pass) = C(c,k)/C(n,k); None if n < k."""
    from math import comb
    values = []
    for passes, total in per_task:
        if total < k:
            return None
        values.append(comb(passes, k) / comb(total, k))
    return sum(values) / len(values) if values else None


def binary_profile(binary):
    """Cargo profile from the binary's immediate parent directory (`.../<profile>/alfredo-tui`), else 'unknown'."""
    parent = Path(binary).parent.name
    return parent if parent in ('release', 'debug') else 'unknown'


def probe_metrics(metric, results, go_enter, debug_note):
    """M8, M9, M10 and M14 from the `--probes` results (None: probes were not run, so the rows are null)."""
    def row(key, name, build, note=None):
        result = (results or {}).get(name)
        if results is None:
            return metric(key, None, PROBE_OFF, secondary=go_enter if key == 'M8' else None)
        if result is None:
            return metric(key, None, 'skipped (--no-render-probe)' if name == 'render' else f'{name} probe not run',
                          secondary=go_enter if key == 'M8' else None)
        if 'error' in result:
            return metric(key, None, result['error'], secondary=go_enter if key == 'M8' else None)
        value, secondary = build(result)
        return metric(key, value, 'no samples' if value is None else None, secondary=secondary, note=note)

    def m8(chat):
        r2s, e2r = probes.stats(chat['reply_to_screen_ms']), probes.stats(chat['enter_to_request_ms'])
        return r2s['p50'], {**go_enter, 'reply_to_screen_p95_ms': r2s['p95'], 'chat_enter_to_request_p50_ms': e2r['p50'],
                            'chat_enter_to_request_p95_ms': e2r['p95'], 'chats': r2s['n']}

    def m9(echo):
        s = probes.stats(echo['echo_ms'])
        return s['p95'], {'echo_p50_ms': s['p50'], 'keys': s['n']}

    def m10(render):
        mission = render['mission_work']
        return render['chat']['warm_p95_ms'], {
            'chat_warm_p50_ms': render['chat']['warm_p50_ms'], 'dashboard_average_ms': render['dashboard_average_ms'],
            'mission_work_warm_p95_ms': {k: v['warm_p95'] for k, v in mission.items()},
            'mission_work_cold_p95_ms': {k: v['cold_p95'] for k, v in mission.items()}}

    def m14(recovery):
        done = recovery['recovered']
        return (sum(done) / len(done) if done else None), {'trials': len(done), 'recovered': sum(done)}

    tail = '; ' + debug_note if debug_note else ''
    return {
        'M8': row('M8', 'chat', m8, note='value is reply to screen p50 (server send to marker in raw pty bytes); '
                  'enter_to_request_p50_ms is the /go trial figure (includes preflight and planning), '
                  'chat_enter_to_request_* is the plain-chat Enter-to-request from the probe' + tail),
        'M9': row('M9', 'echo', m9, note='value is the p95 from keystroke to first pty output byte, over keys into an idle input' + tail),
        'M10': row('M10', 'render', m10, note='value is chat warm p95 ms; release `cargo test` of tests/rendering_cost.rs '
                   '(TestBackend draw, no terminal IO), independent of --binary'),
        'M14': row('M14', 'recovery', m14, note='value is the share of trials where kill -9 during a worker request, '
                   'restart (paused, interrupted) and /resume ended accepted and graded pass' + tail),
    }


def code_health_metric(metric, result):
    """M15 row: value is the clippy::pedantic count; `secondary` carries the other parts (see code_health.py)."""
    if result is None:
        return metric('M15', None, CODE_HEALTH_OFF)
    reasons = result.get('reasons', {})
    keys = ('pedantic_lib', 'pedantic_bin', 'unwrap_expect_nontest', 'largest_module', 'total_loc', 'rust_tests',
            'rust_tests_ignored', 'py_tests')
    secondary = {key: result.get(key) for key in keys}
    notes = [f'{part}: {why}' for part, why in reasons.items() if part != 'pedantic']
    return metric('M15', result.get('pedantic'), reasons.get('pedantic'), secondary=secondary,
                  note='; '.join(notes + ['non-test = src/**/*.rs minus #[cfg(test)]/all(test..)/any(test..) items '
                                           '(brace matched); tests/ excluded; see README M15']))


def aggregate(trials, tasks, config, probes_result=None, code_health_result=None):
    """Per-task summary, M1..M16 and warnings from the trial list."""
    positives = [t for t in trials if t['kind'] == 'positive']
    # M4-M7 and M16 headline go-mode (/go) tasks only, so adding a chat task never moves the scoreboard (issue #284);
    # chat positives report under `secondary.chat`. A record without `mode` (a result file from before the field)
    # takes its task's mode from the task list, else counts as go. M1/M2 stay over every positive task.
    task_modes = {task['id']: task.get('mode') or 'go' for task in tasks}
    def mode_of(t):
        return t.get('mode') or task_modes.get(t['task'], 'go')

    go_positives = [t for t in positives if mode_of(t) != 'chat']
    chat_positives = [t for t in positives if mode_of(t) == 'chat']
    summary, warnings = {}, []
    for task in tasks:
        mine = [t for t in trials if t['task'] == task['id']]
        passes = sum(t['pass'] for t in mine)
        expected = 'none' if config['backend'] != 'fixture' else ('pass' if task['kind'] == 'positive' else 'fail')
        as_expected = None if expected == 'none' or not mine else all(t['pass'] == (expected == 'pass') for t in mine)
        summary[task['id']] = {'kind': task['kind'], 'trials': len(mine), 'passes': passes,
                               'pass_rate': round(passes / len(mine), 4) if mine else None,
                               'expected': expected, 'as_expected': as_expected}
        if task['kind'] == 'positive' and mine and passes == 0:
            warnings.append({'code': 'task_zero_pass', 'task': task['id'],
                             'message': f'{task["id"]} passed 0 of {len(mine)} trials: likely a broken task or grader'})
        if task['kind'] == 'negative_control' and passes:
            warnings.append({'code': 'control_passed', 'task': task['id'],
                             'message': f'negative control {task["id"]} passed {passes} of {len(mine)} trials: '
                                        'the grader accepts a wrong answer'})
        if as_expected is False:
            warnings.append({'code': 'fixture_mismatch', 'task': task['id'],
                             'message': f'{task["id"]} did not grade as scripted ({expected}) in every trial'})
    likely_broken = any(w['code'] == 'control_passed' for w in warnings)
    if positives and not any(t['pass'] for t in positives):
        likely_broken = True
        warnings.append({'code': 'zero_pass_rate',
                         'message': '0% pass rate on every positive task: likely a broken task or grader '
                                    '(or the agent cannot reach the endpoint); check grade_output and notes'})
    for t in trials:
        if t['panic']:
            warnings.append({'code': 'panic', 'task': t['task'], 'message': f'{t["task"]} trial {t["trial"]} panicked'})
        warnings.extend({'code': PLANNER_MISMATCH, 'task': t['task'], 'message': f'{t["task"]} trial {t["trial"]} {note}'}
                        for note in t['notes'] if note.startswith(PLANNER_MISMATCH))
        if t['outcome'] == 'harness_error':
            warnings.append({'code': 'harness_error', 'task': t['task'],
                             'message': f'{t["task"]} trial {t["trial"]}: ' + '; '.join(t['notes'])})

    def metric(key, value, reason=None, digits=3, note=None, secondary=None):
        name, unit = METRIC_NAMES[key]
        if isinstance(value, float):
            value = round(value, digits)
        result = {'name': name, 'value': value, 'unit': unit, 'reason': None if value is not None else reason}
        if note:
            result['note'] = note
        if secondary:
            def rounded(v):
                if isinstance(v, dict):
                    return {k: rounded(x) for k, x in v.items()}
                return round(v, digits) if isinstance(v, float) else v
            result['secondary'] = {k: rounded(v) for k, v in secondary.items()}
        return result

    def over(field, reducer, getter=None, pool=None):
        pool = go_positives if pool is None else pool
        get = getter or (lambda t: t[field])
        values = [get(t) for t in pool]
        have = [v for v in values if v is not None]
        if have:
            return reducer(have), None
        reasons = [t['unavailable'].get(field) for t in pool if t['unavailable'].get(field)]
        return None, (reasons[0] if reasons else f'no positive trial produced {field}')

    def chat_block(value):
        return {'chat': {'value': value, 'trials': len(chat_positives)}} if chat_positives else {}

    def per_task_median(getter):
        # Global medians hide one task regressing (T2/T3 vanish behind T1/T5/T6), so also report each task's own.
        result = {}
        for task in tasks:
            if task['kind'] != 'positive':
                continue
            values = [getter(t) for t in positives if t['task'] == task['id']]
            have = [v for v in values if v is not None]
            result[task['id']] = statistics.median(have) if have else None
        return result

    profile, binary = config.get('profile'), config.get('binary')
    binary_path = Path(binary) if binary else None
    binary_kb = binary_path.stat().st_size // 1024 if binary_path and binary_path.is_file() else None
    debug_note = None
    if profile == 'debug':
        debug_note = ('measured on a debug build: not comparable with the release baselines; '
                      'rerun with --binary target/release/alfredo-tui')
        warnings.append({'code': 'debug_profile',
                         'message': 'binary is a debug build: M11 cold start and M16 peak RSS'
                                    + (', and the probe rows M8, M9 and M14,' if config.get('probes') else '')
                                    + ' are not comparable with the release baselines (scoreboard #159); pass --binary '
                                    'target/release/alfredo-tui'})

    mean = statistics.fmean
    metrics = {}
    if positives:
        metrics['M1'] = metric('M1', sum(t['pass'] for t in positives) / len(positives))
        per_task = [(sum(t['pass'] for t in positives if t['task'] == task['id']),
                     sum(1 for t in positives if t['task'] == task['id']))
                    for task in tasks if task['kind'] == 'positive']
        m2 = pass_hat_k(per_task, 3)
        metrics['M2'] = metric('M2', m2, f'needs at least 3 trials per task (got {config["trials"]})')
    else:
        metrics['M1'] = metric('M1', None, 'no positive task was run')
        metrics['M2'] = metric('M2', None, 'no positive task was run')
    known = [t for t in positives if t['repairs'] is not None]
    attempted = sum(t['repairs'] for t in known)
    accepted_tasks = sum(t['tasks_accepted'] or 0 for t in known)
    value, reason = over('repairs', mean)
    if value is not None:
        value = sum(t['repairs_accepted'] or 0 for t in known) / attempted if attempted else None
        reason = 'no repair attempted'
    repair_ratio = {}
    for task in tasks:
        if task['kind'] != 'positive':
            continue
        mine = [t for t in known if t['task'] == task['id']]
        accepted = sum(t['tasks_accepted'] or 0 for t in mine)
        repair_ratio[task['id']] = sum(t['repairs'] for t in mine) / accepted if accepted else None
    metrics['M3'] = metric('M3', value, reason, secondary={
        'repairs_per_accepted_task': attempted / accepted_tasks if accepted_tasks else None,
        'repairs_attempted': attempted if known else None,
        'per_task': repair_ratio})
    value, reason = over('turns', statistics.median)
    chat_value, _ = over('turns', statistics.median, pool=chat_positives)
    metrics['M4'] = metric('M4', value, reason, secondary={'per_task': per_task_median(lambda t: t['turns']),
                                                           **chat_block(chat_value)})
    value, reason = over('tool_calls', statistics.median)
    chat_value, _ = over('tool_calls', statistics.median, pool=chat_positives)
    metrics['M5'] = metric('M5', value, reason, secondary={'per_task': per_task_median(lambda t: t['tool_calls']),
                                                           **chat_block(chat_value)})

    def total_tokens(t):
        return None if t['tokens']['prompt'] is None else t['tokens']['prompt'] + t['tokens']['eval']

    value, reason = over('tokens', statistics.median, total_tokens)
    chat_value, _ = over('tokens', statistics.median, total_tokens, chat_positives)
    peaks = [t['tokens']['peak_ctx'] for t in go_positives if t['tokens']['peak_ctx'] is not None]
    metrics['M6'] = metric('M6', value, reason, secondary={'peak_ctx_max': max(peaks) if peaks else None,
                                      'per_task': per_task_median(total_tokens), **chat_block(chat_value)},
                           note='fixture token counts are synthetic (ceil(chars / 4)), deterministic, not model counts'
                           if config['backend'] == 'fixture' else None)
    def error_rate(pool):
        scored = [t for t in pool if t['errors']['malformed'] is not None and t['tool_calls'] is not None]
        calls = sum(t['tool_calls'] for t in scored)
        errors = sum(sum(t['errors'].values()) for t in scored)
        return 100 * errors / calls if calls else None

    metrics['M7'] = metric('M7', error_rate(go_positives), 'no tool calls were made',
                           secondary=chat_block(error_rate(chat_positives)))
    enter = [t['enter_to_request_ms'] for t in go_positives if t['enter_to_request_ms'] is not None]
    go_enter = {'enter_to_request_p50_ms': statistics.median(enter) if enter else None}
    metrics.update(probe_metrics(metric, probes_result, go_enter, debug_note))
    value, reason = over('cold_start_ms', statistics.median)
    metrics['M11'] = metric('M11', value, reason, digits=1, note=debug_note)
    abnormal = sum(1 for t in trials if t['panic'] or t['outcome'] == 'crashed'
                   or t['exit_code'] not in (0, None))
    metrics['M12'] = metric('M12', abnormal, note='counts eval trials only (panic, crash or abnormal exit); '
                            'the smoke suites are not included')
    value, reason = over('rss_kb', statistics.median)
    chat_value, _ = over('rss_kb', statistics.median, pool=chat_positives)
    metrics['M16'] = metric(
        'M16', value, reason, digits=0,
        secondary={'binary_kb': binary_kb, 'release_binary_kb': binary_kb if profile == 'release' else None,
                   **chat_block(chat_value)},
        note='main process VmHWM only (process-tree peak is not sampled); binary_kb is the size of the binary '
             'under test (config.binary), release_binary_kb the same only for a release build'
             + (f'; {debug_note}' if debug_note else ''))
    for key, why in FIXED_NULL.items():
        metrics[key] = metric(key, None, why)
    metrics['M15'] = code_health_metric(metric, code_health_result)
    return summary, {key: metrics[key] for key in sorted(metrics, key=lambda k: int(k[1:]))}, warnings, likely_broken


# ----------------------------------------------------------------------------- CLI

def preflight_ollama(endpoint, model):
    parsed = urllib.parse.urlparse(endpoint)
    if parsed.hostname not in LOOPBACK:
        raise EvalError(f'Ollama mode is local-only; {endpoint} is not a loopback address')
    try:
        conn = http.client.HTTPConnection(parsed.hostname, parsed.port or 11434, timeout=5)
        conn.request('GET', '/api/tags')
        names = [m.get('name') for m in json.loads(conn.getresponse().read()).get('models', [])]
    except (OSError, ValueError, http.client.HTTPException) as error:
        raise EvalError(f'Ollama is not reachable at {endpoint} ({error}). Start it with `ollama serve`, '
                        'or pass --ollama-endpoint.') from error
    if model not in names and f'{model}:latest' not in names:
        raise EvalError(f'Ollama at {endpoint} does not have model {model!r}. Run `ollama pull {model}`; '
                        f'installed: {", ".join(n for n in names if n) or "none"}')


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--agent', choices=['alfredo', 'claude-code'], required=True)
    parser.add_argument('--backend', choices=['fixture', 'ollama'],
                        help='alfredo only: scripted fixture (default, deterministic) or a local Ollama')
    parser.add_argument('--model', help='ollama model name (required for --backend ollama); '
                                        'claude-code model alias (default haiku)')
    parser.add_argument('--ollama-endpoint', default='http://127.0.0.1:11434')
    parser.add_argument('--trials', type=int, default=3)
    parser.add_argument('--tasks', help='comma-separated task ids or prefixes (T1,T4); default all')
    parser.add_argument('--out', help='write the JSON here (default: stdout)')
    parser.add_argument('--timeout', type=int, help='seconds per trial (fixture 120, ollama 900, claude-code 600)')
    parser.add_argument('--binary', help='alfredo-tui binary (default $ALFREDO_TUI_BINARY or target/debug; use target/release '
                        'for M11/M16 that compare with the baselines)')
    parser.add_argument('--tmp-root', help='directory for temp repos and state (default: system temp)')
    parser.add_argument('--keep', action='store_true', help='keep temp repos for debugging')
    parser.add_argument('--validate-tasks', action='store_true',
                        help='only check that every check.sh fails on its untouched repo, then exit')
    parser.add_argument('--unsafe-unsandboxed-grading', action='store_true',
                        help='grade model-written code on the host when bubblewrap is missing (never needed when '
                             'bwrap is installed; the flag does not disable the sandbox when it is present)')
    parser.add_argument('--probes', action='store_true',
                        help='alfredo only: also measure M8 chat latency, M9 keystroke echo, M10 render cost (a release '
                             '`cargo test`, minutes on a cold build) and M14 kill -9 recovery')
    parser.add_argument('--no-render-probe', action='store_true',
                        help='with --probes: skip the slow M10 cargo build (M10 stays null)')
    parser.add_argument('--code-health', action='store_true',
                        help='also compute M15: clippy pedantic count, non-test unwrap/expect, largest non-test module '
                             'and test counts (runs cargo clippy and cargo test --list; minutes on a cold build)')
    parser.add_argument('--probe-chats', type=int, default=probes.DEFAULT_CHATS, help='M8 chats (default 20)')
    parser.add_argument('--probe-keys', type=int, default=probes.DEFAULT_KEYS, help='M9 keystrokes (default 100)')
    parser.add_argument('--probe-recovery-trials', type=int, default=probes.DEFAULT_RECOVERY_TRIALS,
                        help='M14 kill/resume trials (default 3)')
    options = parser.parse_args(argv)
    if options.trials < 1:
        parser.error('--trials must be at least 1')
    options.render_probe = not options.no_render_probe
    if options.no_render_probe and not options.probes:
        parser.error('--no-render-probe needs --probes')
    if options.probes and options.agent != 'alfredo':
        parser.error('--probes applies to --agent alfredo only')
    if min(options.probe_chats, options.probe_keys, options.probe_recovery_trials) < 1:
        parser.error('--probe-chats, --probe-keys and --probe-recovery-trials must be at least 1')
    if options.agent == 'claude-code':
        if options.backend:
            parser.error('--backend applies to --agent alfredo only; claude-code always uses the Anthropic API')
        options.backend, options.model = 'claude-api', options.model or 'haiku'
    else:
        options.backend = options.backend or 'fixture'
        if options.backend == 'ollama' and not options.model:
            parser.error('--backend ollama needs --model NAME')
        if options.backend == 'fixture':
            if options.model and options.model != 'fixture':
                parser.error('--model does not apply to the fixture backend')
            options.model = 'fixture'
    options.timeout = options.timeout or {'fixture': 120, 'ollama': 900, 'claude-api': 600}[options.backend]
    return options


def agent_version(options):
    command = [str(options.binary), '--version'] if options.agent == 'alfredo' else ['claude', '--version']
    try:
        return subprocess.run(command, capture_output=True, text=True, timeout=10).stdout.strip() or None
    except (OSError, subprocess.TimeoutExpired):
        return None


def main(argv=None):
    options = parse_args(argv if argv is not None else sys.argv[1:])
    try:
        tasks = load_tasks(HERE, options.tasks.split(',') if options.tasks else None)
        unsafe = options.unsafe_unsandboxed_grading
        refusal = grading_refusal(unsafe)
        if refusal:
            raise EvalError(refusal)
        if sandbox_tools() is None:
            print('warning: grading model-written code WITHOUT a sandbox (--unsafe-unsandboxed-grading)',
                  file=sys.stderr)
        if options.validate_tasks:
            problems = validate_tasks(tasks, unsafe_unsandboxed=unsafe)
            print('\n'.join(problems) or f'{len(tasks)} tasks: every check.sh fails on its untouched repo')
            return 1 if problems else 0
        skipped = {}
        if options.backend != 'fixture':
            skipped = {t['id']: 'scripted negative control; only meaningful against the fixture backend'
                       for t in tasks if t['scripted_only']}
            tasks = [t for t in tasks if not t['scripted_only']]
        if not tasks:
            raise EvalError('No tasks selected')
        if options.agent == 'alfredo':
            options.binary = Path(options.binary or os.environ.get('ALFREDO_TUI_BINARY') or TUI / 'target/debug/alfredo-tui').resolve()
            if not options.binary.is_file():
                raise EvalError(f'Build the binary first (cargo build --manifest-path alfredo-tui/Cargo.toml) '
                                f'or pass --binary: {options.binary}')
            if not Path('/usr/bin/bwrap').exists() and not shutil.which('bwrap'):
                raise EvalError('Alfredo workers need bubblewrap: sudo apt install bubblewrap')
            if options.backend == 'ollama':
                preflight_ollama(options.ollama_endpoint, options.model)
        elif not shutil.which('claude'):
            raise EvalError('`claude` is not on PATH; install Claude Code and log in first')
        run_trial = alfredo_trial if options.agent == 'alfredo' else claude_trial
        config = {
            'trials': options.trials, 'tasks': [t['id'] for t in tasks], 'skipped_tasks': skipped,
            'started_at': datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
            'cpus': os.cpu_count() or 1, 'agent_version': agent_version(options),
            'command': sys.argv[:1] + (argv if argv is not None else sys.argv[1:]),
            'backend': options.backend,
            'binary': str(options.binary) if options.agent == 'alfredo' else None,
            'profile': binary_profile(options.binary) if options.agent == 'alfredo' else None,
        }
        trials = []
        for task in tasks:
            for index in range(1, options.trials + 1):
                runner = chat_trial if options.agent == 'alfredo' and task.get('mode') == 'chat' else run_trial
                trial = runner(task, index, options)
                trials.append(trial)
                print(f'{task["id"]} trial {index}/{options.trials}: {"pass" if trial["pass"] else "FAIL"} '
                      f'({trial["outcome"]}, {trial["wall_s"]:.1f}s)', file=sys.stderr)
        probe_results = None
        if options.probes:
            config['probes'] = {'chats': options.probe_chats, 'keys': options.probe_keys,
                                'recovery_trials': options.probe_recovery_trials, 'render': options.render_probe,
                                'binary': config['binary'], 'profile': config['profile']}
            probe_results = run_probes(options, tasks)
        health = None
        if options.code_health:
            numbers, reasons = code_health.collect(TUI)
            health = {**numbers, 'reasons': reasons}
        summary, metrics, warnings, likely_broken = aggregate(trials, tasks, config, probe_results, health)
        config.pop('backend')
        document = {
            'schema_version': SCHEMA_VERSION, 'agent': options.agent, 'backend': options.backend,
            'model': options.model, 'config': config, 'trials': trials, 'tasks': summary,
            'metrics': metrics, 'warnings': warnings, 'likely_broken': likely_broken,
        }
        problems = schema_check.validate(document)
        if problems:
            raise EvalError('Output does not match schema.json (a runner bug): ' + '; '.join(problems[:5]))
        text = json.dumps(document, indent=2) + '\n'
        if options.out:
            Path(options.out).write_text(text, encoding='utf-8')
        else:
            sys.stdout.write(text)
        for warning in warnings:
            print(f'warning [{warning["code"]}]: {warning["message"]}', file=sys.stderr)
        return 1 if likely_broken or any(w['code'] in ('fixture_mismatch', 'harness_error') for w in warnings) else 0
    except EvalError as error:
        print(f'run_eval: {error}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
