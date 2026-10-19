"""Unit tests for the harness eval (alfredo-tui/eval): tasks, grader, schema, metrics, proxy."""
import contextlib
import copy
import http.client
import http.server
import io
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[2]
EVAL = ROOT / 'alfredo-tui/eval'


def load(name):
    spec = importlib.util.spec_from_file_location(name, EVAL / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(EVAL))
    sys.path.insert(0, str(ROOT / 'alfredo-tui/tests'))
    spec.loader.exec_module(module)
    return module


run_eval = load('run_eval')
schema_check = load('schema_check')


def good_trial(task='T1-add-function', kind='positive', passed=True):
    trial = run_eval.blank_trial({'id': task, 'kind': kind}, 1)
    trial.update({'pass': passed, 'outcome': 'done', 'turns': 2, 'tool_calls': 2, 'repairs': 0, 'wall_s': 1.5,
                  'repairs_accepted': 0, 'tasks_accepted': 1, 'enter_to_request_ms': 5.0,
                  'tokens': {'prompt': 100, 'eval': 20, 'planner': 50, 'peak_ctx': 120},
                  'errors': {'malformed': 0, 'refused': 0, 'sandbox': 0}, 'rss_kb': 24000, 'exit_code': 0,
                  'cold_start_ms': 40.0, 'overhead_s': 1.4,
                  'unavailable': {'ttft_ms': 'fixture'}})
    return trial


class TaskSetTests(unittest.TestCase):
    def setUp(self):
        self.tasks = run_eval.load_tasks(EVAL)

    def test_ships_the_required_tasks(self):
        ids = [t['id'] for t in self.tasks]
        self.assertGreaterEqual(len(ids), 6)
        for prefix in ('T1', 'T2', 'T3', 'T4'):
            self.assertTrue(any(i.startswith(prefix + '-') for i in ids), prefix)
        self.assertTrue(any(t['kind'] == 'negative_control' for t in self.tasks))
        self.assertTrue(any('test-and-fix' in i for i in ids))
        self.assertTrue(any('rename' in i for i in ids))

    def test_each_task_has_the_documented_files(self):
        for task in self.tasks:
            for name in ('goal.md', 'check.sh', 'answers.json', 'task.json'):
                self.assertTrue((task['dir'] / name).is_file(), f'{task["id"]}/{name}')
            self.assertTrue(any((task['dir'] / 'repo').iterdir()), task['id'])
            self.assertIn('plan', task['answers'])
            self.assertIn('workers', task['answers'])

    def test_every_check_fails_on_the_untouched_repo(self):
        self.assertEqual([], run_eval.validate_tasks(self.tasks))

    def test_scripted_answers_pass_every_positive_check_and_never_the_control(self):
        for task in self.tasks:
            with self.subTest(task=task['id']), tempfile.TemporaryDirectory() as tmp:
                repo = Path(tmp) / 'repo'
                run_eval.make_repo(task, repo)
                wrote = False
                for entry in reversed(task['answers']['workers']):  # repair entries are listed first
                    reply = entry['replies'][-1]
                    for file in reply.get('files', []):
                        (repo / file['path']).write_text(file['content'])
                        wrote = True
                run_eval.git(repo, 'add', '-A')
                run_eval.git(repo, '-c', 'commit.gpgSign=false', 'commit', '-qm', 'answer', '--allow-empty')
                ok, output = run_eval.grade(task, repo, 'HEAD')
                if task['kind'] == 'positive':
                    self.assertTrue(ok, output)
                else:
                    self.assertFalse(wrote or ok)

    def test_repair_task_first_answer_is_wrong(self):
        task = next(t for t in self.tasks if t['id'].startswith('T2-'))
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp) / 'repo'
            run_eval.make_repo(task, repo)
            first = next(e for e in task['answers']['workers'] if not e['prefix'].endswith('Repair #'))
            for file in first['replies'][0]['files']:
                (repo / file['path']).write_text(file['content'])
            run_eval.git(repo, 'add', '-A')
            run_eval.git(repo, '-c', 'commit.gpgSign=false', 'commit', '-qm', 'first')
            self.assertFalse(run_eval.grade(task, repo, 'HEAD')[0])

    def test_selector_accepts_prefixes_and_rejects_unknown_ids(self):
        self.assertEqual(['T1-add-function'], [t['id'] for t in run_eval.load_tasks(EVAL, ['T1'])])
        with self.assertRaises(run_eval.EvalError):
            run_eval.load_tasks(EVAL, ['T99'])

    def test_grading_without_a_candidate_fails(self):
        self.assertFalse(run_eval.grade(self.tasks[0], Path('/nonexistent'), None)[0])


class SchemaTests(unittest.TestCase):
    def document(self):
        tasks = [{'id': 'T1-add-function', 'kind': 'positive'}]
        trials = [good_trial() for _ in range(3)]
        for index, trial in enumerate(trials, 1):
            trial['trial'] = index
        config = {'trials': 3, 'backend': 'fixture'}
        summary, metrics, warnings, broken = run_eval.aggregate(trials, tasks, config)
        return {'schema_version': 1, 'agent': 'alfredo', 'backend': 'fixture', 'model': 'fixture',
                'config': {'trials': 3, 'tasks': ['T1-add-function'], 'started_at': 'now', 'cpus': 2,
                           'agent_version': None, 'binary': '/x/target/release/alfredo-tui', 'profile': 'release'}, 'trials': trials, 'tasks': summary, 'metrics': metrics,
                'warnings': warnings, 'likely_broken': broken}

    def test_valid_document_passes(self):
        self.assertEqual([], schema_check.validate(self.document()))

    def test_violations_are_reported(self):
        for mutate, fragment in [
            (lambda d: d.pop('metrics'), "missing required 'metrics'"),
            (lambda d: d.update(schema_version=2), 'expected 1'),
            (lambda d: d['trials'][0].update({'pass': 'yes'}), 'expected boolean'),
            (lambda d: d['trials'][0]['tokens'].update(extra=1), 'unexpected property'),
            (lambda d: d['metrics'].pop('M16'), "missing required 'M16'"),
            (lambda d: d['trials'][0].update(wall_s=-1), 'minimum'),
            (lambda d: d.update(backend='cloud'), 'not in'),
            (lambda d: d['config'].update(profile='fast'), 'not in'),
            (lambda d: d['metrics']['M4']['secondary'].update(per_task='x'), 'expected number|null|object'),
        ]:
            document = copy.deepcopy(self.document())
            mutate(document)
            self.assertTrue(any(fragment in p for p in schema_check.validate(document)), fragment)

    def test_schema_file_is_the_one_the_runner_uses(self):
        self.assertEqual(['schema_version'], json.loads(schema_check.SCHEMA_PATH.read_text())['required'][:1])


class AggregateTests(unittest.TestCase):
    def run_aggregate(self, trials, tasks, backend='fixture', count=3, **config):
        return run_eval.aggregate(trials, tasks, {'trials': count, 'backend': backend, **config})

    def test_zero_pass_rate_on_positive_tasks_flags_a_broken_grader(self):
        tasks = [{'id': 'A', 'kind': 'positive'}]
        _summary, metrics, warnings, broken = self.run_aggregate(
            [good_trial('A', passed=False) for _ in range(3)], tasks)
        self.assertTrue(broken)
        self.assertIn('zero_pass_rate', [w['code'] for w in warnings])
        self.assertEqual(0.0, metrics['M1']['value'])

    def test_control_that_passes_is_flagged(self):
        tasks = [{'id': 'A', 'kind': 'positive'}, {'id': 'C', 'kind': 'negative_control'}]
        trials = [good_trial('A'), good_trial('C', 'negative_control', passed=True)]
        _s, metrics, warnings, broken = self.run_aggregate(trials, tasks, count=1)
        self.assertTrue(broken)
        self.assertIn('control_passed', [w['code'] for w in warnings])
        self.assertEqual(1.0, metrics['M1']['value'], 'controls never count towards M1')

    def test_pass_hat_k_and_pass_at_1(self):
        tasks = [{'id': 'A', 'kind': 'positive'}, {'id': 'B', 'kind': 'positive'}]
        trials = [good_trial('A') for _ in range(3)] + [good_trial('B', passed=i < 2) for i in range(3)]
        _s, metrics, warnings, broken = self.run_aggregate(trials, tasks)
        self.assertAlmostEqual(5 / 6, metrics['M1']['value'], places=3)
        self.assertEqual(0.5, metrics['M2']['value'])  # A: 1, B: C(2,3)=0 -> mean 0.5
        self.assertFalse(broken)
        self.assertEqual([], [w for w in warnings if w['code'] != 'fixture_mismatch'])

    def test_unproducible_metrics_are_null_with_a_reason(self):
        tasks = [{'id': 'A', 'kind': 'positive'}]
        _s, metrics, _w, _b = self.run_aggregate([good_trial('A')], tasks, count=1)
        self.assertIsNone(metrics['M2']['value'])
        self.assertIn('3 trials', metrics['M2']['reason'])
        self.assertEqual(120, metrics['M6']['value'])
        self.assertEqual(120, metrics['M6']['secondary']['peak_ctx_max'])
        self.assertIsNone(metrics['M3']['value'])
        self.assertEqual('no repair attempted', metrics['M3']['reason'])
        self.assertIn('Enter/render timestamps', metrics['M8']['reason'])
        self.assertEqual(5.0, metrics['M8']['secondary']['enter_to_request_p50_ms'])
        self.assertEqual('keystroke echo', metrics['M9']['name'])
        self.assertEqual('frame render cost', metrics['M10']['name'])
        for key in ('M8', 'M9', 'M10', 'M13', 'M14', 'M15'):
            self.assertIsNone(metrics[key]['value'])
            self.assertTrue(metrics[key]['reason'])
        for key in ('M1', 'M4', 'M5', 'M6', 'M7', 'M11', 'M12', 'M16'):
            self.assertIsNotNone(metrics[key]['value'], key)
            self.assertIsNone(metrics[key]['reason'])

    def test_m3_m7_and_m12_follow_the_scoreboard_definitions(self):
        tasks = [{'id': 'A', 'kind': 'positive'}]
        fixed, unfixed, crashed = good_trial('A'), good_trial('A'), good_trial('A')
        fixed.update(repairs=2, repairs_accepted=1, tasks_accepted=1, tool_calls=6,
                     errors={'malformed': 1, 'refused': 0, 'sandbox': 0})
        unfixed.update(repairs=2, repairs_accepted=1, tasks_accepted=1, tool_calls=4,
                       errors={'malformed': 1, 'refused': 1, 'sandbox': 0})
        crashed.update(repairs=0, panic=True, exit_code=101)
        _s, metrics, _w, _b = self.run_aggregate([fixed, unfixed, crashed], tasks)
        self.assertEqual(0.5, metrics['M3']['value'])  # 2 of 4 repairs accepted
        self.assertAlmostEqual(4 / 3, metrics['M3']['secondary']['repairs_per_accepted_task'], places=3)
        self.assertAlmostEqual(100 * 3 / 12, metrics['M7']['value'], places=3)
        self.assertEqual(('count', 1), (metrics['M12']['unit'], metrics['M12']['value']))
        self.assertIn('eval trials only', metrics['M12']['note'])

    def test_fixture_expectations_flag_a_mismatch(self):
        tasks = [{'id': 'A', 'kind': 'positive'}, {'id': 'C', 'kind': 'negative_control'}]
        trials = [good_trial('A', passed=False), good_trial('C', 'negative_control', passed=False),
                  good_trial('A')]
        summary, _m, warnings, _b = self.run_aggregate(trials, tasks, count=2)
        self.assertFalse(summary['A']['as_expected'])
        self.assertTrue(summary['C']['as_expected'])
        self.assertIn('fixture_mismatch', [w['code'] for w in warnings])


    def test_per_task_medians_expose_a_regression_the_global_median_hides(self):
        tasks = [{'id': 'A', 'kind': 'positive'}, {'id': 'B', 'kind': 'positive'}, {'id': 'C', 'kind': 'positive'}]
        trials = []
        for task, turns, calls in [('A', 2, 2), ('B', 2, 2), ('C', 3, 4)]:
            for _ in range(3):
                trial = good_trial(task)
                trial.update(turns=turns, tool_calls=calls)
                trials.append(trial)
        _s, metrics, _w, _b = self.run_aggregate(trials, tasks)
        self.assertEqual(2, metrics['M4']['value'])  # the global median is blind to C
        self.assertEqual({'A': 2, 'B': 2, 'C': 3}, metrics['M4']['secondary']['per_task'])
        self.assertEqual({'A': 2, 'B': 2, 'C': 4}, metrics['M5']['secondary']['per_task'])
        self.assertEqual({'A': 120, 'B': 120, 'C': 120}, metrics['M6']['secondary']['per_task'])
        self.assertEqual(120, metrics['M6']['secondary']['peak_ctx_max'])

    def test_m3_reports_repairs_per_accepted_task_for_each_task(self):
        tasks = [{'id': 'A', 'kind': 'positive'}, {'id': 'B', 'kind': 'positive'}]
        trials = [good_trial('A') for _ in range(3)] + [good_trial('B') for _ in range(3)]
        for trial in trials[3:]:
            trial.update(repairs=1, repairs_accepted=1, tasks_accepted=1)
        _s, metrics, _w, _b = self.run_aggregate(trials, tasks)
        secondary = metrics['M3']['secondary']
        self.assertAlmostEqual(3 / 6, secondary['repairs_per_accepted_task'], places=3)  # the global figure
        self.assertEqual({'A': 0.0, 'B': 1.0}, secondary['per_task'])

    def test_per_task_is_null_when_a_task_produced_no_value(self):
        tasks = [{'id': 'A', 'kind': 'positive'}, {'id': 'B', 'kind': 'positive'}]
        trials = [good_trial('A'), good_trial('B')]
        trials[1].update(turns=None, unavailable={'turns': 'not counted'})
        _s, metrics, _w, _b = self.run_aggregate(trials, tasks, count=1)
        self.assertEqual({'A': 2, 'B': None}, metrics['M4']['secondary']['per_task'])

    def test_profile_is_derived_from_the_binary_path(self):
        for path, profile in [('/x/target/debug/alfredo-tui', 'debug'), ('/x/target/release/alfredo-tui', 'release'),
                              ('/opt/bin/alfredo-tui', 'unknown')]:
            self.assertEqual(profile, run_eval.binary_profile(Path(path)), path)

    def test_debug_profile_warns_on_m11_and_m16_and_release_does_not(self):
        tasks = [{'id': 'A', 'kind': 'positive'}]
        trials = [good_trial('A')]
        _s, metrics, warnings, broken = self.run_aggregate(
            trials, tasks, count=1, binary='/x/target/debug/alfredo-tui', profile='debug')
        self.assertFalse(broken)
        warning = next(w for w in warnings if w['code'] == 'debug_profile')
        self.assertIn('M11', warning['message'])
        self.assertIn('M16', warning['message'])
        for key in ('M11', 'M16'):
            self.assertIn('debug', metrics[key]['note'])
        for profile in ('release', None):
            _s, metrics, warnings, _b = self.run_aggregate(
                trials, tasks, count=1, binary='/x/target/release/alfredo-tui', profile=profile)
            self.assertNotIn('debug_profile', [w['code'] for w in warnings])
            self.assertNotIn('debug', metrics['M11'].get('note', ''))

    def test_m16_reports_the_size_of_the_binary_under_test(self):
        tasks = [{'id': 'A', 'kind': 'positive'}]
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / 'alfredo-tui'
            binary.write_bytes(b'x' * 4096)
            for profile, release_kb in (('release', 4), ('debug', None)):
                _s, metrics, _w, _b = self.run_aggregate(
                    [good_trial('A')], tasks, count=1, binary=str(binary), profile=profile)
                self.assertEqual(4, metrics['M16']['secondary']['binary_kb'])
                self.assertEqual(release_kb, metrics['M16']['secondary']['release_binary_kb'])

    def test_missing_binary_gives_null_sizes(self):
        _s, metrics, _w, _b = self.run_aggregate([good_trial('A')], [{'id': 'A', 'kind': 'positive'}], count=1)
        self.assertIsNone(metrics['M16']['secondary']['binary_kb'])
        self.assertIsNone(metrics['M16']['secondary']['release_binary_kb'])


class RecorderTests(unittest.TestCase):
    def test_proxy_records_kind_tokens_and_ttft_from_a_streaming_upstream(self):
        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"models":[{"name":"m"}]}')

            def do_POST(self):
                self.rfile.read(int(self.headers['Content-Length']))
                self.send_response(200)
                self.send_header('Content-Type', 'application/x-ndjson')
                self.end_headers()
                for line in ({'message': {'content': 'hi'}, 'done': False},
                             {'message': {'content': ''}, 'done': True, 'prompt_eval_count': 120, 'eval_count': 30}):
                    self.wfile.write((json.dumps(line) + '\n').encode())
                    self.wfile.flush()

        upstream = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
        threading.Thread(target=upstream.serve_forever, daemon=True).start()
        self.addCleanup(upstream.server_close)
        self.addCleanup(upstream.shutdown)
        proxy = run_eval.ModelServer(upstream=('127.0.0.1', upstream.server_port))
        self.addCleanup(proxy.close)
        for prompt in ('Plan this', 'Implement this task: do it'):
            conn = http.client.HTTPConnection('127.0.0.1', proxy.endpoint.rsplit(':', 1)[1])
            conn.request('POST', '/api/chat', json.dumps({'model': 'm', 'messages': [
                {'role': 'system', 'content': 's'}, {'role': 'user', 'content': prompt}]}),
                {'Content-Type': 'application/json'})
            self.assertIn(b'"done": true', conn.getresponse().read())
        records = proxy.snapshot()
        self.assertEqual(['planner', 'worker'], [r['kind'] for r in records])
        stats, unavailable = run_eval.model_stats(records, 'ollama')
        self.assertEqual({'prompt': 240, 'eval': 60, 'planner': 150, 'peak_ctx': 150}, stats['tokens'])
        self.assertEqual(2, stats['turns'])
        self.assertIsNotNone(stats['ttft_ms'])
        self.assertEqual({}, unavailable)

    def test_missing_token_counts_make_tokens_null_not_zero(self):
        records = [{'kind': 'worker', 'start': 0, 'end': 1, 'ttft': None, 'prompt': None, 'eval': None}]
        stats, unavailable = run_eval.model_stats(records, 'ollama')
        self.assertIsNone(stats['tokens']['prompt'])
        self.assertIn('tokens', unavailable)


class ClaudeStreamTests(unittest.TestCase):
    def test_counts_tool_calls_errors_turns_and_tokens(self):
        lines = [json.dumps(e) for e in [
            {'type': 'system', 'subtype': 'init', 'claude_code_version': '9.9'},
            {'type': 'assistant', 'message': {'content': [{'type': 'text', 'text': 'x'},
                                                         {'type': 'tool_use', 'name': 'Edit'}]}},
            {'type': 'user', 'message': {'content': [{'type': 'tool_result', 'is_error': True,
                                                      'content': 'String to replace not found'}]}},
            {'type': 'user', 'message': {'content': [{'type': 'tool_result', 'is_error': True,
                                                      'content': "Claude requested permissions to write"}]}},
            {'type': 'assistant', 'message': {'content': [{'type': 'tool_use', 'name': 'Write'}]}},
            {'type': 'result', 'num_turns': 3, 'usage': {'input_tokens': 10, 'cache_read_input_tokens': 90,
                                                         'output_tokens': 7}},
        ]] + ['not json']
        facts = run_eval.parse_claude_stream(lines)
        self.assertEqual((3, 2, 1, 1, '9.9'), (facts['turns'], facts['tool_calls'], facts['malformed'],
                                               facts['refused'], facts['version']))
        self.assertEqual({'prompt': 100, 'eval': 7}, facts['tokens'])


class CliTests(unittest.TestCase):
    def test_claude_code_rejects_a_backend_and_ollama_needs_a_model(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                run_eval.parse_args(['--agent', 'claude-code', '--backend', 'fixture'])
            with self.assertRaises(SystemExit):
                run_eval.parse_args(['--agent', 'alfredo', '--backend', 'ollama'])
        options = run_eval.parse_args(['--agent', 'claude-code'])
        self.assertEqual(('claude-api', 'haiku', 3), (options.backend, options.model, options.trials))

    def test_ollama_mode_fails_fast_when_ollama_is_unreachable_or_remote(self):
        with self.assertRaises(run_eval.EvalError) as error:
            run_eval.preflight_ollama('http://127.0.0.1:1', 'm')
        self.assertIn('not reachable', str(error.exception))
        with self.assertRaises(run_eval.EvalError) as error:
            run_eval.preflight_ollama('http://example.com:11434', 'm')
        self.assertIn('local-only', str(error.exception))

    def test_remove_tree_deletes_read_only_trees(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'x'
            (root / 'd').mkdir(parents=True)
            (root / 'd/f').write_text('x')
            (root / 'd').chmod(0o500)
            run_eval.remove_tree(root)
            self.assertFalse(root.exists())
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == '__main__':
    unittest.main()
