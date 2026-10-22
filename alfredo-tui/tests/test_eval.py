"""Unit tests for the harness eval (alfredo-tui/eval): tasks, grader, schema, metrics, proxy."""
import contextlib
import copy
import http.client
import http.server
import io
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

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
probes = load('probes')
schema_check = load('schema_check')
code_health = load('code_health')
# These tests grade author-written scripted answers, never model output, so without bwrap they may grade
# unsandboxed. The sandboxed path is covered by test_eval_grader.py, whose live tests CI runs with bwrap.
NO_SANDBOX_AVAILABLE = run_eval.sandbox_tools() is None


def good_trial(task='T1-add-function', kind='positive', passed=True):
    trial = run_eval.blank_trial({'id': task, 'kind': kind}, 1)
    trial.update({'pass': passed, 'outcome': 'done', 'turns': 2, 'tool_calls': 2, 'repairs': 0, 'wall_s': 1.5,
                  'repairs_accepted': 0, 'tasks_accepted': 1, 'enter_to_request_ms': 5.0,
                  'tokens': {'prompt': 100, 'eval': 20, 'planner': 50, 'peak_ctx': 120},
                  'errors': {'malformed': 0, 'refused': 0, 'sandbox': 0}, 'rss_kb': 24000, 'exit_code': 0,
                  'cold_start_ms': 40.0, 'overhead_s': 1.4,
                  'unavailable': {'ttft_ms': 'fixture'}})
    return trial


def apply_chat_script(task, repo):
    """Apply the scripted edit_file/write_file calls of a chat task, as the approved tools would."""
    wrote = False
    for reply in task['answers']['chat']:
        for call in reply.get('tool_calls', []):
            args = call['arguments']
            if call['name'] == 'write_file':
                (repo / args['path']).write_text(args['content'])
            elif call['name'] == 'edit_file':
                text = (repo / args['path']).read_text()
                assert text.count(args['old_text']) == 1, args
                (repo / args['path']).write_text(text.replace(args['old_text'], args['new_text']))
            else:
                continue
            wrote = True
    return wrote


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
            if task['mode'] == 'chat':
                self.assertIn('chat', task['answers'])
            else:
                self.assertIn('plan', task['answers'])
                self.assertIn('workers', task['answers'])

    def test_ships_a_chat_tool_task_scripted_read_then_edit_then_reply(self):
        chat = [t for t in self.tasks if t['mode'] == 'chat']
        self.assertTrue(any(t['id'].startswith('T7-') for t in chat))
        for task in chat:
            names = [call['name'] for reply in task['answers']['chat'] for call in reply.get('tool_calls', [])]
            edits = [i for i, n in enumerate(names) if n in ('edit_file', 'write_file')]
            self.assertTrue(edits, task['id'])
            self.assertTrue(any(n in run_eval.CHAT_READ_TOOLS for n in names[:edits[0]]), names)
            self.assertIn('content', task['answers']['chat'][-1], 'the script ends with a plain reply')

    def test_every_check_fails_on_the_untouched_repo(self):
        self.assertEqual([], run_eval.validate_tasks(self.tasks, unsafe_unsandboxed=NO_SANDBOX_AVAILABLE))

    def test_scripted_answers_pass_every_positive_check_and_never_the_control(self):
        for task in self.tasks:
            with self.subTest(task=task['id']), tempfile.TemporaryDirectory() as tmp:
                repo = Path(tmp) / 'repo'
                run_eval.make_repo(task, repo)
                wrote = False
                if task['mode'] == 'chat':
                    wrote = apply_chat_script(task, repo)
                for entry in reversed(task['answers'].get('workers', [])):  # repair entries are listed first
                    reply = entry['replies'][-1]
                    for file in reply.get('files', []):
                        (repo / file['path']).write_text(file['content'])
                        wrote = True
                run_eval.git(repo, 'add', '-A')
                run_eval.git(repo, '-c', 'commit.gpgSign=false', 'commit', '-qm', 'answer', '--allow-empty')
                ok, output = run_eval.grade(task, repo, 'HEAD', unsafe_unsandboxed=NO_SANDBOX_AVAILABLE)
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
            self.assertFalse(run_eval.grade(task, repo, 'HEAD', unsafe_unsandboxed=NO_SANDBOX_AVAILABLE)[0])

    def test_selector_accepts_prefixes_and_rejects_unknown_ids(self):
        self.assertEqual(['T1-add-function'], [t['id'] for t in run_eval.load_tasks(EVAL, ['T1'])])
        with self.assertRaises(run_eval.EvalError):
            run_eval.load_tasks(EVAL, ['T99'])

    def test_grading_without_a_candidate_fails(self):
        self.assertFalse(run_eval.grade(self.tasks[0], Path('/nonexistent'), None)[0])


class SandboxToolTests(unittest.TestCase):
    def test_accepts_only_the_trusted_absolute_installs(self):
        with mock.patch.object(run_eval.Path, 'is_file', return_value=True), \
                mock.patch('shutil.which', side_effect=AssertionError('PATH must not be consulted')):
            self.assertEqual(('/usr/bin/bwrap', '/usr/bin/prlimit'), run_eval.sandbox_tools())

    def test_path_lookup_never_supplies_the_tools(self):
        with mock.patch.object(run_eval.Path, 'is_file', return_value=False), \
                mock.patch('shutil.which', side_effect=AssertionError('PATH must not be consulted')):
            self.assertIsNone(run_eval.sandbox_tools())

    def test_one_helper_decides_the_grading_refusal(self):
        with mock.patch.object(run_eval, 'sandbox_tools', return_value=None):
            self.assertEqual(run_eval.SANDBOX_MISSING, run_eval.grading_refusal())
            self.assertIsNone(run_eval.grading_refusal(unsafe_unsandboxed=True))
        with mock.patch.object(run_eval, 'sandbox_tools', return_value=('/usr/bin/bwrap', '/usr/bin/prlimit')):
            self.assertIsNone(run_eval.grading_refusal())


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
                           'agent_version': None, 'binary': '/x/target/release/alfredo-tui',
                           'profile': 'release'},
                'trials': trials, 'tasks': summary, 'metrics': metrics,
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
        self.assertIn('--probes', metrics['M8']['reason'])
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
                              ('/opt/bin/alfredo-tui', 'unknown'),
                              ('/work/release/proj/target/debug/alfredo-tui', 'debug'),
                              ('/opt/debug/bin/alfredo-tui', 'unknown'),
                              ('/tmp/tgt/release/alfredo-tui', 'release')]:
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


RENDER_OUTPUT = """running 3 tests
test measure_mission_work_redraws ... {"schema_version":1,"fixture":"x","snapshot_bytes":1,"cohorts":[{"panel":"activity","viewport":[140,40],"cold_p50_ns":2000000,"cold_p95_ns":3000000,"warm_p50_ns":500000,"warm_p95_ns":900000}],"scope":"s"}
ok
test measure_dashboard_streaming_redraws ... {"average_ms":1.25,"fixture":"200 tasks"}
ok
test measure_chat_transcript_redraws ... {"fixture":"400 messages","transcript_bytes":9,"warm_p50_ms":0.6,"warm_p95_ms":0.8}
ok
"""


def probe_results(**override):
    results = {
        'chat': {'enter_to_request_ms': [2.0, 4.0, 6.0], 'reply_to_screen_ms': [1.0, 1.5, 3.0]},
        'echo': {'echo_ms': [float(i) for i in range(1, 101)]},
        'render': probes.parse_rendering_cost(RENDER_OUTPUT),
        'recovery': {'recovered': [True, True, False]},
    }
    results.update(override)
    return results


class ProbeTests(unittest.TestCase):
    def aggregate(self, results, **config):
        return run_eval.aggregate([good_trial('A')], [{'id': 'A', 'kind': 'positive'}],
                                  {'trials': 1, 'backend': 'fixture', **config}, probes_result=results)

    def test_parses_the_three_rendering_cost_json_lines_into_milliseconds(self):
        parsed = probes.parse_rendering_cost(RENDER_OUTPUT)
        self.assertEqual({'cold_p50': 2.0, 'cold_p95': 3.0, 'warm_p50': 0.5, 'warm_p95': 0.9},
                         parsed['mission_work']['activity@140x40'])
        self.assertEqual(1.25, parsed['dashboard_average_ms'])
        self.assertEqual({'warm_p50_ms': 0.6, 'warm_p95_ms': 0.8}, parsed['chat'])

    def test_rendering_cost_output_without_json_is_an_error_not_a_crash(self):
        self.assertIn('no JSON', probes.parse_rendering_cost('running 0 tests\n')['error'])

    def test_percentiles_use_the_rounded_index_and_handle_no_samples(self):
        values = list(range(1, 101))
        self.assertEqual({'p50': 51, 'p95': 95, 'n': 100}, probes.stats(values))
        self.assertEqual({'p50': None, 'p95': None, 'n': 0}, probes.stats([]))

    def test_probes_fill_m8_m9_m10_and_m14(self):
        _s, metrics, _w, broken = self.aggregate(probe_results())
        self.assertFalse(broken)
        m8 = metrics['M8']
        self.assertEqual(1.5, m8['value'])  # reply to screen p50
        self.assertIsNone(m8['reason'])
        self.assertEqual(3.0, m8['secondary']['reply_to_screen_p95_ms'])
        self.assertEqual(4.0, m8['secondary']['chat_enter_to_request_p50_ms'])
        self.assertEqual(6.0, m8['secondary']['chat_enter_to_request_p95_ms'])
        self.assertEqual(3, m8['secondary']['chats'])
        self.assertEqual(5.0, m8['secondary']['enter_to_request_p50_ms'], 'the /go figure keeps its meaning')
        self.assertEqual(95, metrics['M9']['value'])  # echo p95
        self.assertEqual(51, metrics['M9']['secondary']['echo_p50_ms'])
        self.assertEqual(100, metrics['M9']['secondary']['keys'])
        self.assertEqual(0.8, metrics['M10']['value'])  # chat warm p95
        self.assertEqual(0.6, metrics['M10']['secondary']['chat_warm_p50_ms'])
        self.assertEqual(1.25, metrics['M10']['secondary']['dashboard_average_ms'])
        self.assertEqual({'activity@140x40': 0.9}, metrics['M10']['secondary']['mission_work_warm_p95_ms'])
        self.assertEqual({'activity@140x40': 3.0}, metrics['M10']['secondary']['mission_work_cold_p95_ms'])
        self.assertEqual(0.667, metrics['M14']['value'])
        self.assertEqual(3, metrics['M14']['secondary']['trials'])
        self.assertEqual(2, metrics['M14']['secondary']['recovered'])
        self.assertEqual([], schema_check.validate(self.document(metrics)))

    def document(self, metrics):
        return {'schema_version': 1, 'agent': 'alfredo', 'backend': 'fixture', 'model': 'fixture', 'trials': [],
                'config': {'trials': 1, 'tasks': [], 'started_at': 'x', 'cpus': 1, 'agent_version': None},
                'tasks': {}, 'metrics': metrics, 'warnings': [], 'likely_broken': False}

    def test_without_probes_the_rows_stay_null_and_point_at_the_flag(self):
        _s, metrics, _w, _b = self.aggregate(None)
        for key in ('M8', 'M9', 'M10', 'M14'):
            self.assertIsNone(metrics[key]['value'])
            self.assertIn('--probes', metrics[key]['reason'])

    def test_a_failed_probe_nulls_only_its_own_rows_with_the_reason(self):
        _s, metrics, _w, _b = self.aggregate(probe_results(chat={'error': 'marker never seen'},
                                                           render={'error': 'cargo failed'}))
        self.assertIsNone(metrics['M8']['value'])
        self.assertEqual('marker never seen', metrics['M8']['reason'])
        self.assertEqual('cargo failed', metrics['M10']['reason'])
        self.assertEqual(95, metrics['M9']['value'])
        self.assertEqual(0.667, metrics['M14']['value'])

    def test_skipped_render_probe_gives_a_reason(self):
        _s, metrics, _w, _b = self.aggregate({k: v for k, v in probe_results().items() if k != 'render'})
        self.assertIn('--no-render-probe', metrics['M10']['reason'])

    def test_debug_binary_is_noted_on_the_probe_latency_rows(self):
        _s, metrics, _w, _b = self.aggregate(probe_results(), binary='/x/target/debug/alfredo-tui', profile='debug')
        for key in ('M8', 'M9', 'M14'):
            self.assertIn('debug', metrics[key]['note'])
        self.assertNotIn('debug build', metrics['M10']['note'], 'the render bench is always a release build')

    def test_probes_flag_is_alfredo_only_and_counts_must_be_positive(self):
        for argv in (['--agent', 'claude-code', '--probes'], ['--agent', 'alfredo', '--probes', '--probe-chats', '0'],
                     ['--agent', 'alfredo', '--no-render-probe']):
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                run_eval.parse_args(argv)
        options = run_eval.parse_args(['--agent', 'alfredo', '--probes'])
        self.assertEqual((20, 100, 3, True), (options.probe_chats, options.probe_keys,
                                              options.probe_recovery_trials, options.render_probe))

    def test_a_raising_probe_nulls_only_its_own_rows_and_the_eval_still_emits_json(self):
        options = run_eval.parse_args(['--agent', 'alfredo', '--probes', '--probe-recovery-trials', '1'])
        options.binary = Path('/x/target/release/alfredo-tui')
        tasks = [{'id': 'A', 'kind': 'positive'}]
        with mock.patch.object(run_eval.probes, 'run_chat', side_effect=OSError('pty gone')), \
                mock.patch.object(run_eval.probes, 'run_echo', return_value={'echo_ms': [1.0]}), \
                mock.patch.object(run_eval, 'recovery_trial', side_effect=subprocess.TimeoutExpired('git', 1)), \
                mock.patch.object(run_eval.probes, 'run_render', side_effect=RuntimeError('cargo')):
            results = run_eval.run_probes(options, tasks)
        _s, metrics, _w, _b = self.aggregate(results)
        self.assertIn('pty gone', metrics['M8']['reason'])
        self.assertIn('TimeoutExpired', metrics['M14']['reason'])
        self.assertIn('cargo', metrics['M10']['reason'])
        self.assertEqual(1.0, metrics['M9']['value'])

    def test_debug_warning_names_the_probe_rows_only_when_probes_ran(self):
        tasks = [{'id': 'A', 'kind': 'positive'}]
        config = dict(binary='/x/target/debug/alfredo-tui', profile='debug')
        _s, _m, warnings, _b = self.aggregate(probe_results(), probes={'chats': 3}, **config)
        text = next(w for w in warnings if w['code'] == 'debug_profile')['message']
        for key in ('M8', 'M9', 'M14'):
            self.assertIn(key, text)
        _s, _m, warnings, _b = self.aggregate(None, **config)
        self.assertNotIn('M8', next(w for w in warnings if w['code'] == 'debug_profile')['message'])

    def test_marker_server_replies_with_the_prompt_keyed_marker_and_stamps_it(self):
        server = probes.MarkerServer()
        self.addCleanup(server.close)
        conn = http.client.HTTPConnection('127.0.0.1', server.server.server_port, timeout=5)
        conn.request('POST', '/api/chat', json.dumps({'messages': [{'role': 'user', 'content': 'chat 007'}]}),
                     {'Content-Type': 'application/json'})
        body = conn.getresponse().read().decode()
        self.assertIn(probes.marker('007'), body)
        self.assertLessEqual(server.arrive['007'], server.sent['007'])

    def test_server_stamps_are_waited_for_when_the_reply_is_seen_before_the_stamp(self):
        server = probes.MarkerServer()
        self.addCleanup(server.close)
        server.arrive['004'] = 10.0

        def stamp_late():  # the handler flushed, the pty reader already saw the marker, the stamp lands later
            time.sleep(0.15)
            server.sent['004'] = 10.2

        thread = threading.Thread(target=stamp_late)
        thread.start()
        self.addCleanup(thread.join)
        self.assertEqual((10.0, 10.2), probes.server_stamps(server, '004', wait=1.0))

    def test_server_stamps_name_which_stamp_is_missing(self):
        server = probes.MarkerServer()
        self.addCleanup(server.close)
        self.assertIn('never reached the server', probes.server_stamps(server, '001', wait=0.05)['error'])
        server.arrive['001'] = 1.0
        error = probes.server_stamps(server, '001', wait=0.05)['error']
        self.assertIn('reached the server', error)
        self.assertIn('no send stamp', error)
        self.assertNotIn('never reached', error)

    def test_reply_to_screen_is_never_negative(self):
        self.assertEqual(0.0, probes.reply_to_screen_ms(seen_at=5.0, sent_at=5.002))
        self.assertAlmostEqual(3.0, probes.reply_to_screen_ms(seen_at=5.003, sent_at=5.0))

    def test_model_server_can_hold_worker_requests_until_released(self):
        task = run_eval.load_tasks(EVAL, ['T1'])[0]
        server = run_eval.ModelServer(answers=task['answers'])
        self.addCleanup(server.close)
        server.hold_workers = threading.Event()
        result = []

        def ask():
            conn = http.client.HTTPConnection('127.0.0.1', server.server.server_port, timeout=10)
            conn.request('POST', '/api/chat', json.dumps({'messages': [{'role': 'user', 'content':
                         task['answers']['workers'][0]['prefix']}]}), {'Content-Type': 'application/json'})
            result.append(conn.getresponse().read())

        thread = threading.Thread(target=ask)
        thread.start()
        deadline = time.monotonic() + 5
        while not server.worker_requests and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(1, server.worker_requests)
        self.assertEqual([], result, 'held until released')
        server.hold_workers.set()
        thread.join(5)
        self.assertEqual(1, len(result))


class ChatTaskTests(unittest.TestCase):
    def setUp(self):
        self.task = next(t for t in run_eval.load_tasks(EVAL) if t['mode'] == 'chat')

    def ask(self, server, messages, tools=True):
        conn = http.client.HTTPConnection('127.0.0.1', server.server.server_port, timeout=5)
        body = {'model': 'fixture', 'messages': messages}
        if tools:
            body['tools'] = [{'type': 'function', 'function': {'name': 'read_file'}}]
        conn.request('POST', '/api/chat', json.dumps(body), {'Content-Type': 'application/json'})
        lines = [json.loads(line) for line in conn.getresponse().read().decode().splitlines() if line.strip()]
        return lines[-1]

    def test_fixture_answers_each_tool_round_with_the_scripted_calls(self):
        server = run_eval.ModelServer(answers=self.task['answers'])
        self.addCleanup(server.close)
        script = self.task['answers']['chat']
        messages = [{'role': 'system', 'content': 'ctx'}, {'role': 'user', 'content': self.task['goal']}]
        for round_ in script:
            final = self.ask(server, messages)
            self.assertTrue(final['done'])
            if 'tool_calls' in round_:
                calls = final['message']['tool_calls']
                self.assertEqual([c['name'] for c in round_['tool_calls']], [c['function']['name'] for c in calls])
                messages += [{'role': 'assistant', 'content': '', 'tool_calls': calls},
                             {'role': 'tool', 'content': 'ok', 'tool_name': calls[0]['function']['name']}]
            else:
                self.assertEqual(round_['content'], final['message']['content'])
        self.assertEqual([], server.errors)
        self.assertEqual({'chat'}, {r['kind'] for r in server.snapshot()})
        self.assertTrue(all(isinstance(r['prompt'], int) for r in server.snapshot()))

    @staticmethod
    def call(tool, path=None, result='ok'):
        return {'tool': tool, 'path': path, 'error': result != 'ok', 'result': result}

    def test_chat_facts_count_calls_error_classes_and_denials(self):
        call = self.call
        result = {'state': 'done', 'policy': 'workspace-edits', 'reply': 'Fixed.', 'steps': 3,
                  'calls': [call('grep'), call('read_file', 'units.py'), call('read_file', 'gone.py', 'error'),
                            call('nope', None, 'malformed'), call('edit_file', 'units.py'),
                            call('edit_file', 'units.py', 'error'), call('write_file', 'x', 'denied')],
                  'approvals': [{'tool': 'edit_file', 'target': 'edit units.py', 'outcome': 'applied', 'reason': None},
                                {'tool': 'edit_file', 'target': 'edit units.py', 'outcome': 'failed',
                                 'reason': 'changed'},
                                {'tool': 'write_file', 'target': 'new file x', 'outcome': 'denied', 'reason': None}],
                  'error': None}
        facts = run_eval.chat_facts(result)
        self.assertEqual(7, facts['tool_calls'])
        self.assertEqual({'malformed': 1, 'refused': 1, 'sandbox': 0}, facts['errors'],
                         'a well-formed call the tool refused and a failed approval are not malformed')
        self.assertEqual(2, facts['tool_errors'])
        self.assertEqual(['grep', 'read_file', 'read_file', 'nope', 'edit_file', 'edit_file', 'write_file'],
                         facts['chat']['calls'])
        self.assertEqual(1, facts['chat']['applied'])
        self.assertEqual('workspace-edits', facts['chat']['policy'])
        self.assertEqual(result['approvals'], facts['approvals'])

    def test_read_before_edit_needs_a_read_of_the_edited_file_before_the_first_applied_edit(self):
        call = self.call
        for calls, expected in [
            ([call('read_file', 'units.py'), call('edit_file', 'units.py')], True),
            ([call('grep'), call('edit_file', 'units.py')], True),
            ([call('grep', 'src'), call('edit_file', 'src/units.py')], True),
            ([call('grep', 'other.py'), call('edit_file', 'units.py')], False),
            ([call('read_file', 'other.py'), call('edit_file', 'units.py')], False),
            ([call('edit_file', 'units.py'), call('read_file', 'units.py')], False),
            ([call('read_file', 'units.py', 'error'), call('write_file', 'units.py')], False),
            ([call('write_file', 'units.py')], False),
            ([call('read_file', 'units.py'), call('edit_file', 'units.py', 'denied')], False),
        ]:
            with self.subTest(calls=calls):
                self.assertEqual(expected, run_eval.chat_facts({'calls': calls})['chat']['read_before_edit'])

    def test_chat_verdict_needs_check_pass_and_the_read_edit_flow(self):
        good = {'chat': {'read_before_edit': True}}
        blind = {'chat': {'read_before_edit': False}}
        self.assertEqual((True, 'ok'), run_eval.chat_verdict(True, 'ok', good))
        self.assertEqual((False, 'bad'), run_eval.chat_verdict(False, 'bad', good))
        passed, output = run_eval.chat_verdict(True, 'ok', blind)
        self.assertFalse(passed)
        self.assertIn('read_before_edit', output)
        self.assertFalse(run_eval.chat_verdict(True, 'ok', None)[0], 'no chat result: no flow evidence')

    def test_chat_command_is_headless_and_scoped_to_workspace_edits(self):
        argv = run_eval.chat_command(Path('/bin/alfredo'), 'http://127.0.0.1:1', Path('/w'), Path('/s'), 'm', 'goal')
        self.assertEqual(['--chat', 'goal', '--headless', '--approve', 'workspace-edits'], argv[-5:])
        self.assertIn('--workspace', argv)
        self.assertIn('--state-dir', argv)

    def test_schema_accepts_chat_fields_and_rejects_a_bad_outcome(self):
        trial = good_trial(self.task['id'])
        trial['approvals'] = [{'tool': 'edit_file', 'target': 'edit units.py', 'outcome': 'applied', 'reason': None}]
        trial['chat'] = {'policy': 'workspace-edits', 'reply': 'ok', 'steps': 3, 'calls': ['grep', 'edit_file'],
                         'read_before_edit': True, 'applied': 1}
        document = SchemaTests().document()
        document['trials'][0] = trial
        self.assertEqual([], schema_check.validate(document))
        trial['approvals'][0]['outcome'] = 'skipped'
        self.assertTrue(any('not in' in p for p in schema_check.validate(document)))


class CodeHealthTests(unittest.TestCase):
    """M15: the static code-health numbers (eval/code_health.py)."""

    def scan(self, source):
        return code_health.scan_source(source)

    def test_inference_admission_shape_counts_only_code_before_the_all_test_module(self):
        # inference_admission.rs gates its test module with #[cfg(all(test, unix))]: a scan that only knows
        # #[cfg(test)] counts every unwrap inside it (181 instead of 86 across src at 5f7912f).
        source = ('fn real() { a.unwrap(); }\n'
                  '\n'
                  '#[cfg(all(test, unix))]\n'
                  'mod tests {\n'
                  '    use super::*;\n'
                  '    fn t() { b.unwrap(); c.expect("x"); }\n'
                  '}\n')
        result = self.scan(source)
        self.assertEqual(1, result['unwrap_expect'])
        self.assertEqual(2, result['loc_nontest'])
        self.assertEqual(7, result['loc_total'])

    def test_cfg_test_all_test_and_any_test_are_test_code_but_not_test_is_not(self):
        for attribute in ('#[cfg(test)]', '#[cfg(all(test, unix))]', '#[cfg(any(test, feature = "x"))]',
                          '  #[cfg(all(test, target_os = "linux"))]'):
            result = self.scan(f'{attribute}\nmod tests {{ fn t() {{ x.unwrap(); }} }}\nfn real() {{}}\n')
            self.assertEqual(0, result['unwrap_expect'], attribute)
            self.assertEqual(1, result['loc_nontest'], attribute)
        result = self.scan('#[cfg(not(test))]\nfn real() { x.unwrap(); }\n')
        self.assertEqual(1, result['unwrap_expect'])
        self.assertEqual(2, result['loc_nontest'])

    def test_code_after_a_test_module_is_still_counted(self):
        source = ('fn a() { x.unwrap(); }\n#[cfg(test)]\nmod tests {\n    fn t() { y.unwrap(); }\n}\n'
                  'fn b() { z.expect("late"); }\n')
        result = self.scan(source)
        self.assertEqual(2, result['unwrap_expect'])
        self.assertEqual(2, result['loc_nontest'])

    def test_a_gated_use_fn_or_statement_hides_only_that_item(self):
        source = ('#[cfg(test)]\nuse std::{a, b};\n'
                  'fn real() {\n'
                  '    #[cfg(all(test, unix))]\n'
                  '    pause(x.unwrap());\n'
                  '    y.unwrap();\n'
                  '}\n'
                  '#[cfg(test)]\n#[allow(dead_code)]\nfn helper(a: [u8; 2]) -> u8 { a.unwrap(); }\n'
                  'fn tail() {}\n')
        result = self.scan(source)
        self.assertEqual(1, result['unwrap_expect'])
        self.assertEqual(4, result['loc_nontest'])

    def test_comments_strings_and_lookalike_calls_do_not_count(self):
        source = ('// a.unwrap();\n'
                  '/* b.unwrap(); /* nested.expect("x") */ */\n'
                  'fn f() {\n'
                  '    let s = "c.unwrap()";\n'
                  '    let r = r#"d.expect("q")"#;\n'
                  '    e.unwrap_or(1); f.expect_err("no"); g.unwrap_or_else(|| 1);\n'
                  '    let c = \'{\'; let l: &\'static str = "}";\n'
                  '    h.unwrap().expect("two");\n'
                  '}\n')
        result = self.scan(source)
        self.assertEqual(2, result['unwrap_expect'])

    def test_braces_inside_strings_chars_and_comments_do_not_end_a_test_module(self):
        source = ('#[cfg(test)]\nmod tests {\n    fn t() { let a = "}"; let b = \'}\'; /* } */ x.unwrap(); }\n}\n'
                  'fn real() { y.unwrap(); }\n')
        result = self.scan(source)
        self.assertEqual(1, result['unwrap_expect'])
        self.assertEqual(1, result['loc_nontest'])

    def test_inner_cfg_test_attribute_makes_the_whole_file_test_code(self):
        result = self.scan('#![cfg(test)]\nfn t() { x.unwrap(); }\n')
        self.assertEqual((0, 0), (result['unwrap_expect'], result['loc_nontest']))

    def test_scan_tree_picks_the_largest_non_test_module_and_ignores_tests_dir(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'src').mkdir()
            (root / 'tests').mkdir()
            (root / 'src/big.rs').write_text('fn a() {}\n#[cfg(test)]\nmod t {\n' + 'fn x() {}\n' * 50 + '}\n')
            (root / 'src/small.rs').write_text('fn a() {}\nfn b() { x.unwrap(); }\nfn c() {}\n')
            (root / 'tests/it.rs').write_text('fn t() { x.unwrap(); }\n')
            tree = code_health.scan_tree(root)
        self.assertEqual({'path': 'src/small.rs', 'loc_nontest': 3}, tree['largest_module'])
        self.assertEqual(1, tree['unwrap_expect_nontest'])
        self.assertEqual(4, tree['total_loc'])

    def test_clippy_counts_diagnostics_not_summary_lines_and_splits_lib_from_bin(self):
        output = ('src/a.rs:1:1: warning: first\nsrc/b.rs:2:3: warning: second: help: add a `;` here\n'
                  'warning: `alfredo-tui` (lib) generated 2 warnings (run `cargo clippy --fix`)\n'
                  'src/main.rs:9:1: warning: third\n'
                  'warning: `alfredo-tui` (bin "alfredo-tui") generated 1 warning\n')
        self.assertEqual({'pedantic_lib': 2, 'pedantic_bin': 1, 'pedantic': 3}, code_health.parse_clippy(output))
        with self.assertRaises(code_health.CodeHealthError):
            code_health.parse_clippy('src/a.rs:1:1: warning: x\nwarning: `alfredo-tui` (lib) generated 5 warnings\n')

    def test_cargo_test_list_counts_tests_and_ignored_tests(self):
        listing = 'a::t1: test\na::t2: test\nb::t3: test\n\n3 tests, 0 benchmarks\n'
        self.assertEqual(3, code_health.count_listed_tests(listing))
        self.assertEqual(0, code_health.count_listed_tests('\n0 tests, 0 benchmarks\n'))

    def test_python_tests_are_the_def_test_methods_in_tests_test_star_py(self):
        with tempfile.TemporaryDirectory() as directory:
            tests = Path(directory) / 'tests'
            tests.mkdir()
            (tests / 'test_a.py').write_text('class A:\n    def test_x(self): pass\n    def helper(self): pass\n'
                                             '    def test_y(self): pass\n')
            (tests / 'x_smoke.py').write_text('def test_z(): pass\n')
            self.assertEqual(2, code_health.count_python_tests(Path(directory)))

    def test_m15_is_null_with_its_reason_without_the_option_and_filled_with_it(self):
        _s, metrics, _w, _b = self.aggregate_with(None)
        self.assertIsNone(metrics['M15']['value'])
        self.assertIn('--code-health', metrics['M15']['reason'])
        health = {'pedantic': 877, 'pedantic_lib': 837, 'pedantic_bin': 40, 'unwrap_expect_nontest': 86,
                  'largest_module': {'path': 'src/task_control.rs', 'loc_nontest': 2933}, 'total_loc': 100,
                  'rust_tests': 804, 'rust_tests_ignored': 13, 'py_tests': 89}
        metrics = self.aggregate_with(health)[1]
        self.assertEqual(877, metrics['M15']['value'])
        self.assertEqual({'path': 'src/task_control.rs', 'loc_nontest': 2933},
                         metrics['M15']['secondary']['largest_module'])
        self.assertEqual(86, metrics['M15']['secondary']['unwrap_expect_nontest'])
        doc = {'schema_version': 1, 'agent': 'alfredo', 'backend': 'fixture', 'model': 'fixture', 'config': {},
               'trials': [], 'tasks': {}, 'metrics': metrics, 'warnings': [], 'likely_broken': False}
        self.assertEqual([], [e for e in schema_check.validate(doc) if 'M15' in e])

    def aggregate_with(self, health):
        return run_eval.aggregate([good_trial('A')], [{'id': 'A', 'kind': 'positive'}],
                                  {'trials': 1, 'backend': 'fixture'}, None, health)

    def test_code_health_flag_parses_for_both_agents(self):
        self.assertTrue(run_eval.parse_args(['--agent', 'alfredo', '--code-health']).code_health)
        self.assertFalse(run_eval.parse_args(['--agent', 'alfredo']).code_health)
        self.assertTrue(run_eval.parse_args(['--agent', 'claude-code', '--code-health']).code_health)


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

    def test_planner_tokens_are_read_from_saved_plan_receipts_and_cross_checked(self):
        with tempfile.TemporaryDirectory() as tmp:
            state = Path(tmp)
            folder = state / 'rust-tasks-v1' / 'abc'
            folder.mkdir(parents=True)
            self.assertIsNone(run_eval.planner_tokens(state))
            receipts = [{'request': {'action': {'kind': 'plan', 'plan': {'usage': {'prompt_tokens': 700, 'completion_tokens': 9}}}}},
                        {'request': {'action': {'kind': 'plan', 'plan': {}}}},
                        {'request': {'action': {'kind': 'approve', 'task': 1}}}]
            (folder / 'tasks.json').write_text(json.dumps({'tasks': [], 'receipts': receipts}))
            self.assertEqual(709, run_eval.planner_tokens(state))
        self.assertIsNone(run_eval.planner_token_mismatch(709, 709))
        self.assertIsNone(run_eval.planner_token_mismatch(None, None))
        self.assertIn('counted 709, Alfredo saved 700', run_eval.planner_token_mismatch(709, 700))
        self.assertIn('saved none', run_eval.planner_token_mismatch(709, None))


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
