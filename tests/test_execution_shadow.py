from __future__ import annotations

import base64
import hashlib
import json
import os
import runpy
import shutil
import subprocess
import sys
import tarfile
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

    def _verified_release_manifest(
        self, *, meta_platform_version: str = "0.1.0"
    ) -> Path:
        release = self.root / "verified-release"
        platform_package = self.root / "platform-package"
        provider = platform_package / "bin" / "alfredo-execution-provider"
        provider.parent.mkdir(parents=True)
        shutil.copyfile(self.artifact, provider)
        provider_digest = shadow_artifact_sha256(self.artifact)
        executable = platform_package / "bin" / "alfredo-desktop.AppImage"
        executable.write_text("verified desktop", encoding="utf-8")
        executable_digest = shadow_artifact_sha256(executable)
        (platform_package / "desktop.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "package": "alfredo-agent-linux-x64-gnu",
                    "version": "0.1.0",
                    "platform": "linux",
                    "arch": "x64",
                    "libc": "glibc",
                    "format": "appimage",
                    "executable": "bin/alfredo-desktop.AppImage",
                    "executable_sha256": executable_digest,
                    "shadow_provider": "bin/alfredo-execution-provider",
                    "shadow_provider_sha256": provider_digest,
                }
            ),
            encoding="utf-8",
        )
        (platform_package / "package.json").write_text(
            json.dumps(
                {
                    "name": "alfredo-agent-linux-x64-gnu",
                    "version": "0.1.0",
                    "os": ["linux"],
                    "cpu": ["x64"],
                    "libc": ["glibc"],
                }
            ),
            encoding="utf-8",
        )
        meta_package = self.root / "meta-package"
        meta_package.mkdir()
        (meta_package / "package.json").write_text(
            json.dumps(
                {
                    "name": "alfredo-agent",
                    "version": "0.1.0",
                    "bin": {
                        "alfredo": "bin/alfredo.js",
                        "albert": "bin/alfredo.js",
                    },
                    "optionalDependencies": {
                        "alfredo-agent-linux-x64-gnu": meta_platform_version
                    },
                }
            ),
            encoding="utf-8",
        )
        release.mkdir()
        platform_tarball = release / "alfredo-agent-linux-x64-gnu-0.1.0.tgz"
        with tarfile.open(platform_tarball, "w:gz") as archive:
            archive.add(platform_package, arcname="package")
        meta_tarball = release / "alfredo-agent-0.1.0.tgz"
        with tarfile.open(meta_tarball, "w:gz") as archive:
            archive.add(meta_package, arcname="package")

        def integrity(path: Path) -> str:
            digest = hashlib.sha512(path.read_bytes()).digest()
            return f"sha512-{base64.b64encode(digest).decode('ascii')}"

        manifest = release / "manifest.json"
        manifest.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "status": "verified",
                    "verification_kind": "production-appimage",
                    "publishable": True,
                    "package_version": "0.1.0",
                    "install_spec": "alfredo-agent@0.1.0",
                    "publish_order": [
                        "alfredo-agent-linux-x64-gnu",
                        "alfredo-agent",
                    ],
                    "packages": [
                        {
                            "role": "platform",
                            "name": "alfredo-agent-linux-x64-gnu",
                            "version": "0.1.0",
                            "filename": platform_tarball.name,
                            "bytes": platform_tarball.stat().st_size,
                            "sha256": shadow_artifact_sha256(platform_tarball),
                            "integrity": integrity(platform_tarball),
                        },
                        {
                            "role": "meta",
                            "name": "alfredo-agent",
                            "version": "0.1.0",
                            "filename": meta_tarball.name,
                            "bytes": meta_tarball.stat().st_size,
                            "sha256": shadow_artifact_sha256(meta_tarball),
                            "integrity": integrity(meta_tarball),
                        },
                    ],
                    "shadow_execution_provider": {
                        "package": "alfredo-agent-linux-x64-gnu",
                        "path": "bin/alfredo-execution-provider",
                        "sha256": provider_digest,
                        "contract": "python-rust-production-parity",
                        "verification": "installed-package",
                        "request_sha256": "a" * 64,
                        "store_unchanged": True,
                    },
                },
                sort_keys=True,
                separators=(",", ":"),
            ),
            encoding="utf-8",
        )
        return manifest

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
        cancelled_argv = list(request.argv)
        cancelled_argv[-2:] = ["/usr/bin/sleep", "1"]
        cancelled = RustShadowProvider((str(binary),)).execute(
            request.with_updates(
                request_id="shadow-jsonl-cancelled",
                argv=tuple(cancelled_argv),
            ),
            cancel_after_seconds=0.05,
        )
        self.assertEqual(cancelled.status, "cancelled")
        self.assertEqual(cancelled.error_code, "cancelled")
        self.assertEqual(cancelled.error_message, "release cancellation")

    def test_release_probe_hashes_each_rust_sample(self) -> None:
        binary = Path(
            os.environ.get(
                "ALFREDO_RUST_EXECUTION_PROVIDER",
                "mission-control/src-tauri/target/debug/alfredo-execution-provider",
            )
        )
        if not binary.is_absolute():
            binary = (Path.cwd() / binary).resolve()
        if not binary.exists():
            self.skipTest("build alfredo-execution-provider to run the release probe")
        if not any(
            candidate.is_file()
            for candidate in (
                Path("/usr/bin/bwrap"),
                Path("/usr/sbin/bwrap"),
                Path("/bin/bwrap"),
                Path("/sbin/bwrap"),
            )
        ):
            self.skipTest("trusted Bubblewrap is required for the release probe")

        canonical_root = self.root / "canonical-runtime"
        canonical_root.mkdir()
        canonical_file = canonical_root / "runtime.json"
        canonical_file.write_text("canonical\n", encoding="utf-8")
        counter = self.root / "provider-count"
        wrapper = self.root / "alternating-provider"
        wrapper.write_text(
            "\n".join(
                (
                    "#!/usr/bin/python3",
                    "import pathlib, subprocess, sys",
                    f"counter = pathlib.Path({str(counter)!r})",
                    f"canonical = pathlib.Path({str(canonical_file)!r})",
                    f"provider = {str(binary)!r}",
                    "count = int(counter.read_text()) if counter.exists() else 0",
                    "counter.write_text(str(count + 1))",
                    "canonical.write_text('mutated\\n' if count % 2 == 0 else 'canonical\\n')",
                    "completed = subprocess.run([provider], input=sys.stdin.buffer.read(), capture_output=True)",
                    "sys.stdout.buffer.write(completed.stdout)",
                    "sys.stderr.buffer.write(completed.stderr)",
                    "raise SystemExit(completed.returncode)",
                    "",
                )
            ),
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        script = runpy.run_path(
            str(Path("mission-control/scripts/verify-shadow-provider.py").resolve())
        )

        with self.assertRaisesRegex(
            RuntimeError, "packaged-shadow-completed changed canonical stores"
        ):
            script["verify"](wrapper, self.worktree, (canonical_root,))

    def test_release_probe_proves_cleanup_and_full_crash_parity(self) -> None:
        binary = Path(
            os.environ.get(
                "ALFREDO_RUST_EXECUTION_PROVIDER",
                "mission-control/src-tauri/target/debug/alfredo-execution-provider",
            )
        )
        if not binary.is_absolute():
            binary = (Path.cwd() / binary).resolve()
        if not binary.exists():
            self.skipTest("build alfredo-execution-provider to run the release probe")
        if not any(
            candidate.is_file()
            for candidate in (
                Path("/usr/bin/bwrap"),
                Path("/usr/sbin/bwrap"),
                Path("/bin/bwrap"),
                Path("/sbin/bwrap"),
            )
        ):
            self.skipTest("trusted Bubblewrap is required for the release probe")

        canonical_root = self.root / "installed-runtime"
        canonical_root.mkdir()
        script = runpy.run_path(
            str(Path("mission-control/scripts/verify-shadow-provider.py").resolve())
        )

        result = script["verify"](binary, self.worktree, (canonical_root,))
        cohorts = {case["request_id"]: case for case in result["cohorts"]}

        self.assertTrue(
            cohorts["packaged-shadow-timeout-cleanup"]["cleanup_verified"]
        )
        self.assertTrue(
            cohorts["packaged-shadow-crash-cut"]["normalized_parity"]
        )
        self.assertTrue(all(case["store_unchanged"] for case in cohorts.values()))

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
        manifest = self._verified_release_manifest()
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

    def test_release_evidence_rejects_a_synthetic_gate_manifest(self) -> None:
        manifest = self.root / "package-manifest.json"
        manifest.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "provider_path": str(self.artifact.resolve()),
                    "provider_sha256": shadow_artifact_sha256(self.artifact),
                    "release_gate": "verified",
                }
            ),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(ShadowContractError, "release manifest"):
            RustReleaseGateEvidence.from_verified_artifacts(
                provider_path=self.artifact,
                package_manifest_path=manifest,
            )

    def test_release_evidence_recomputes_package_sha512_integrity(self) -> None:
        manifest = self._verified_release_manifest()
        payload = json.loads(manifest.read_text(encoding="utf-8"))
        payload["packages"][0]["integrity"] = "sha512-invalid"
        manifest.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(ShadowContractError, "SHA-512 integrity"):
            RustReleaseGateEvidence.from_verified_artifacts(
                provider_path=self.artifact,
                package_manifest_path=manifest,
            )

    def test_release_evidence_requires_exact_package_identity(self) -> None:
        manifest = self._verified_release_manifest(meta_platform_version="9.9.9")

        with self.assertRaisesRegex(ShadowContractError, "exact platform version"):
            RustReleaseGateEvidence.from_verified_artifacts(
                provider_path=self.artifact,
                package_manifest_path=manifest,
            )

    def test_eligible_state_fails_closed_when_the_release_package_changes(self) -> None:
        manifest = self._verified_release_manifest()
        release_evidence = RustReleaseGateEvidence.from_verified_artifacts(
            provider_path=self.artifact,
            package_manifest_path=manifest,
        )
        store = RustEligibilityStore(self.root / "shadow" / "rust-eligibility.json")
        decision = store.record(
            RustEligibilityEvidence.all_passed(
                sample_id="sample-shadow-package",
                cohort_id="cohort-shadow-package",
                stages=("S1", "R1"),
                release_evidence=release_evidence,
            )
        )
        self.assertTrue(decision.eligible)
        platform_tarball = manifest.parent / "alfredo-agent-linux-x64-gnu-0.1.0.tgz"
        platform_tarball.write_bytes(platform_tarball.read_bytes() + b"tampered")

        reloaded = store.load()

        self.assertFalse(reloaded.eligible)
        self.assertEqual(reloaded.disabled_reason, "invalid-shadow-state")


if __name__ == "__main__":
    unittest.main()
