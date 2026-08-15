from __future__ import annotations

import json
from pathlib import Path
import shutil
import tempfile
import threading
import time
import unittest
from urllib.request import Request

from albert_mvp.core import AlbertError, AlbertMission
from albert_mvp.inference import (
    LocalInferenceAdapter,
    LocalInferenceLease,
    LocalInferenceLeaseError,
    LocalInferenceProfile,
    estimate_prompt_tokens,
)


class FakeResponse:
    def __init__(self, payload: bytes | list[bytes]) -> None:
        self._lines = payload if isinstance(payload, list) else None
        self._payload = payload if isinstance(payload, bytes) else None

    def __enter__(self) -> "FakeResponse":
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def read(self, _limit: int = -1) -> bytes:
        if self._payload is None:
            return b"".join(self._lines or [])
        return self._payload

    def readline(self, _limit: int = -1) -> bytes:
        if self._lines is None or not self._lines:
            return b""
        return self._lines.pop(0)


def running_model_response(
    model: str,
    digest: str,
    *,
    size: int = 1_000,
    size_vram: int = 800,
) -> FakeResponse:
    return FakeResponse(
        json.dumps(
            {
                "models": [
                    {
                        "name": model,
                        "model": model,
                        "digest": digest,
                        "size": size,
                        "size_vram": size_vram,
                    }
                ]
            }
        ).encode("utf-8")
    )


class LocalInferenceAdapterTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.runtime_root = Path(self.temporary_directory.name)
        self.requests: list[tuple[str, dict[str, object]]] = []

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def profile(self, **overrides: object) -> LocalInferenceProfile:
        values: dict[str, object] = {
            "profile_id": "controller-v1",
            "version": 1,
            "model": "qwen3:14b",
            "model_digest": "auto",
            "keep_alive": "10m",
            "context_budget": 512,
            "output_budget": 128,
            "thinking": False,
            "sampling": {"temperature": 0.2, "top_p": 0.9},
            "schema": {"type": "object", "required": ["answer"]},
            "quantization": "auto",
            "residency": "normal",
            "processor_placement": "gpu",
            "priority": 50,
            "queue_limit": 4,
            "max_queue_wait_seconds": 0.2,
            "timeout_seconds": 1.0,
            "max_output_bytes": 4096,
        }
        values.update(overrides)
        return LocalInferenceProfile.from_dict(values)

    def opener(self, request: Request, *, timeout: float) -> FakeResponse:
        del timeout
        payload = json.loads(request.data or b"{}")
        self.requests.append((request.full_url, payload))
        if request.full_url.endswith("/api/tags"):
            return FakeResponse(
                json.dumps(
                    {
                        "models": [
                            {
                                "name": "qwen3:14b",
                                "digest": "digest-qwen3",
                                "details": {"quantization_level": "Q4_K_M"},
                            }
                        ]
                    }
                ).encode("utf-8")
            )
        if request.full_url.endswith("/api/ps"):
            return running_model_response("qwen3:14b", "digest-qwen3")
        return FakeResponse(
            [
                b'{"model":"qwen3:14b","response":"{\\"answer\\":","done":false}\n',
                b'{"model":"qwen3:14b","response":"\\"ok\\"}","done":false}\n',
                b'{"model":"qwen3:14b","response":"","done":true,"load_duration":1000000,"prompt_eval_duration":2000000,"eval_duration":3000000,"prompt_eval_count":12,"eval_count":3}\n',
            ]
        )

    def test_success_records_resolved_profile_schema_usage_and_stage_timings(
        self,
    ) -> None:
        adapter = LocalInferenceAdapter(
            runtime_root=self.runtime_root,
            opener=self.opener,
        )

        result = adapter.infer(
            prompt="Return the answer object.",
            profile=self.profile(),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
        )

        self.assertTrue(result.authoritative)
        self.assertEqual(result.value, {"answer": "ok"})
        self.assertEqual(result.receipt["outcome"], "completed")
        self.assertEqual(result.receipt["profile"]["model_digest"], "digest-qwen3")
        self.assertEqual(result.receipt["profile"]["quantization"], "Q4_K_M")
        self.assertEqual(result.receipt["profile"]["keep_alive"], "10m")
        self.assertEqual(
            result.receipt["profile"]["processor_placement"],
            "requested=gpu;gpu_bytes=800;total_bytes=1000",
        )
        self.assertEqual(result.receipt["usage"]["prompt_tokens"], 12)
        self.assertEqual(result.receipt["usage"]["output_tokens"], 3)
        self.assertEqual(result.receipt["timings"]["load_ms"], 1.0)
        self.assertEqual(result.receipt["timings"]["prompt_evaluation_ms"], 2.0)
        self.assertGreaterEqual(result.receipt["timings"]["first_token_ms"], 0.0)
        self.assertEqual(result.receipt["timings"]["decoding_ms"], 3.0)
        generate_requests = [
            url for url, _ in self.requests if url.endswith("/api/generate")
        ]
        self.assertEqual(generate_requests, ["http://127.0.0.1:11434/api/generate"])
        payload = next(
            payload for url, payload in self.requests if url.endswith("/api/generate")
        )
        self.assertEqual(payload["keep_alive"], "10m")
        self.assertEqual(payload["options"]["num_ctx"], 512)
        self.assertEqual(payload["options"]["num_predict"], 128)
        self.assertEqual(payload["options"]["num_gpu"], -1)
        self.assertEqual(payload["think"], False)
        self.assertEqual(payload["raw"], True)
        self.assertEqual(
            [url for url, _ in self.requests if url.endswith("/api/ps")],
            ["http://127.0.0.1:11434/api/ps"],
        )

    def test_complete_result_requires_runtime_placement_and_residency_evidence(
        self,
    ) -> None:
        def opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"qwen3:14b","digest":"digest-qwen3"}]}'
                )
            if request.full_url.endswith("/api/ps"):
                return FakeResponse(b'{"models":[]}')
            return FakeResponse(
                [
                    b'{"response":"{\\"answer\\":\\"ok\\"}","done":true,"load_duration":1000000,"prompt_eval_duration":2000000,"eval_duration":3000000,"prompt_eval_count":12,"eval_count":3}\n'
                ]
            )

        result = LocalInferenceAdapter(
            runtime_root=self.runtime_root,
            opener=opener,
        ).infer(
            prompt="Return the answer object.",
            profile=self.profile(),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
        )

        self.assertFalse(result.authoritative)
        self.assertEqual(result.receipt["outcome"], "metadata-error")
        self.assertIn("running model metadata", result.receipt["error"])
        self.assertIsNone(result.value)

    def test_complete_result_rejects_running_model_digest_drift(self) -> None:
        def opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"qwen3:14b","digest":"digest-qwen3"}]}'
                )
            if request.full_url.endswith("/api/ps"):
                return running_model_response("qwen3:14b", "digest-changed")
            return FakeResponse(
                [
                    b'{"response":"{\\"answer\\":\\"ok\\"}","done":true,"load_duration":1000000,"prompt_eval_duration":2000000,"eval_duration":3000000,"prompt_eval_count":12,"eval_count":3}\n'
                ]
            )

        result = LocalInferenceAdapter(
            runtime_root=self.runtime_root,
            opener=opener,
        ).infer(
            prompt="Return the answer object.",
            profile=self.profile(),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
        )

        self.assertFalse(result.authoritative)
        self.assertEqual(result.receipt["outcome"], "digest-mismatch")
        self.assertIsNone(result.value)

    def test_complete_result_requires_all_stage_metrics(self) -> None:
        def opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"qwen3:14b","digest":"digest-qwen3"}]}'
                )
            if request.full_url.endswith("/api/ps"):
                return running_model_response("qwen3:14b", "digest-qwen3")
            return FakeResponse(
                [b'{"response":"{\\"answer\\":\\"ok\\"}","done":true}\n']
            )

        result = LocalInferenceAdapter(
            runtime_root=self.runtime_root,
            opener=opener,
        ).infer(
            prompt="Return the answer object.",
            profile=self.profile(),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
        )

        self.assertFalse(result.authoritative)
        self.assertEqual(result.receipt["outcome"], "malformed-stream")
        self.assertIn("usage and timing metrics", result.receipt["error"])

    def test_thinking_bytes_are_part_of_the_bounded_model_output(self) -> None:
        def opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"qwen3:14b","digest":"digest-qwen3"}]}'
                )
            if request.full_url.endswith("/api/ps"):
                return running_model_response("qwen3:14b", "digest-qwen3")
            return FakeResponse(
                [
                    b'{"thinking":"123456789","response":"{\\"answer\\":\\"ok\\"}","done":true,"load_duration":1000000,"prompt_eval_duration":2000000,"eval_duration":3000000,"prompt_eval_count":12,"eval_count":3}\n'
                ]
            )

        result = LocalInferenceAdapter(
            runtime_root=self.runtime_root,
            opener=opener,
        ).infer(
            prompt="Return the answer object.",
            profile=self.profile(thinking=True, max_output_bytes=20),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
        )

        self.assertFalse(result.authoritative)
        self.assertEqual(result.receipt["outcome"], "oversized-output")

    def test_token_admission_rejects_before_generate(self) -> None:
        adapter = LocalInferenceAdapter(
            runtime_root=self.runtime_root, opener=self.opener
        )

        result = adapter.infer(
            prompt="x" * 200,
            profile=self.profile(context_budget=20, output_budget=10),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
        )

        self.assertFalse(result.authoritative)
        self.assertEqual(result.receipt["outcome"], "rejected-over-budget")
        self.assertEqual(result.receipt["admission"]["prompt_tokens"], 200)
        self.assertEqual(
            [url for url, _ in self.requests if url.endswith("/api/generate")],
            [],
        )
        self.assertEqual(result.receipt["admission"]["admitted"], False)

    def test_complete_result_must_match_the_declared_schema(self) -> None:
        def opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"qwen3:14b","digest":"digest-qwen3"}]}'
                )
            return FakeResponse([b'{"response":"{\\"wrong\\":true}","done":true}\n'])

        result = LocalInferenceAdapter(
            runtime_root=self.runtime_root,
            opener=opener,
        ).infer(
            prompt="Return the answer object.",
            profile=self.profile(
                schema={
                    "type": "object",
                    "required": ["answer"],
                    "properties": {"answer": {"type": "string"}},
                    "additionalProperties": False,
                }
            ),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
        )

        self.assertFalse(result.authoritative)
        self.assertEqual(result.receipt["outcome"], "malformed-output")
        self.assertIn("schema validation", result.receipt["error"])

    def test_timeout_is_non_authoritative_even_when_the_stream_is_well_formed_so_far(
        self,
    ) -> None:
        clock_values = iter((0.0, 2.0))

        def opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"qwen3:14b","digest":"digest-qwen3"}]}'
                )
            return FakeResponse([b'{"response":"{","done":false}\n'])

        result = LocalInferenceAdapter(
            runtime_root=self.runtime_root,
            opener=opener,
            clock=lambda: next(clock_values),
        ).infer(
            prompt="small prompt",
            profile=self.profile(timeout_seconds=1.0),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
        )

        self.assertFalse(result.authoritative)
        self.assertEqual(result.receipt["outcome"], "timed-out")

    def test_cancellation_after_a_partial_chunk_remains_non_authoritative(self) -> None:
        checks = 0

        def should_cancel() -> bool:
            nonlocal checks
            checks += 1
            return checks >= 3

        def opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"qwen3:14b","digest":"digest-qwen3"}]}'
                )
            return FakeResponse(
                [
                    b'{"response":"{","done":false}\n',
                    b'{"response":"\\"answer\\":\\"ok\\"}","done":false}\n',
                ]
            )

        result = LocalInferenceAdapter(
            runtime_root=self.runtime_root,
            opener=opener,
        ).infer(
            prompt="small prompt",
            profile=self.profile(),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
            cancellation_requested=should_cancel,
        )

        self.assertFalse(result.authoritative)
        self.assertEqual(result.receipt["outcome"], "cancelled")
        self.assertEqual(result.raw_output, "{")

    def test_partial_malformed_and_oversized_streams_are_non_authoritative(
        self,
    ) -> None:
        streams = {
            "partial-stream": [b'{"response":"{","done":false}\n'],
            "malformed-stream": [b"not-json\n"],
            "oversized-output": [b'{"response":"123456789","done":true}\n'],
        }
        for expected, stream in streams.items():
            with self.subTest(expected=expected):
                calls: list[str] = []

                def opener(request: Request, *, timeout: float) -> FakeResponse:
                    del timeout
                    calls.append(request.full_url)
                    if request.full_url.endswith("/api/tags"):
                        return FakeResponse(
                            b'{"models":[{"name":"qwen3:14b","digest":"digest-qwen3","details":{"quantization_level":"Q4_K_M"}}]}'
                        )
                    return FakeResponse(stream.copy())

                result = LocalInferenceAdapter(
                    runtime_root=self.runtime_root / expected,
                    opener=opener,
                ).infer(
                    prompt="small prompt",
                    profile=self.profile(
                        max_output_bytes=8 if expected == "oversized-output" else 4096
                    ),
                    mission_id="mission-1",
                    session_id="session-1",
                    turn_kind="worker",
                    validator=json.loads,
                )

                self.assertFalse(result.authoritative)
                self.assertEqual(result.receipt["outcome"], expected)
                self.assertTrue(any(url.endswith("/api/generate") for url in calls))

    def test_cancelled_stream_is_structured_and_non_authoritative(self) -> None:
        cancelled = False

        def opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"qwen3:14b","digest":"digest-qwen3","details":{"quantization_level":"Q4_K_M"}}]}'
                )
            return FakeResponse(
                [
                    b'{"response":"{","done":false}\n',
                    b'{"response":"\\"answer\\":\\"ok\\"}","done":false}\n',
                ]
            )

        def should_cancel() -> bool:
            return cancelled

        adapter = LocalInferenceAdapter(runtime_root=self.runtime_root, opener=opener)
        cancelled = True
        result = adapter.infer(
            prompt="small prompt",
            profile=self.profile(),
            mission_id="mission-1",
            session_id="session-1",
            turn_kind="worker",
            validator=json.loads,
            cancellation_requested=should_cancel,
        )

        self.assertFalse(result.authoritative)
        self.assertEqual(result.receipt["outcome"], "cancelled")


class LocalInferenceLeaseTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.runtime_root = Path(self.temporary_directory.name)

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def test_one_lease_exposes_queue_priority_mission_attribution_and_audit(
        self,
    ) -> None:
        first = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-a",
            session_id="session-a",
            request_id="request-a",
            priority=10,
            max_queue_wait_seconds=1.0,
        )
        second = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-b",
            session_id="session-b",
            request_id="request-b",
            priority=90,
            max_queue_wait_seconds=1.0,
        )
        acquired: list[str] = []

        with first.acquire() as first_handle:
            self.assertEqual(first_handle.snapshot["mission_id"], "mission-a")
            thread = threading.Thread(
                target=lambda: self._acquire_and_release(second, acquired)
            )
            thread.start()
            time.sleep(0.03)
            state = LocalInferenceLease.inspect(self.runtime_root)
            self.assertEqual(state["active"]["session_id"], "session-a")
            self.assertEqual(state["queued"][0]["session_id"], "session-b")
            first_handle.mark_resident(model="qwen3:14b", digest="digest-qwen3")
        thread.join(timeout=1.0)

        self.assertEqual(acquired, ["request-b"])
        state = LocalInferenceLease.inspect(self.runtime_root)
        self.assertIsNone(state["active"])
        self.assertEqual(state["resident"]["digest"], "digest-qwen3")
        self.assertTrue(any(item["event"] == "released" for item in state["audit"]))
        self.assertTrue(
            any(item["mission_id"] == "mission-b" for item in state["audit"])
        )

    def test_queue_selects_highest_priority_before_waiting_fifo_work(self) -> None:
        holder = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-holder",
            session_id="session-holder",
            request_id="request-holder",
            priority=1,
            max_queue_wait_seconds=1.0,
        )
        lower_priority = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-low",
            session_id="session-low",
            request_id="request-low",
            priority=10,
            max_queue_wait_seconds=1.0,
        )
        higher_priority = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-high",
            session_id="session-high",
            request_id="request-high",
            priority=90,
            max_queue_wait_seconds=1.0,
        )
        acquired: list[str] = []
        errors: list[BaseException] = []
        first_acquired = threading.Event()
        release_workers = threading.Event()

        def acquire_and_hold(lease: LocalInferenceLease) -> None:
            try:
                with lease.acquire() as handle:
                    acquired.append(handle.snapshot["request_id"])
                    first_acquired.set()
                    release_workers.wait(timeout=1.0)
            except (
                BaseException
            ) as exc:  # pragma: no cover - assertion below reports it
                errors.append(exc)

        threads = [
            threading.Thread(target=acquire_and_hold, args=(lower_priority,)),
            threading.Thread(target=acquire_and_hold, args=(higher_priority,)),
        ]
        with holder.acquire():
            for thread in threads:
                thread.start()
            deadline = time.monotonic() + 1.0
            while time.monotonic() < deadline:
                queued_ids = {
                    item["request_id"]
                    for item in LocalInferenceLease.inspect(self.runtime_root)["queued"]
                }
                if {"request-low", "request-high"}.issubset(queued_ids):
                    break
                time.sleep(0.005)
            else:
                self.fail("both queued leases did not become visible")

        try:
            self.assertTrue(first_acquired.wait(timeout=1.0))
            self.assertEqual(errors, [])
            self.assertEqual(acquired[0], "request-high")
        finally:
            release_workers.set()
            for thread in threads:
                thread.join(timeout=1.0)

    def test_equal_priority_prefers_the_qualified_resident_model(self) -> None:
        holder = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-holder",
            session_id="session-holder",
            request_id="request-holder",
            priority=1,
            model="qwen3:14b",
            model_digest="digest-qwen3",
            max_queue_wait_seconds=1.0,
        )
        resident = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-resident",
            session_id="session-resident",
            request_id="request-resident",
            priority=50,
            model="qwen3:14b",
            model_digest="digest-qwen3",
            max_queue_wait_seconds=1.0,
        )
        swap = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-swap",
            session_id="session-swap",
            request_id="request-swap",
            priority=50,
            model="gemma4:12b",
            model_digest="digest-gemma",
            max_queue_wait_seconds=1.0,
        )
        acquired: list[str] = []
        errors: list[BaseException] = []
        first_acquired = threading.Event()
        release_workers = threading.Event()

        def acquire_and_hold(lease: LocalInferenceLease) -> None:
            try:
                with lease.acquire() as handle:
                    acquired.append(handle.snapshot["request_id"])
                    first_acquired.set()
                    release_workers.wait(timeout=1.0)
            except (
                BaseException
            ) as exc:  # pragma: no cover - assertion below reports it
                errors.append(exc)

        threads = [
            threading.Thread(target=acquire_and_hold, args=(swap,)),
            threading.Thread(target=acquire_and_hold, args=(resident,)),
        ]
        with holder.acquire() as holder_handle:
            holder_handle.mark_resident(model="qwen3:14b", digest="digest-qwen3")
            for thread in threads:
                thread.start()
            deadline = time.monotonic() + 1.0
            while time.monotonic() < deadline:
                queued_ids = {
                    item["request_id"]
                    for item in LocalInferenceLease.inspect(self.runtime_root)["queued"]
                }
                if {"request-resident", "request-swap"}.issubset(queued_ids):
                    break
                time.sleep(0.005)
            else:
                self.fail("both equal-priority leases did not become visible")

        try:
            self.assertTrue(first_acquired.wait(timeout=1.0))
            self.assertEqual(errors, [])
            self.assertEqual(acquired[0], "request-resident")
        finally:
            release_workers.set()
            for thread in threads:
                thread.join(timeout=1.0)

    def _acquire_and_release(
        self,
        lease: LocalInferenceLease,
        acquired: list[str],
    ) -> None:
        with lease.acquire() as handle:
            acquired.append(handle.snapshot["request_id"])

    def test_queued_lease_can_be_cancelled_without_running(self) -> None:
        first = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-a",
            session_id="session-a",
            request_id="request-a",
            max_queue_wait_seconds=1.0,
        )
        cancelled = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-b",
            session_id="session-b",
            request_id="request-b",
            max_queue_wait_seconds=1.0,
        )
        with first.acquire():
            with self.assertRaises(LocalInferenceLeaseError) as raised:
                cancelled.acquire(cancellation_requested=lambda: True).__enter__()
            self.assertEqual(raised.exception.outcome, "cancelled")
            state = LocalInferenceLease.inspect(self.runtime_root)
            self.assertEqual(state["active"]["request_id"], "request-a")
            self.assertEqual(state["queued"], [])

    def test_queue_full_is_structured_before_a_second_lease_runs(self) -> None:
        first = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-a",
            session_id="session-a",
            request_id="request-a",
            queue_limit=1,
            max_queue_wait_seconds=1.0,
        )
        second = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id="mission-b",
            session_id="session-b",
            request_id="request-b",
            queue_limit=1,
            max_queue_wait_seconds=1.0,
        )
        with first.acquire():
            with self.assertRaises(LocalInferenceLeaseError) as raised:
                second.acquire().__enter__()
            self.assertEqual(raised.exception.outcome, "queue-full")
            self.assertNotEqual(
                LocalInferenceLease.inspect(self.runtime_root)["active"]["request_id"],
                "request-b",
            )
            self.assertTrue(
                any(
                    item["event"] == "queue-full"
                    for item in LocalInferenceLease.inspect(self.runtime_root)["audit"]
                )
            )

    def test_corrupt_lease_entry_fails_closed(self) -> None:
        ledger_root = self.runtime_root / "inference"
        ledger_root.mkdir(parents=True)
        (ledger_root / "lease-state.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "active": None,
                    "queued": [{"request_id": "missing-identity"}],
                    "resident": None,
                    "audit": [],
                    "next_sequence": 2,
                }
            ),
            encoding="utf-8",
        )
        with self.assertRaises(LocalInferenceLeaseError) as raised:
            LocalInferenceLease.inspect(self.runtime_root)
        self.assertEqual(raised.exception.outcome, "ledger-invalid")


class LocalInferenceProfileTest(unittest.TestCase):
    def test_profile_is_versioned_and_rejects_invalid_budget(self) -> None:
        profile = LocalInferenceProfile.from_dict(
            {"version": 1, "profile_id": "worker-v1", "model": "gemma4:12b"}
        )
        self.assertEqual(profile.to_dict()["version"], 1)
        self.assertEqual(profile.to_dict()["model"], "gemma4:12b")
        with self.assertRaises(ValueError):
            LocalInferenceProfile.from_dict(
                {
                    "version": 1,
                    "profile_id": "invalid",
                    "model": "gemma4:12b",
                    "context_budget": 10,
                    "output_budget": 10,
                }
            )
        with self.assertRaises(ValueError):
            LocalInferenceProfile.from_dict(
                {
                    "version": 1,
                    "profile_id": "invalid",
                    "model": "gemma4:12b",
                    "sampling": "bad",
                }
            )
        with self.assertRaises(ValueError):
            LocalInferenceProfile.from_dict(
                {"version": 1, "profile_id": "invalid", "model": True}
            )
        with self.assertRaises(ValueError):
            LocalInferenceProfile.from_dict(
                {
                    "version": 1,
                    "profile_id": "invalid",
                    "model": "gemma4:12b",
                    "processor_placement": "somewhere",
                }
            )

    def test_prompt_token_estimate_is_bounded_and_deterministic(self) -> None:
        self.assertEqual(estimate_prompt_tokens(""), 0)
        self.assertEqual(estimate_prompt_tokens("1234"), 1)
        self.assertEqual(estimate_prompt_tokens("12345"), 2)


class AlbertMissionInferenceIntegrationTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        root = Path(self.temporary_directory.name)
        self.target = root / "target"
        self.target.mkdir()
        self.tracker = root / "tracker"
        self.issues = self.tracker / "issues"
        self.issues.mkdir(parents=True)
        (self.tracker / "PRD.md").write_text("# Inference Mission\n", encoding="utf-8")
        (self.issues / "01-root.md").write_text(
            "# Root\n\nStatus: ready-for-agent\nType: AFK\nRisk: Low\n"
            "Suggested agent: worker\nAssigned agent: worker\n\n"
            "## What to build\n\nRoute the work.\n\n"
            "## Acceptance criteria\n\n- [ ] Route is recorded.\n\n"
            "## Blocked by\n\nNone - can start immediately\n",
            encoding="utf-8",
        )
        self.runtime = root / "runtime"
        self.config = root / "agents.json"
        self.config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "router",
                            "role": "frontier",
                            "provider": "ollama",
                            "runner": "ollama",
                            "model": "qwen3:14b",
                            "routing": "router",
                            "inference_profile": {
                                "version": 1,
                                "profile_id": "router-v1",
                                "context_budget": 8_192,
                                "output_budget": 128,
                                "keep_alive": "10m",
                            },
                        },
                        {
                            "id": "worker",
                            "role": "local-agent",
                            "provider": "local",
                            "runner": "fake",
                            "routing": "worker",
                        },
                    ]
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    @staticmethod
    def opener(request: Request, *, timeout: float) -> FakeResponse:
        del timeout
        if request.full_url.endswith("/api/tags"):
            return FakeResponse(
                b'{"models":[{"name":"qwen3:14b","digest":"digest-router","details":{"quantization_level":"Q4_K_M"}}]}'
            )
        if request.full_url.endswith("/api/ps"):
            return running_model_response("qwen3:14b", "digest-router")
        return FakeResponse(
            [
                b'{"response":"{\\"complexity\\":\\"low\\",\\"recommended_agent\\":\\"worker\\",\\"requires_approval\\":false,\\"reason\\":\\"Bounded worker\\"}","done":true,"load_duration":1000000,"prompt_eval_duration":2000000,"eval_duration":3000000,"prompt_eval_count":10,"eval_count":7}\n'
            ]
        )

    def mission(self, opener: object = opener) -> AlbertMission:
        return AlbertMission(
            target_repo=self.target,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="mission-inference",
            agent_config_path=self.config,
            inference_opener=opener,
        ).load()

    def test_route_uses_http_profile_and_persists_receipt_without_authority_bypass(
        self,
    ) -> None:
        mission = self.mission()
        mission.approve_issue("ISS-01")

        decision = mission.route_issue("ISS-01")

        self.assertEqual(decision.recommended_agent, "worker")
        self.assertEqual(len(mission.inference_turns), 1)
        receipt = mission.inference_turns[0]
        self.assertEqual(receipt["profile"]["model_digest"], "digest-router")
        self.assertEqual(receipt["profile"]["keep_alive"], "10m")
        self.assertTrue(receipt["authoritative"])
        self.assertEqual(mission.delegations["ISS-01"].recommended_agent, "worker")
        reloaded = self.mission()
        self.assertEqual(
            reloaded.inference_turns[0]["request_id"], receipt["request_id"]
        )

    def test_non_authoritative_http_route_records_structured_failure_and_does_not_route(
        self,
    ) -> None:
        def malformed_opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"qwen3:14b","digest":"digest-router"}]}'
                )
            return FakeResponse([b'{"response":"partial","done":false}\n'])

        mission = self.mission(malformed_opener)
        mission.approve_issue("ISS-01")

        with self.assertRaisesRegex(AlbertError, "partial-stream"):
            mission.route_issue("ISS-01")

        self.assertEqual(mission.delegations, {})
        self.assertEqual(mission.inference_turns[0]["outcome"], "partial-stream")
        self.assertFalse(mission.inference_turns[0]["authoritative"])

    def test_persisted_receipt_schema_is_validated_on_reload(self) -> None:
        mission = self.mission()
        mission.approve_issue("ISS-01")
        mission.route_issue("ISS-01")
        runtime_path = mission.runtime_path
        payload = json.loads(runtime_path.read_text(encoding="utf-8"))
        payload["inference_turns"][0]["usage"]["output_tokens"] = "not-a-token-count"
        runtime_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(AlbertError, "usage record is invalid"):
            self.mission()

    def test_local_agent_uses_http_profile_and_keeps_only_valid_plan_authoritative(
        self,
    ) -> None:
        if shutil.which("bwrap") is None:
            self.skipTest("governed Local Agent execution requires Bubblewrap")
        self.config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "worker",
                            "role": "local-agent",
                            "provider": "ollama",
                            "runner": "ollama",
                            "model": "gemma4:12b",
                            "routing": "worker",
                            "inference_profile": {
                                "version": 1,
                                "profile_id": "worker-v1",
                                "context_budget": 16_384,
                                "output_budget": 128,
                                "keep_alive": "10m",
                            },
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )

        def worker_opener(request: Request, *, timeout: float) -> FakeResponse:
            del timeout
            if request.full_url.endswith("/api/tags"):
                return FakeResponse(
                    b'{"models":[{"name":"gemma4:12b","digest":"digest-worker","details":{"quantization_level":"Q4_K_M"}}]}'
                )
            if request.full_url.endswith("/api/ps"):
                return running_model_response("gemma4:12b", "digest-worker")
            return FakeResponse(
                [
                    b'{"response":"{\\"summary\\":\\"created\\",\\"files\\":[{\\"path\\":\\"result.txt\\",\\"content\\":\\"ok\\"}],\\"commands\\":[]}","done":true,"load_duration":1000000,"prompt_eval_duration":2000000,"eval_duration":3000000,"prompt_eval_count":10,"eval_count":6}\n'
                ]
            )

        mission = self.mission(worker_opener)
        mission.assign_issue("ISS-01", "worker")
        mission.approve_issue("ISS-01")

        session = mission.launch_issue("ISS-01")
        completed = mission.run_session(session.session_id)

        self.assertEqual(completed.status, "evidence-ready")
        self.assertEqual(len(mission.inference_turns), 1)
        self.assertTrue(mission.inference_turns[0]["authoritative"])
        self.assertEqual(mission.inference_turns[0]["session_id"], session.session_id)
        self.assertEqual(
            mission.board_summary()["issue_slices"][0]["sessions"][0]["inference"][
                "state"
            ],
            "authoritative",
        )
        self.assertTrue((session.worktree_path / "result.txt").exists())


if __name__ == "__main__":
    unittest.main()
