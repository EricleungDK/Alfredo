from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path

from albert_mvp.execution import (
    ExecutionLimits,
    ExecutionReceipt,
    ExecutionRequest,
    ExecutionSandbox,
    LocalAgentExecutionAuthority,
    PythonExecutionProvider,
)
from albert_mvp.execution_shadow import (
    CanonicalStoreHashGuard,
    RustEligibilityEvidence,
    RustEligibilityStore,
    RustShadowProvider,
    RustShadowProviderError,
    ShadowCohortDefinition,
    ShadowContractError,
    ShadowSampleMetadata,
    ShadowSampleRunner,
    compare_execution_receipts,
)


class _StubRustProvider:
    provider_id = "rust-shadow"

    def __init__(self, receipt: ExecutionReceipt, mutate: Path | None = None) -> None:
        self.receipt = receipt
        self.mutate = mutate

    def execute(self, request: ExecutionRequest) -> ExecutionReceipt:
        if self.mutate is not None:
            self.mutate.write_text("unauthorized mutation", encoding="utf-8")
        return replace(self.receipt, provider=self.provider_id)


class ExecutionShadowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.worktree = self.root / "fixture-worktree"
        self.worktree.mkdir()

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def _request(self, request_id: str = "shadow-request-1") -> ExecutionRequest:
        working_directory = str(self.worktree.resolve())
        return ExecutionRequest(
            request_id=request_id,
            effect="local-agent",
            argv=(
                "/usr/bin/bwrap",
                "--die-with-parent",
                "--new-session",
                "--chdir",
                working_directory,
                "--bind",
                working_directory,
                working_directory,
                "--",
                "/bin/true",
            ),
            working_directory=working_directory,
            authority=LocalAgentExecutionAuthority(
                mission_id="shadow-mission",
                session_id="shadow-session",
                session_revision=3,
                runner_operation_id="runner:shadow-session:3",
                worktree_identity="managed:shadow-session",
                allowed_paths=("src",),
            ),
            limits=ExecutionLimits(
                timeout_seconds=2,
                output_limit_bytes=4096,
            ),
            sandbox=ExecutionSandbox(
                mode="bubblewrap",
                readable_roots=(working_directory,),
                writable_roots=(working_directory,),
            ),
        )

    def _python_receipt(self, request: ExecutionRequest) -> ExecutionReceipt:
        return PythonExecutionProvider(
            executor=lambda argv, **_kwargs: subprocess.CompletedProcess(
                argv, 0, "same output", ""
            )
        ).execute(request)

    def _cohort(self) -> ShadowCohortDefinition:
        return ShadowCohortDefinition(
            cohort_id="cohort-shadow-1",
            fixture_id="fixture-execution-contract-v1",
            fixture_root=str(self.root.resolve()),
            fixture_sha256="a" * 64,
            source_sha256="b" * 64,
            artifact_sha256="c" * 64,
            required_stages=("validation", "launch", "replay"),
        )

    def _metadata(self, stage: str = "launch") -> ShadowSampleMetadata:
        return ShadowSampleMetadata(
            sample_id="sample-shadow-1",
            cohort_id="cohort-shadow-1",
            fixture_id="fixture-execution-contract-v1",
            fixture_sha256="a" * 64,
            source_sha256="b" * 64,
            artifact_sha256="c" * 64,
            fixture_root=str(self.root.resolve()),
            stage=stage,
        )

    def test_canonical_store_guard_allows_only_explicit_observation_records(self) -> None:
        canonical = self.root / "runtime.json"
        observation = self.root / "shadow-observation.json"
        canonical.write_text("canonical", encoding="utf-8")
        observation.write_text("before", encoding="utf-8")

        guard = CanonicalStoreHashGuard(
            (canonical, observation), approved_observation_paths=(observation,)
        )
        before = guard.capture()
        observation.write_text("after", encoding="utf-8")
        after = guard.capture()

        self.assertEqual(before.unauthorized_changes(after), ())
        self.assertEqual(after.changed_paths(before), (str(observation.resolve()),))

        canonical.write_text("forged canonical", encoding="utf-8")
        forged = guard.capture()
        self.assertEqual(
            forged.unauthorized_changes(after), (str(canonical.resolve()),)
        )

    def test_canonical_store_guard_rejects_symlink_paths_and_replacement(self) -> None:
        canonical = self.root / "runtime.json"
        target = self.root / "target.json"
        canonical.write_text("canonical", encoding="utf-8")
        target.write_text("target", encoding="utf-8")
        alias = self.root / "alias.json"
        alias.symlink_to(target)

        with self.assertRaisesRegex(ShadowContractError, "non-symlink"):
            CanonicalStoreHashGuard((alias,))

        guard = CanonicalStoreHashGuard((canonical,))
        guard.capture()
        canonical.unlink()
        canonical.symlink_to(target)
        with self.assertRaisesRegex(ShadowContractError, "non-symlink"):
            guard.capture()

    def test_shadow_sample_matches_normalized_receipt_and_preserves_canonical_store(self) -> None:
        request = self._request()
        python_receipt = self._python_receipt(request)
        canonical = self.root / "runtime.json"
        canonical.write_text("canonical", encoding="utf-8")
        runner = ShadowSampleRunner(
            _StubRustProvider(python_receipt),
            self._cohort(),
            canonical_store_paths=(canonical,),
        )

        result = runner.run(request, python_receipt, self._metadata())

        self.assertTrue(result.parity.passed)
        self.assertTrue(result.store_unchanged)
        self.assertTrue(result.eligible)
        self.assertEqual(result.failure_codes, ())

    def test_jsonl_rust_provider_matches_python_contract_fixture(self) -> None:
        binary = Path(
            os.environ.get(
                "ALFREDO_RUST_EXECUTION_PROVIDER",
                "mission-control/src-tauri/target/debug/alfredo-execution-provider",
            )
        )
        if not binary.is_absolute():
            binary = (Path.cwd() / binary).resolve()
        if not binary.exists():
            self.skipTest("build alfredo-execution-provider to run the cross-provider fixture")
        fake_bwrap = self.worktree / "bwrap"
        fake_bwrap.write_text(
            "#!/bin/sh\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n",
            encoding="utf-8",
        )
        fake_bwrap.chmod(0o755)
        request = self._request("shadow-jsonl-fixture").with_updates(
            argv=(
                str(fake_bwrap),
                "--die-with-parent",
                "--new-session",
                "--chdir",
                str(self.worktree.resolve()),
                "--bind",
                str(self.worktree.resolve()),
                str(self.worktree.resolve()),
                "--",
                "/usr/bin/printf",
                "same output",
            )
        )
        python_receipt = self._python_receipt(request)
        rust_receipt = RustShadowProvider((str(binary),)).execute(request)
        self.assertEqual(
            compare_execution_receipts(python_receipt, rust_receipt).mismatches,
            (),
        )
        self.assertEqual(rust_receipt.status, "completed")

    def test_rust_provider_crash_and_structured_failure_are_non_eligible(self) -> None:
        request = self._request("shadow-provider-failure")
        crashed = RustShadowProvider(("/usr/bin/false",)).execute(request)
        self.assertEqual(crashed.status, "outcome-unknown")
        self.assertTrue(crashed.reconciliation_required)

        failure_command = (
            "/usr/bin/printf",
            '{"ok":false,"failure":{"code":"parity-failure","message":"fixture mismatch","recoverable":false}}\n',
        )
        with self.assertRaises(RustShadowProviderError) as raised:
            RustShadowProvider(failure_command).execute(request)
        self.assertEqual(raised.exception.code, "parity-failure")

    def test_shadow_sample_reports_parity_and_store_failures_explicitly(self) -> None:
        request = self._request()
        python_receipt = self._python_receipt(request)
        canonical = self.root / "runtime.json"
        canonical.write_text("canonical", encoding="utf-8")
        rust_receipt = replace(
            python_receipt,
            status="failed",
            exit_code=17,
            error_code="process-failed",
        )
        runner = ShadowSampleRunner(
            _StubRustProvider(rust_receipt, mutate=canonical),
            self._cohort(),
            canonical_store_paths=(canonical,),
        )

        result = runner.run(request, python_receipt, self._metadata())

        self.assertFalse(result.parity.passed)
        self.assertFalse(result.store_unchanged)
        self.assertFalse(result.eligible)
        self.assertEqual(
            result.failure_codes,
            ("canonical-store-mutated", "receipt-parity-failure"),
        )

    def test_shadow_cohort_rejects_reducer_sidecar_and_microbenchmark_evidence(self) -> None:
        for kind in ("reducer", "sidecar", "microbenchmark"):
            with self.subTest(kind=kind), self.assertRaisesRegex(
                ShadowContractError, "production-equivalent"
            ):
                ShadowCohortDefinition(
                    cohort_id="cohort-shadow-1",
                    fixture_id="fixture-execution-contract-v1",
                    fixture_root=str(self.root.resolve()),
                    fixture_sha256="a" * 64,
                    source_sha256="b" * 64,
                    artifact_sha256="c" * 64,
                    required_stages=("launch",),
                    evidence_kind=kind,
                )

    def test_eligibility_requires_every_gate_and_disables_on_any_failure(self) -> None:
        store = RustEligibilityStore(self.root / "shadow" / "eligibility.json")
        passing = RustEligibilityEvidence.all_passed(
            sample_id="sample-shadow-1",
            cohort_id="cohort-shadow-1",
            stages=("validation", "launch", "replay"),
        )
        enabled = store.record(passing)
        self.assertTrue(enabled.eligible)

        disabled = store.record(
            replace(passing, crash_cut_passed=False, failure_reasons=("crash-cut",))
        )
        self.assertFalse(disabled.eligible)
        self.assertEqual(disabled.disabled_reason, "crash-cut")
        self.assertFalse(store.load().eligible)

        incomplete = replace(passing, stages_complete=False)
        self.assertFalse(store.record(incomplete).eligible)


if __name__ == "__main__":
    unittest.main()
