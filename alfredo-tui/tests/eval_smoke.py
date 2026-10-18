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
            self.assertEqual([], document['warnings'])
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
            control = next(t for t in document['trials'] if t['kind'] == 'negative_control')
            self.assertGreater(control['errors']['malformed'], 0)
            self.assertEqual([out.name], sorted(p.name for p in scratch.iterdir()),
                             'temp repos and state dirs must be removed')


if __name__ == '__main__':
    unittest.main()
