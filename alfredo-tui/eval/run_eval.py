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

METRIC_NAMES = {
    'M1': ('pass@1', 'fraction'), 'M2': ('pass^3', 'fraction'),
    'M3': ('repair recovery rate', 'fraction of repairs accepted'),
    'M4': ('turns per task', 'model requests per trial (median)'),
    'M5': ('tool/command calls per task', 'worker runs + check runs per trial (median)'),
    'M6': ('tokens per task', 'prompt+completion tokens per trial (median)'),
    'M7': ('tool-call error rate', 'errors per 100 calls'),
    'M8': ('harness overhead per turn', 'ms p50'),
    'M9': ('keystroke echo', 'ms p50/p95'), 'M10': ('frame render cost', 'ms p50/p95'),
    'M11': ('cold start to first frame', 'ms (median)'),
    'M12': ('panics/crashes', 'count'), 'M13': ('flaky tests', 'count'),
    'M14': ('crash recovery', 'fraction'), 'M15': ('code health', 'score'),
    'M16': ('peak RSS during /go', 'kB, main process, median'),
}
FIXED_NULL = {
    'M9': 'keystroke-echo benches (tests/rendering_cost.rs) are not wrapped by the eval yet (follow-up to #176)',
    'M10': 'frame-render-cost benches (tests/rendering_cost.rs) are not wrapped by the eval yet (follow-up to #176)',
    'M13': 'flakiness needs repeated identical runs of one build; --trials only samples task outcomes (see M2)',
    'M14': 'kill/restart recovery needs a dedicated kill probe, which eval trials do not run',
    'M15': 'code health is a static analysis of the source tree, not of an agent run',
}


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
            'scripted_only': bool(meta.get('scripted_only')),
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
        run = subprocess.run(argv, cwd=export, env=env, timeout=60, capture_output=True, text=True)
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
            record['kind'] = 'worker' if last.startswith('Implement this task:') else 'planner'
            if self.upstream:
                self.forward(handler, request, record)
            else:
                content = self.fixture_reply(record['kind'], request, last)
                handler.send_response(200)
                handler.send_header('Content-Type', 'application/x-ndjson')
                handler.end_headers()
                # Deterministic synthetic counts (ceil(chars / 4)); a real server reports its own.
                chars = sum(len(m.get('content') or '') for m in request['messages'])
                done = {'message': {'content': content}, 'done': True,
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
            'tool_calls': runs + checks, 'errors': errors}


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
        'task': task['id'], 'kind': task['kind'], 'trial': index, 'pass': False, 'outcome': 'harness_error',
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
            time.sleep(0.01)
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


def aggregate(trials, tasks, config):
    """Per-task summary, M1..M16 and warnings from the trial list."""
    positives = [t for t in trials if t['kind'] == 'positive']
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
            result['secondary'] = {k: round(v, digits) if isinstance(v, float) else v for k, v in secondary.items()}
        return result

    def over(field, reducer, getter=None):
        get = getter or (lambda t: t[field])
        values = [get(t) for t in positives]
        have = [v for v in values if v is not None]
        if have:
            return reducer(have), None
        reasons = [t['unavailable'].get(field) for t in positives if t['unavailable'].get(field)]
        return None, (reasons[0] if reasons else f'no positive trial produced {field}')

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
    metrics['M3'] = metric('M3', value, reason, secondary={
        'repairs_per_accepted_task': attempted / accepted_tasks if accepted_tasks else None,
        'repairs_attempted': attempted if known else None})
    value, reason = over('turns', statistics.median)
    metrics['M4'] = metric('M4', value, reason)
    value, reason = over('tool_calls', statistics.median)
    metrics['M5'] = metric('M5', value, reason)
    value, reason = over('tokens', statistics.median, lambda t: None if t['tokens']['prompt'] is None
                         else t['tokens']['prompt'] + t['tokens']['eval'])
    peaks = [t['tokens']['peak_ctx'] for t in positives if t['tokens']['peak_ctx'] is not None]
    metrics['M6'] = metric('M6', value, reason, secondary={'peak_ctx_max': max(peaks) if peaks else None},
                           note='fixture token counts are synthetic (ceil(chars / 4)), deterministic, not model counts'
                           if config['backend'] == 'fixture' else None)
    scored = [t for t in positives if t['errors']['malformed'] is not None and t['tool_calls'] is not None]
    calls = sum(t['tool_calls'] for t in scored)
    errors = sum(sum(t['errors'].values()) for t in scored)
    metrics['M7'] = metric('M7', 100 * errors / calls if calls else None, 'no tool calls were made')
    enter = [t['enter_to_request_ms'] for t in positives if t['enter_to_request_ms'] is not None]
    metrics['M8'] = metric(
        'M8', None, 'needs Enter/render timestamps: reply-to-screen time is not measured because spinner redraws '
        'make byte-level screen-change detection unreliable at the 30 ms poll; only Enter-to-request is measured '
        '(secondary, exact: both stamps are taken in this process); trial.overhead_s is the whole non-model time',
        secondary={'enter_to_request_p50_ms': statistics.median(enter) if enter else None})
    value, reason = over('cold_start_ms', statistics.median)
    metrics['M11'] = metric('M11', value, reason, digits=1)
    abnormal = sum(1 for t in trials if t['panic'] or t['outcome'] == 'crashed'
                   or t['exit_code'] not in (0, None))
    metrics['M12'] = metric('M12', abnormal, note='counts eval trials only (panic, crash or abnormal exit); '
                            'the smoke suites are not included')
    value, reason = over('rss_kb', statistics.median)
    release = TUI / 'target/release/alfredo-tui'
    metrics['M16'] = metric(
        'M16', value, reason, digits=0,
        secondary={'release_binary_kb': release.stat().st_size // 1024 if release.is_file() else None},
        note='main process VmHWM only (process-tree peak is not sampled); release_binary_kb is null unless '
             'target/release/alfredo-tui exists')
    for key, why in FIXED_NULL.items():
        metrics[key] = metric(key, None, why)
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
    parser.add_argument('--binary', help='alfredo-tui binary (default $ALFREDO_TUI_BINARY or target/debug)')
    parser.add_argument('--tmp-root', help='directory for temp repos and state (default: system temp)')
    parser.add_argument('--keep', action='store_true', help='keep temp repos for debugging')
    parser.add_argument('--validate-tasks', action='store_true',
                        help='only check that every check.sh fails on its untouched repo, then exit')
    parser.add_argument('--unsafe-unsandboxed-grading', action='store_true',
                        help='grade model-written code on the host when bubblewrap is missing (never needed when '
                             'bwrap is installed; the flag does not disable the sandbox when it is present)')
    options = parser.parse_args(argv)
    if options.trials < 1:
        parser.error('--trials must be at least 1')
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
        }
        trials = []
        for task in tasks:
            for index in range(1, options.trials + 1):
                trial = run_trial(task, index, options)
                trials.append(trial)
                print(f'{task["id"]} trial {index}/{options.trials}: {"pass" if trial["pass"] else "FAIL"} '
                      f'({trial["outcome"]}, {trial["wall_s"]:.1f}s)', file=sys.stderr)
        summary, metrics, warnings, likely_broken = aggregate(trials, tasks, config)
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
