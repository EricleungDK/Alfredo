#!/usr/bin/env python3
"""Exercise the installed diagnostic CLI against a bounded, observable fake Ollama."""
import hashlib
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import unittest


MODEL = 'qualification-fixture'
MODEL_DIGEST = 'b' * 64
PORT_SOLUTION = (
    'def parse_port(text):\n'
    '    if not isinstance(text, str) or not text:\n'
    '        return None\n'
    '    value = 0\n'
    '    for character in text:\n'
    '        if character < "0" or character > "9":\n'
    '            return None\n'
    '        value = value * 10 + ord(character) - ord("0")\n'
    '        if value > 65535:\n'
    '            return None\n'
    '    return value if 1 <= value <= 65535 else None\n'
)
METRICS = dict(total_duration=20_000_000, load_duration=1_000_000,
               prompt_eval_duration=4_000_000, eval_duration=15_000_000,
               prompt_eval_count=128, eval_count=64)


def wire(value, *, canonical=False):
    return json.dumps(value, sort_keys=canonical, separators=(',', ':'),
                      ensure_ascii=False).encode()


def sha(value):
    return hashlib.sha256(value).hexdigest()


def fixture_response(body):
    prompt = body['messages'][-1]['content']
    if prompt.startswith('Classify the decimal port numbers'):
        return {'valid': [1, 80, 443, 65535], 'invalid': [0, 65536]}, False
    if prompt.startswith('Implement this task:'):
        if 'transform(value)' in prompt:
            content = 'def transform(value):\n    return value * 17 + 23\n'
        elif 'QUALIFICATION_REPAIR_SEED' in prompt and not prompt.startswith('Implement this task: Repair'):
            content = 'def parse_port(text):\n    return 0\n'
        else:
            content = PORT_SOLUTION
        return {'files': [{'path': 'solution.py', 'content': content}]}, True
    decoder = json.JSONDecoder()
    policy, _ = decoder.raw_decode(prompt.split('The exact policy must be ', 1)[1])
    criteria, _ = decoder.raw_decode(prompt.split('Preserve these exact ordered acceptance criteria: ', 1)[1])
    if 'reference_left.py' in policy['files']:
        title = 'Implement solution.transform(value) from both required reference files'
    elif 'QUALIFICATION_REPAIR_SEED' in prompt:
        title = 'QUALIFICATION_REPAIR_SEED: keep parse_port returning 0 for this initial diagnostic attempt'
    else:
        title = 'Implement solution.parse_port with the exact ASCII decimal port contract'
    return {'tasks': [{'title': title, 'acceptance': criteria, 'model': body['model'],
                       'dependencies': [], 'policy': policy}]}, False


class FakeOllama(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, *, missing_digest=False):
        super().__init__(('127.0.0.1', 0), Handler)
        self.lock = threading.Lock()
        self.posts = []
        self.gets = []
        self.errors = []
        self.context = None
        self.missing_digest = missing_digest
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def endpoint(self):
        return f'http://127.0.0.1:{self.server_port}'

    def counts(self):
        with self.lock:
            return len(self.gets), len(self.posts)

    def close(self):
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=5)


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def respond(self, body, status=200, content_type='application/json'):
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        with self.server.lock:
            self.server.gets.append(self.path)
            context = self.server.context
        model = {'name': MODEL, 'model': MODEL, 'digest': MODEL_DIGEST}
        if self.path == '/api/version':
            result = {'version': '0.34.0'}
        elif self.path == '/api/tags':
            if self.server.missing_digest:
                model.pop('digest')
            model['details'] = {'quantization_level': 'Q4_K_M'}
            result = {'models': [model]}
        elif self.path == '/api/ps':
            model.update(context_length=context, size=2048, size_vram=1024)
            result = {'models': [] if context is None else [model]}
        else:
            self.respond(b'{}', status=404)
            return
        self.respond(wire(result))

    def do_POST(self):
        if self.path == '/api/generate':
            # Startup/model-selection preload: loads the model, generates nothing.
            self.rfile.read(int(self.headers.get('Content-Length', 0)))
            body = b'{"done":true,"done_reason":"load"}\n'
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        try:
            if self.path != '/api/chat':
                raise ValueError(f'Unexpected inference endpoint: {self.path}')
            size = int(self.headers['Content-Length'])
            if not 0 < size <= 4 * 1024 * 1024:
                raise ValueError('Unbounded generation payload')
            payload = self.rfile.read(size)
            body = json.loads(payload)
            with self.server.lock:
                self.server.posts.append((payload, body))
                self.server.context = body['options'].get('num_ctx', 4096)
            result, worker = fixture_response(body)
            if worker:
                # Keep the actual worker admitted long enough for the independently
                # scheduled foreground turn to observe a shared queue position.
                time.sleep(0.3)
            frame = {'message': {'content': wire(result).decode()}, 'done': True, **METRICS}
            self.respond(wire(frame) + b'\n', content_type='application/x-ndjson')
        except Exception as error:
            with self.server.lock:
                self.server.errors.append(repr(error))
            self.respond(wire({'error': str(error)}), status=500)


class QualificationCliSmoke(unittest.TestCase):
    def setUp(self):
        fallback = Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui'
        self.binary = Path(os.environ.get('ALFREDO_TUI_BINARY', str(fallback))).resolve()
        self.assertTrue(self.binary.is_file(), f'Build or set ALFREDO_TUI_BINARY: {self.binary}')
        temporary = tempfile.TemporaryDirectory(prefix='alfredo qualification installed ')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        # Every subprocess runs outside the source checkout with isolated app state.
        self.env = dict(os.environ, ALFREDO_STATE_DIR=str(self.root / 'unused-state'))

    def cli(self, *arguments):
        return subprocess.run([str(self.binary), *map(str, arguments)], cwd=self.root,
                              env=self.env, text=True, capture_output=True, timeout=180)

    def server(self, **options):
        server = FakeOllama(**options)
        self.addCleanup(server.close)
        return server

    def assert_receipt(self, snapshot, reference):
        matches = [receipt for receipt in snapshot['receipts']
                   if receipt['request']['correlation'] == reference['correlation']]
        self.assertEqual(len(matches), 1)
        receipt = matches[0]
        self.assertEqual(receipt['revision'], reference['revision'])
        self.assertEqual(receipt['task'], reference['task'])
        self.assertEqual(sha(wire(receipt)), reference['sha256'])
        return receipt

    def assert_wire_record(self, record, payload, body, endpoint):
        profile = record['profile']
        self.assertEqual(record['request_sha256'], sha(payload))
        self.assertEqual(record['request_bytes'], len(payload))
        self.assertEqual(record['profile_sha256'], sha(wire(profile, canonical=True)))
        self.assertEqual((profile['endpoint_origin'], profile['model'], profile['capacity']),
                         (endpoint, MODEL, 1))
        self.assertEqual((profile['connect_timeout_ms'], profile['idle_timeout_ms'], profile['total_timeout_ms']),
                         (5000, 60_000, 600_000))
        self.assertEqual(body['model'], MODEL)
        self.assertTrue(body['stream'])
        context = None if profile['context_profile'] == 'baseline' else (
            16384 if profile['class'] == 'background' else 8192)
        expected_options = {'num_predict': 4096}
        if context is not None:
            expected_options['num_ctx'] = context
        if 'format' in body:
            expected_options['temperature'] = 0
            self.assertIs(body['think'], False)
            self.assertIs(profile['think'], False)
            self.assertEqual(profile['temperature'], 0)
            self.assertEqual(profile['format_sha256'], sha(wire(body['format'], canonical=True)))
        else:
            self.assertNotIn('think', body)
            self.assertIsNone(profile['think'])
            self.assertIsNone(profile['temperature'])
            self.assertIsNone(profile['format_sha256'])
        self.assertEqual(body['options'], expected_options)
        self.assertEqual(profile['num_ctx'], context)
        self.assertNotIn('keep_alive', body)
        self.assertIsNone(profile['keep_alive'])
        self.assertEqual(len(record['messages']), len(body['messages']))
        for observed, actual in zip(record['messages'], body['messages']):
            self.assertEqual(set(observed), {'role', 'content_bytes', 'content_sha256', 'wire_sha256'})
            self.assertEqual(observed['role'], actual['role'])
            self.assertEqual(observed['content_bytes'], len(actual['content'].encode()))
            self.assertEqual(observed['content_sha256'], sha(actual['content'].encode()))
            self.assertEqual(observed['wire_sha256'], sha(wire(actual, canonical=True)))
        self.assertEqual(record['prefix_messages'], len(body['messages']) - 1)
        self.assertEqual(record['prefix_wire_sha256'], sha(wire(body['messages'][:-1], canonical=True)))
        prefix_binding = {key: record[key] for key in
                          ('profile_sha256', 'prefix_wire_sha256', 'prefix_messages')}
        self.assertEqual(record['prefix_sha256'], sha(wire(prefix_binding, canonical=True)))
        self.assertEqual(record['outcome'], 'completed')
        self.assertEqual(record['metrics'], METRICS)
        self.assertIsNone(record['runtime_error'])
        runtime = record['runtime_after']
        self.assertEqual((runtime['endpoint_origin'], runtime['selected_model'], runtime['server_version']),
                         (endpoint, MODEL, '0.34.0'))
        self.assertEqual(runtime['catalog'], {'digest': MODEL_DIGEST, 'quantization': 'Q4_K_M'})
        self.assertEqual(runtime['running'], {'digest': MODEL_DIGEST, 'context_length': context or 4096,
                                             'size': 2048, 'size_vram': 1024})
        self.assertGreaterEqual(record['first_content_ms'], record['queue_ms'])
        self.assertGreaterEqual(record['generation_ms'], record['first_content_ms'])
        self.assertGreaterEqual(record['total_ms'], record['generation_ms'])
        self.assertGreaterEqual(record['total_ms'], record['runtime_probe_ms'])

    def test_complete_cohort_wire_binding_canonical_reviews_and_read_only_inspection(self):
        server = self.server()
        path = self.root / 'complete report.json'
        completed = self.cli('--endpoint', server.endpoint, '--model', MODEL,
                             '--qualify-inference', path, '--qualification-repetitions', '1')
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertFalse(server.errors, server.errors)
        original = path.read_bytes()
        report = json.loads(original)
        self.assertTrue(report['finished'], report.get('stop_reason'))
        self.assertIn('8 canonical accepted, 8 with complete', completed.stdout)
        self.assertFalse(report['manifest']['upstream_binary_pin_verified'])
        self.assertEqual(report['manifest']['executable_sha256'], sha(self.binary.read_bytes()))
        self.assertIsNone(report['manifest']['initial_runtime']['running'])
        self.assertEqual(len(report['cases']), 8)
        expected_cases = [(scenario, profile) for scenario in
                          ('small-edit', 'required-source', 'repair', 'queued-foreground')
                          for profile in ('baseline', 'context-candidate')]
        self.assertEqual([(case['key']['scenario'], case['key']['profile'])
                          for case in report['cases']], expected_cases)
        records = []
        for index, case in enumerate(report['cases'], 1):
            self.assertEqual(case['phase'], 'finished')
            result = case['result']
            self.assertEqual(result['outcome'], 'accepted', result)
            self.assertEqual(result['scope_revision'], 2)
            self.assertTrue(result['required_sources_present'])
            self.assertIsNotNone(result['reviewed_elapsed_ms'])
            artifact = Path(report['manifest']['artifact_directory']) / f'case-{index:02}'
            self.assertEqual(Path(result['artifact_directory']), artifact)
            snapshots = list((artifact / 'state').rglob('tasks.json'))
            self.assertEqual(len(snapshots), 1)
            snapshot = json.loads(snapshots[0].read_bytes())
            plan_receipt = self.assert_receipt(snapshot, result['plan_receipt'])
            self.assertEqual(plan_receipt['request']['action']['kind'], 'plan')
            plan = plan_receipt['request']['action']['plan']
            self.assertEqual(plan['planner'], MODEL)
            self.assertEqual(plan_receipt['task'], result['runs'][0]['task'])
            self.assertEqual(len(plan['tasks']), 1)
            supplied = plan['context']['sources']
            self.assertEqual([(source['path'], source['sha256'], source['bytes'])
                              for source in result['planner_sources']],
                             [(source['path'], sha(source['content'].encode()),
                               len(source['content'].encode())) for source in supplied])
            expected_roles = [('foreground', 0)] + [('background', 1)] * len(result['runs'])
            if case['key']['scenario'] == 'queued-foreground':
                expected_roles.append(('foreground', 1))
            self.assertEqual([(request['profile']['class'], request['attempt'])
                              for request in case['requests']], expected_roles)
            self.assertEqual(result['generation_attempts'], len(case['requests']))
            worker_records = [record for record in case['requests']
                              if record['profile']['class'] == 'background']
            for run, record in zip(result['runs'], worker_records):
                task = next(task for task in snapshot['tasks'] if task['id'] == run['task'])
                self.assertEqual(task['status'], run['status'])
                self.assertEqual(task['run']['id'], run['run'])
                self.assertEqual(task.get('repair_of'), run['repair_of'])
                self.assertEqual(task['run']['evidence_sha256'], run['evidence_sha256'])
                evidence = list(artifact.rglob(f'{run["run"]}/evidence.json'))
                self.assertEqual(len(evidence), 1)
                self.assertEqual(sha(evidence[0].read_bytes()), run['evidence_sha256'])
                evidence_body = json.loads(evidence[0].read_bytes())
                self.assertEqual(evidence_body['run'], run['run'])
                self.assertEqual(evidence_body['check']['exit_code'], run['check_exit_code'])
                transcript_bytes = (evidence[0].parent / 'agent-conversation.json').read_bytes()
                self.assertEqual(sha(transcript_bytes), evidence_body['agent']['transcript_sha256'])
                transcript = json.loads(transcript_bytes)
                self.assertEqual((transcript['run'], transcript['model']), (run['run'], MODEL))
                actual_body = server.posts[record['sequence'] - 1][1]
                self.assertEqual(transcript['messages'][:-1], actual_body['messages'])
                review = run['review']['receipt']
                receipt = self.assert_receipt(snapshot, review)
                action = receipt['request']['action']
                expected_kind = 'review-and-repair' if run['review']['outcome'] == 'needs-repair' else 'decide'
                self.assertEqual(action['kind'], expected_kind)
                self.assertEqual(action['task'], run['task'])
                self.assertEqual(run['review']['reviewed_task'], run['task'])
                self.assertEqual(action['decision']['outcome'], run['review']['outcome'])
            self.assertEqual(result['runs'][-1]['status'], 'accepted')
            self.assertEqual(result['runs'][-1]['review']['outcome'], 'approved')
            if case['key']['scenario'] == 'repair':
                self.assertEqual(len(result['runs']), 2)
                self.assertFalse(result['runs'][0]['check_passed'])
                self.assertEqual(result['runs'][0]['review']['outcome'], 'needs-repair')
                self.assertEqual(result['runs'][1]['repair_of'], result['runs'][0]['task'])
            if case['key']['scenario'] == 'queued-foreground':
                self.assertTrue(result['foreground']['queue_observed'])
                self.assertTrue(result['foreground']['check_passed'])
                self.assertEqual(result['foreground']['background_request'], worker_records[0]['sequence'])
            records.extend(case['requests'])
        self.assertEqual(len(records), 20)
        self.assertEqual(len(server.posts), len(records))
        for sequence, (record, (payload, body)) in enumerate(zip(records, server.posts), 1):
            self.assertEqual(record['sequence'], sequence)
            self.assert_wire_record(record, payload, body, server.endpoint)
            # Hashes may appear in the report; complete model input and output may not.
            for message in body['messages']:
                self.assertNotIn(message['content'], original.decode())
        before = server.counts()
        inspected = self.cli('--inspect-qualification', path)
        self.assertEqual(inspected.returncode, 0, inspected.stdout + inspected.stderr)
        self.assertIn('8 canonical accepted, 8 with complete', inspected.stdout)
        self.assertEqual(server.counts(), before)
        self.assertEqual(path.read_bytes(), original)
        repeated = self.cli('--endpoint', server.endpoint, '--model', MODEL,
                            '--qualify-inference', path, '--qualification-repetitions', '1')
        self.assertNotEqual(repeated.returncode, 0)
        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(server.counts()[1], before[1])

        # Recompute public checksums so these reject inconsistent phase/request
        # relationships, rather than only noticing an unchanged outer checksum.
        before_resealed = server.counts()
        def inspect_resealed(name, changed, *, valid=False):
            changed['sha256'] = ''
            changed['sha256'] = sha(wire(changed))
            candidate = self.root / f'{name}.json'
            candidate.write_bytes(wire(changed))
            inspected = self.cli('--inspect-qualification', candidate)
            if valid:
                self.assertEqual(inspected.returncode, 0, inspected.stdout + inspected.stderr)
            else:
                self.assertNotEqual(inspected.returncode, 0, f'{name}: {inspected.stdout}')

        inspect_resealed('unchanged-resealed', json.loads(original), valid=True)
        changed = json.loads(original)
        del changed['cases'][0]['requests'][1]
        renumbered = {}
        sequence = 0
        for case in changed['cases']:
            for record in case['requests']:
                sequence += 1
                renumbered[record['sequence']] = sequence
                record['sequence'] = sequence
        for case in changed['cases']:
            foreground = case['result']['foreground']
            if foreground is not None:
                foreground['background_request'] = renumbered[foreground['background_request']]
        inspect_resealed('missing-worker-request', changed)
        changed = json.loads(original)
        queued = next(case for case in changed['cases']
                      if case['key']['scenario'] == 'queued-foreground')
        queued['result']['foreground']['background_request'] = changed['cases'][0]['requests'][1]['sequence']
        inspect_resealed('different-case-queue-owner', changed)
        changed = json.loads(original)
        changed['cases'][0]['requests'][0]['attempt'] = 1
        inspect_resealed('wrong-planner-attempt', changed)
        changed = json.loads(original)
        record = changed['cases'][0]['requests'][0]
        record['profile']['class'] = 'background'
        record['profile_sha256'] = sha(wire(record['profile'], canonical=True))
        prefix = {key: record[key] for key in
                  ('profile_sha256', 'prefix_wire_sha256', 'prefix_messages')}
        record['prefix_sha256'] = sha(wire(prefix, canonical=True))
        inspect_resealed('wrong-planner-role', changed)
        self.assertEqual(server.counts(), before_resealed)

        tampered = self.root / 'tampered.json'
        report['cases'][0]['requests'][0]['profile']['num_ctx'] = 12345
        tampered.write_bytes(wire(report))
        malformed = self.root / 'malformed.json'
        malformed.write_text('{broken')
        before = server.counts()
        for invalid in (tampered, malformed):
            self.assertNotEqual(self.cli('--inspect-qualification', invalid).returncode, 0)
        self.assertEqual(server.counts(), before)
        self.assertFalse((self.root / 'unused-state').exists())

    def test_missing_catalog_identity_retains_incomplete_report_without_generation(self):
        server = self.server(missing_digest=True)
        path = self.root / 'incomplete.json'
        completed = self.cli('--endpoint', server.endpoint, '--model', MODEL,
                             '--qualify-inference', path, '--qualification-repetitions', '1')
        self.assertEqual(completed.returncode, 2, completed.stdout + completed.stderr)
        report = json.loads(path.read_bytes())
        self.assertFalse(report['finished'])
        self.assertIsNotNone(report['stop_reason'])
        self.assertIsNone(report['manifest']['initial_runtime']['catalog']['digest'])
        self.assertEqual(len(report['cases']), 8)
        self.assertTrue(all(case['phase'] == 'pending' and not case['requests'] for case in report['cases']))
        self.assertFalse(server.posts)
        self.assertFalse(server.errors, server.errors)
        before = server.counts()
        inspected = self.cli('--inspect-qualification', path)
        self.assertEqual(inspected.returncode, 0, inspected.stdout + inspected.stderr)
        self.assertIn('0/8 attempted, 0 canonical accepted', inspected.stdout)
        self.assertEqual(server.counts(), before)
        self.assertFalse(list(Path(report['manifest']['artifact_directory']).glob('case-*')))


if __name__ == '__main__':
    unittest.main()
