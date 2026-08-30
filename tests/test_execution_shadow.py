from __future__ import annotations

import json
import os
import subprocess
import sys
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
    RustReleaseGateEvidence,
    RustShadowProvider,
    RustShadowProviderError,
    ShadowCohortDefinition,
    ShadowContractError,
    ShadowSampleMetadata,
    ShadowSampleRunner,
    ShadowStageMark,
    compare_execution_receipts,
    shadow_artifact_sha256,
)


class _StubRustProvider:
    provider_id = "rust-shadow"

    def __init__(
        self,
        receipt: ExecutionReceipt,
        mutate: Path | None = None,
        artifact: Path | None = None,
    ) -> None:
        self.receipt = receipt
        self.mutate = mutate
        self.command = (str(artifact),) if artifact is not None else ()

    def execute(self, request: ExecutionRequest) -> ExecutionReceipt:
        if self.mutate is not None:
            self.mutate.write_text("unauthorized mutation", encoding="utf-8")
        return replace(self.receipt, provider=self.provider_id)


class ExecutionShadowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.fixture_root = self.root / "fixture"
        self.worktree = self.fixture_root / "fixture-worktree"
        self.worktree.mkdir(parents=True)
        self.source_root = self.root / "source"
        self.source_root.mkdir()
        (self.source_root / "contract.json").write_text("source", encoding="utf-8")
        self.artifact = self.root / "rust-provider"
        self.artifact.write_text("verified provider", encoding="utf-8")

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
                "--unshare-user",
                "--unshare-pid",
                "--tmpfs",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
                "--chdir",
                working_directory,
                "--bind",
                working_directory,
                working_directory,
                "--",
                "/usr/bin/prlimit",
                "--as=8589934592",
                "--fsize=2147483648",
                "--nofile=1024",
                "--nproc=256",
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
            fixture_root=str(self.fixture_root.resolve()),
            fixture_sha256=shadow_artifact_sha256(self.fixture_root),
            source_sha256=shadow_artifact_sha256(self.source_root),
            artifact_sha256=shadow_artifact_sha256(self.artifact),
            required_stages=("S1", "R1"),
            source_root=str(self.source_root.resolve()),
            artifact_path=str(self.artifact.resolve()),
        )

    def _metadata(self, stage: str = "S1") -> ShadowSampleMetadata:
        return ShadowSampleMetadata(
            sample_id="sample-shadow-1",
            cohort_id="cohort-shadow-1",
            fixture_id="fixture-execution-contract-v1",
            fixture_sha256=shadow_artifact_sha256(self.fixture_root),
            source_sha256=shadow_artifact_sha256(self.source_root),
            artifact_sha256=shadow_artifact_sha256(self.artifact),
            fixture_root=str(self.fixture_root.resolve()),
            stage=stage,
        )

    def _stage_marks(self, metadata: ShadowSampleMetadata) -> tuple[ShadowStageMark, ...]:
        return tuple(
            ShadowStageMark(
                sample_id=metadata.sample_id,
                cohort_id=metadata.cohort_id,
                fixture_id=metadata.fixture_id,
                stage=stage,
                boundary=boundary,
                outcome="pass",
            )
            for stage in self._cohort().required_stages
            for boundary in ("start", "end")
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
            _StubRustProvider(python_receipt, artifact=self.artifact),
            self._cohort(),
            canonical_store_paths=(canonical,),
            eligibility_store=RustEligibilityStore(
                self.root / "shadow" / "rust-eligibility.json"
            ),
        )

        metadata = self._metadata()
        result = runner.run(
            request,
            python_receipt,
            metadata,
            stage_marks=self._stage_marks(metadata),
        )

        self.assertTrue(result.parity.passed)
        self.assertTrue(result.store_unchanged)
        self.assertFalse(result.eligible)
        self.assertEqual(result.eligibility.disabled_reason, "packaging")
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
        bwrap = next(
            (
                candidate
                for candidate in (
                    Path("/usr/bin/bwrap"),
                    Path("/usr/sbin/bwrap"),
                    Path("/bin/bwrap"),
                    Path("/sbin/bwrap"),
                )
                if candidate.is_file()
            ),
            None,
        )
        if bwrap is None:
            self.skipTest("trusted Bubblewrap is required for the cross-provider fixture")
        system_roots = [
            root
            for root in ("/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc")
            if Path(root).exists()
        ]
        request = self._request("shadow-jsonl-fixture").with_updates(
            argv=(
                str(bwrap),
                "--die-with-parent",
                "--new-session",
                "--unshare-user",
                "--unshare-pid",
                "--tmpfs",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
                *(item for root in system_roots for item in ("--ro-bind", root, root)),
                "--chdir",
                str(self.worktree.resolve()),
                "--bind",
                str(self.worktree.resolve()),
                str(self.worktree.resolve()),
                "--",
                "/usr/bin/prlimit",
                "--as=8589934592",
                "--fsize=2147483648",
                "--nofile=1024",
                "--nproc=256",
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

    def test_rust_provider_rejects_a_valid_receipt_from_a_crashed_process(self) -> None:
        request = self._request("shadow-provider-nonzero")
        python_receipt = self._python_receipt(request).to_dict()
        response_text = json.dumps({"ok": True, "receipt": python_receipt})
        script = (
            f"import sys; print({response_text!r}); "
            "sys.exit(7)"
        )
        crashed = RustShadowProvider((sys.executable, "-c", script)).execute(request)
        self.assertEqual(crashed.status, "outcome-unknown")
        self.assertTrue(crashed.reconciliation_required)

    def test_rust_jsonl_validation_matches_python_unprepared_boundary_failure(self) -> None:
        binary = Path(
            os.environ.get(
                "ALFREDO_RUST_EXECUTION_PROVIDER",
                "mission-control/src-tauri/target/debug/alfredo-execution-provider",
            )
        )
        if not binary.is_absolute():
            binary = (Path.cwd() / binary).resolve()
        if not binary.exists():
            self.skipTest("build alfredo-execution-provider to run the contract fixture")
        request = self._request("shadow-invalid-boundary").with_updates(
            argv=("/bin/sh", "-c", "echo unsafe")
        )
        with self.assertRaises(ValueError):
            PythonExecutionProvider(
                executor=lambda *_args, **_kwargs: subprocess.CompletedProcess(
                    [], 0, "", ""
                )
            ).execute(request)
        with self.assertRaises(RustShadowProviderError) as raised:
            RustShadowProvider((str(binary),)).execute(request)
        self.assertEqual(raised.exception.code, "contract-failure")

    def test_shadow_sample_reports_parity_and_store_failures_explicitly(self) -> None:
        request = self._request()
        python_receipt = self._python_receipt(request)
        canonical = self.root / "runtime.json"
        canonical.write_text("canonical", encoding="utf-8")
        rust_receipt = ExecutionReceipt.completed(
            request,
            exit_code=17,
            stdout=python_receipt.stdout,
            stderr=python_receipt.stderr,
        )
        runner = ShadowSampleRunner(
            _StubRustProvider(rust_receipt, mutate=canonical, artifact=self.artifact),
            self._cohort(),
            canonical_store_paths=(canonical,),
            eligibility_store=RustEligibilityStore(
                self.root / "shadow" / "rust-eligibility.json"
            ),
        )

        metadata = self._metadata()
        result = runner.run(
            request,
            python_receipt,
            metadata,
            stage_marks=self._stage_marks(metadata),
        )

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
                ShadowCohortDefinition(**{**self._cohort().__dict__, "evidence_kind": kind})

    def test_eligibility_requires_every_gate_and_disables_on_any_failure(self) -> None:
        store = RustEligibilityStore(self.root / "shadow" / "rust-eligibility.json")
        provider_digest = shadow_artifact_sha256(self.artifact)
        manifest = self.root / "package-manifest.json"
        manifest.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "provider_path": str(self.artifact.resolve()),
                    "provider_sha256": provider_digest,
                    "release_gate": "verified",
                },
                sort_keys=True,
                separators=(",", ":"),
            ),
            encoding="utf-8",
        )
        release_evidence = RustReleaseGateEvidence.from_verified_artifacts(
            provider_path=self.artifact,
            package_manifest_path=manifest,
        )
        passing = RustEligibilityEvidence.all_passed(
            sample_id="sample-shadow-1",
            cohort_id="cohort-shadow-1",
            stages=("S1", "R1"),
            release_evidence=release_evidence,
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
