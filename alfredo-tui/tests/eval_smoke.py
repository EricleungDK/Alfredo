#!/usr/bin/env python3
"""Fixture-mode harness eval end to end (stdlib only; run after cargo build).

Runs every eval task once against the scripted Ollama fixture, then checks that the JSON
validates against eval/schema.json, that positive tasks pass and the negative control fails,
and that no temp repo is left behind.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
EVAL = ROOT / 'alfredo-tui/eval'
sys.path.insert(0, str(EVAL))
import schema_check  # noqa: E402


class EvalSmoke(unittest.TestCase):
    def test_fixture_run_is_schema_valid_and_grades_as_scripted(self):
        binary = Path(os.environ.get('ALFREDO_TUI_BINARY', str(ROOT / 'alfredo-tui/target/debug/alfredo-tui')))
        self.assertTrue(binary.is_file(), f'Build or set ALFREDO_TUI_BINARY: {binary}')
        with tempfile.TemporaryDirectory(prefix='alfredo eval smoke ') as directory:
            scratch = Path(directory)
            out = scratch / 'out.json'
            run = subprocess.run([sys.executable, str(EVAL / 'run_eval.py'), '--agent', 'alfredo', '--backend',
                                  'fixture', '--trials', '1', '--binary', str(binary), '--tmp-root', directory,
                                  '--out', str(out)], capture_output=True, text=True, timeout=300)
            self.assertEqual(0, run.returncode, run.stderr)
            document = json.loads(out.read_text())
            self.assertEqual([], schema_check.validate(document))
            self.assertEqual(1, document['schema_version'])
            self.assertFalse(document['likely_broken'])
            self.assertEqual(str(binary.resolve()), document['config']['binary'])
            profile = document['config']['profile']
            self.assertIn(profile, ('debug', 'release', 'unknown'))
            self.assertEqual(['debug_profile'] if profile == 'debug' else [], [w['code'] for w in document['warnings']])
            self.assertEqual(sorted(t for t, v in document['tasks'].items() if v['kind'] == 'positive'),
                             sorted(document['metrics']['M4']['secondary']['per_task']))
            self.assertEqual(1.0, document['metrics']['M3']['secondary']['per_task'][
                next(t for t in document['tasks'] if t.startswith('T2-'))])
            for name, task in document['tasks'].items():
                self.assertTrue(task['as_expected'], name)
                self.assertEqual(0 if task['kind'] == 'negative_control' else 1, task['passes'], name)
            self.assertEqual(1.0, document['metrics']['M1']['value'])
            for trial in document['trials']:
                self.assertEqual(0, trial['exit_code'], trial['task'])
                self.assertFalse(trial['panic'], trial['task'])
            self.assertIsNotNone(document['metrics']['M6']['value'], 'fixture token counts are synthetic but present')
            self.assertEqual(0, document['metrics']['M12']['value'])
            repaired = next(t for t in document['trials'] if t['task'].startswith('T2-'))
            self.assertEqual(1, repaired['repairs'])
            chat = next(t for t in document['trials'] if t['task'].startswith('T7-'))
            self.assertTrue(chat['pass'], chat)
            self.assertEqual(['grep', 'read_file', 'edit_file'], chat['chat']['calls'])
            self.assertTrue(chat['chat']['read_before_edit'])
            self.assertEqual('workspace-edits', chat['chat']['policy'])
            self.assertEqual([('edit_file', 'applied')], [(a['tool'], a['outcome']) for a in chat['approvals']])
            self.assertEqual(4, chat['turns'])
            control = next(t for t in document['trials'] if t['kind'] == 'negative_control')
            self.assertGreater(control['errors']['malformed'], 0)
            self.assertEqual([out.name], sorted(p.name for p in scratch.iterdir()),
                             'temp repos and state dirs must be removed')

    def test_probes_fill_the_latency_and_recovery_rows(self):
        """Reduced counts keep this under ~10 s; the render probe (a release cargo build) is skipped."""
        binary = Path(os.environ.get('ALFREDO_TUI_BINARY', str(ROOT / 'alfredo-tui/target/debug/alfredo-tui')))
        with tempfile.TemporaryDirectory(prefix='alfredo eval probes ') as directory:
            out = Path(directory) / 'out.json'
            run = subprocess.run([sys.executable, str(EVAL / 'run_eval.py'), '--agent', 'alfredo', '--backend',
                                  'fixture', '--trials', '1', '--tasks', 'T1', '--binary', str(binary), '--tmp-root',
                                  directory, '--out', str(out), '--probes', '--no-render-probe', '--probe-chats', '3',
                                  '--probe-keys', '10', '--probe-recovery-trials', '1'],
                                 capture_output=True, text=True, timeout=120)
            self.assertEqual(0, run.returncode, run.stderr)
            document = json.loads(out.read_text())
            self.assertEqual([], schema_check.validate(document))
            metrics = document['metrics']
            for key in ('M8', 'M9', 'M14'):
                self.assertIsNotNone(metrics[key]['value'], f'{key}: {metrics[key]["reason"]}')
            self.assertEqual(3, metrics['M8']['secondary']['chats'])
            self.assertEqual(10, metrics['M9']['secondary']['keys'])
            self.assertEqual(1.0, metrics['M14']['value'])
            self.assertIn('--no-render-probe', metrics['M10']['reason'])
            self.assertEqual(document['config']['profile'], document['config']['probes']['profile'])


if __name__ == '__main__':
    unittest.main()
