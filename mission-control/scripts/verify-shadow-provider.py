"""Verify one installed Rust shadow provider against the installed Python contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import signal
import subprocess
import threading
import time
from pathlib import Path
from typing import Callable, TypeVar

from albert_mvp.execution import (
    ExecutionCoordinator,
    ExecutionJournal,
    ExecutionLimits,
    ExecutionRequest,
    ExecutionSandbox,
    LocalAgentExecutionAuthority,
    PythonExecutionProvider,
)
from albert_mvp.execution_shadow import (
    RustShadowProvider,
    RustShadowProviderError,
    compare_execution_receipts,
    normalize_structured_failure,
    shadow_artifact_sha256,
)


_Result = TypeVar("_Result")


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _capture_stores(roots: tuple[Path, ...]) -> dict[str, str]:
    return {str(root): shadow_artifact_sha256(root) for root in roots}


def _run_rust_sample(
    sample_id: str,
    canonical_roots: tuple[Path, ...],
    action: Callable[[], _Result],
) -> _Result:
    before = _capture_stores(canonical_roots)
    try:
        return action()
    finally:
        after = _capture_stores(canonical_roots)
        if before != after:
            changed = sorted(
                path
                for path in set(before) | set(after)
                if before.get(path) != after.get(path)
            )
            raise RuntimeError(
                f"{sample_id} changed canonical stores: {changed}"
            )


def _request(
    *,
    bwrap: Path,
    system_roots: list[str],
    workspace: Path,
    request_id: str,
    command: tuple[str, ...],
    timeout_seconds: float = 5,
    output_limit_bytes: int = 4096,
    descendant_grace_seconds: float = 0.1,
) -> ExecutionRequest:
    limits = ExecutionLimits(
        timeout_seconds=timeout_seconds,
        output_limit_bytes=output_limit_bytes,
        descendant_grace_seconds=descendant_grace_seconds,
    )
    return ExecutionRequest(
        request_id=request_id,
        effect="local-agent",
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
            str(workspace),
            "--bind",
            str(workspace),
            str(workspace),
            "--",
            "/usr/bin/prlimit",
            f"--as={limits.address_space_bytes}",
            f"--fsize={limits.file_size_bytes}",
            f"--nofile={limits.open_file_limit}",
            f"--nproc={limits.process_count_limit}",
            "--",
            *command,
        ),
        working_directory=str(workspace),
        authority=LocalAgentExecutionAuthority(
            mission_id="release-shadow-mission",
            session_id="release-shadow-session",
            session_revision=1,
            runner_operation_id="runner:release-shadow-session:1",
            worktree_identity="managed:release-shadow-session",
        ),
        limits=limits,
        sandbox=ExecutionSandbox(
            mode="bubblewrap",
            readable_roots=(str(workspace),),
            writable_roots=(str(workspace),),
        ),
    )


def _require_parity(
    request: ExecutionRequest,
    rust: RustShadowProvider,
    *,
    expected_status: str,
    canonical_roots: tuple[Path, ...],
) -> dict[str, str]:
    python_receipt = PythonExecutionProvider().execute(request)
    rust_receipt = _run_rust_sample(
        request.request_id,
        canonical_roots,
        lambda: rust.execute(request),
    )
    comparison = compare_execution_receipts(python_receipt, rust_receipt)
    if comparison.mismatches:
        raise RuntimeError(
            f"{request.request_id} Python/Rust receipt parity failed: "
            f"{comparison.mismatches}"
        )
    if rust_receipt.status != expected_status:
        raise RuntimeError(
            f"{request.request_id} returned {rust_receipt.status}, expected {expected_status}"
        )
    return {
        "request_id": request.request_id,
        "request_sha256": request.request_digest,
        "status": rust_receipt.status,
        "store_unchanged": True,
    }


def _require_validation_parity(
    request: ExecutionRequest,
    rust: RustShadowProvider,
    canonical_roots: tuple[Path, ...],
) -> dict[str, str]:
    try:
        PythonExecutionProvider().execute(request)
    except ValueError as exc:
        python_failure = normalize_structured_failure(
            {"code": "contract-failure", "message": str(exc), "recoverable": True}
        )
    else:
        raise RuntimeError(f"{request.request_id} Python validation unexpectedly passed")
    try:
        _run_rust_sample(
            request.request_id,
            canonical_roots,
            lambda: rust.execute(request),
        )
    except RustShadowProviderError as exc:
        rust_failure = normalize_structured_failure(exc)
    else:
        raise RuntimeError(f"{request.request_id} Rust validation unexpectedly passed")
    if python_failure != rust_failure:
        raise RuntimeError(
            f"{request.request_id} structured failure parity failed: "
            f"Python={python_failure!r}, Rust={rust_failure!r}"
        )
    return {
        "request_id": request.request_id,
        "request_sha256": request.request_digest,
        "status": "contract-failure",
        "store_unchanged": True,
    }


def _require_state_version_parity(
    request: ExecutionRequest,
    provider: Path,
    canonical_roots: tuple[Path, ...],
) -> dict[str, str]:
    raw_request = request.to_dict(include_input=True)
    raw_request["schema_version"] = 2
    try:
        ExecutionRequest.from_dict(raw_request)
    except ValueError as exc:
        python_failure = normalize_structured_failure(
            {"code": "contract-failure", "message": str(exc), "recoverable": True}
        )
    else:
        raise RuntimeError("Python accepted an unsupported ExecutionRequest schema")
    raw_envelope = json.dumps(
        {"request": raw_request},
        ensure_ascii=True,
        sort_keys=True,
        separators=(",", ":"),
    )
    completed = _run_rust_sample(
        request.request_id,
        canonical_roots,
        lambda: subprocess.run(
            [str(provider)],
            input=f"{raw_envelope}\n",
            text=True,
            capture_output=True,
            timeout=10,
            check=False,
        ),
    )
    if completed.returncode != 0:
        raise RuntimeError("Rust provider exited during the state-version cohort")
    try:
        response = json.loads(completed.stdout)
        rust_failure = normalize_structured_failure(response["failure"])
    except (KeyError, TypeError, json.JSONDecodeError) as exc:
        raise RuntimeError("Rust provider returned invalid state-version evidence") from exc
    if response.get("ok") is not False or python_failure != rust_failure:
        raise RuntimeError(
            "ExecutionRequest state-version failure parity failed: "
            f"Python={python_failure!r}, Rust={rust_failure!r}"
        )
    return {
        "request_id": request.request_id,
        "request_sha256": hashlib.sha256(raw_envelope.encode("utf-8")).hexdigest(),
        "status": "contract-failure",
        "store_unchanged": True,
    }


def _require_timeout_cleanup_parity(
    request: ExecutionRequest,
    rust: RustShadowProvider,
    canonical_roots: tuple[Path, ...],
    cleanup_marker: Path,
) -> dict[str, object]:
    cleanup_marker.unlink(missing_ok=True)
    python_receipt = PythonExecutionProvider().execute(request)
    time.sleep(0.35)
    if cleanup_marker.exists():
        raise RuntimeError(
            f"{request.request_id} Python descendant survived timeout cleanup"
        )

    rust_receipt = _run_rust_sample(
        request.request_id,
        canonical_roots,
        lambda: rust.execute(request),
    )
    time.sleep(0.35)
    if cleanup_marker.exists():
        raise RuntimeError(
            f"{request.request_id} Rust descendant survived timeout cleanup"
        )
    comparison = compare_execution_receipts(python_receipt, rust_receipt)
    if comparison.mismatches:
        raise RuntimeError(
            f"{request.request_id} Python/Rust receipt parity failed: "
            f"{comparison.mismatches}"
        )
    if rust_receipt.status != "timed-out":
        raise RuntimeError(
            f"{request.request_id} returned {rust_receipt.status}, expected timed-out"
        )
    return {
        "request_id": request.request_id,
        "request_sha256": request.request_digest,
        "status": rust_receipt.status,
        "store_unchanged": True,
        "cleanup_verified": True,
    }


def verify(
    provider: Path,
    workspace: Path,
    canonical_roots: tuple[Path, ...] = (),
) -> dict[str, object]:
    provider = provider.resolve(strict=True)
    workspace = workspace.resolve(strict=True)
    sentinel = workspace / "shadow-release-sentinel"
    sentinel.write_text("canonical\n", encoding="utf-8")
    governed_roots = (sentinel,) + tuple(
        root.resolve(strict=True) for root in canonical_roots
    )
    if len(governed_roots) != len(set(governed_roots)):
        raise RuntimeError("canonical store roots must be unique")
    store_before = _capture_stores(governed_roots)

    bwrap = next(
        candidate
        for candidate in (
            Path("/usr/bin/bwrap"),
            Path("/usr/sbin/bwrap"),
            Path("/bin/bwrap"),
            Path("/sbin/bwrap"),
        )
        if candidate.is_file()
    )
    system_roots = [
        root
        for root in ("/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc")
        if Path(root).exists()
    ]
    rust = RustShadowProvider((str(provider),))
    cleanup_script = workspace / "shadow-timeout-cleanup.sh"
    cleanup_marker = workspace / "shadow-timeout-descendant-survived"
    cleanup_script.write_text(
        "#!/bin/sh\n"
        "marker=\"$1\"\n"
        "( sleep 0.20; printf survived > \"$marker\" ) &\n"
        "sleep 2\n",
        encoding="utf-8",
    )
    cleanup_script.chmod(0o700)
    cases = [
        _require_parity(
            _request(
                bwrap=bwrap,
                system_roots=system_roots,
                workspace=workspace,
                request_id="packaged-shadow-completed",
                command=("/usr/bin/printf", "packaged parity"),
            ),
            rust,
            expected_status="completed",
            canonical_roots=governed_roots,
        ),
        _require_parity(
            _request(
                bwrap=bwrap,
                system_roots=system_roots,
                workspace=workspace,
                request_id="packaged-shadow-failed",
                command=("/usr/bin/false",),
            ),
            rust,
            expected_status="failed",
            canonical_roots=governed_roots,
        ),
        _require_timeout_cleanup_parity(
            _request(
                bwrap=bwrap,
                system_roots=system_roots,
                workspace=workspace,
                request_id="packaged-shadow-timeout-cleanup",
                command=(str(cleanup_script), str(cleanup_marker)),
                timeout_seconds=0.05,
            ),
            rust,
            governed_roots,
            cleanup_marker,
        ),
        _require_parity(
            _request(
                bwrap=bwrap,
                system_roots=system_roots,
                workspace=workspace,
                request_id="packaged-shadow-output-limit",
                command=("/usr/bin/yes", "bounded"),
                output_limit_bytes=1024,
            ),
            rust,
            expected_status="output-limit",
            canonical_roots=governed_roots,
        ),
    ]

    cancellation = _request(
        bwrap=bwrap,
        system_roots=system_roots,
        workspace=workspace,
        request_id="packaged-shadow-cancellation",
        command=("/usr/bin/sleep", "1"),
    )
    cancellation_deadline = time.monotonic() + 0.05

    def cancel_python() -> None:
        if time.monotonic() >= cancellation_deadline:
            raise RuntimeError("release cancellation")

    cancellation_journal = ExecutionJournal(
        workspace / "shadow-cancellation-receipts.json"
    )
    try:
        ExecutionCoordinator(
            cancellation_journal,
            PythonExecutionProvider(),
        ).execute(
            cancellation,
            poll_callback=cancel_python,
            exception_status=lambda _error: "cancelled",
        )
    except RuntimeError as exc:
        if str(exc) != "release cancellation":
            raise
    else:
        raise RuntimeError("Python cancellation did not interrupt the provider")
    python_cancelled = cancellation_journal.inspect()[0]
    rust_cancelled = _run_rust_sample(
        cancellation.request_id,
        governed_roots,
        lambda: rust.execute(cancellation, cancel_after_seconds=0.05),
    )
    cancellation_comparison = compare_execution_receipts(
        python_cancelled, rust_cancelled
    )
    if cancellation_comparison.mismatches:
        raise RuntimeError(
            "packaged cancellation parity failed: "
            f"{cancellation_comparison.mismatches}"
        )
    cases.append(
        {
            "request_id": cancellation.request_id,
            "request_sha256": cancellation.request_digest,
            "status": rust_cancelled.status,
            "store_unchanged": True,
        }
    )

    replay = _request(
        bwrap=bwrap,
        system_roots=system_roots,
        workspace=workspace,
        request_id="packaged-shadow-replay",
        command=("/usr/bin/printf", "replay once"),
    )
    replay_coordinator = ExecutionCoordinator(
        ExecutionJournal(workspace / "shadow-replay-receipts.json"),
        PythonExecutionProvider(),
    )
    replay_first = replay_coordinator.execute(replay)
    rust_replay = _run_rust_sample(
        replay.request_id,
        governed_roots,
        lambda: rust.execute(replay),
    )
    if compare_execution_receipts(replay_first, rust_replay).mismatches:
        raise RuntimeError("packaged replay request failed initial provider parity")
    replay_second = replay_coordinator.execute(replay)
    if (
        replay_second.request_digest != replay_first.request_digest
        or replay_second.status != replay_first.status
        or replay_second.stdout
        or replay_second.stderr
    ):
        raise RuntimeError("Python authority did not replay the canonical redacted receipt")
    cases.append(
        {
            "request_id": replay.request_id,
            "request_sha256": replay.request_digest,
            "status": "provider-free-replay",
            "store_unchanged": True,
        }
    )

    crash = _request(
        bwrap=bwrap,
        system_roots=system_roots,
        workspace=workspace,
        request_id="packaged-shadow-crash-cut",
        command=("/usr/bin/sleep", "1"),
    )

    def crash_python(process: object, _token: str) -> None:
        pid = getattr(process, "pid")
        try:
            if os.name == "posix":
                os.killpg(pid, signal.SIGKILL)
            else:
                process.kill()
        except (ProcessLookupError, PermissionError):
            pass
        try:
            process.communicate(timeout=5)
        except (AttributeError, subprocess.TimeoutExpired):
            pass
        raise OSError(
            "Rust shadow provider exited before a trustworthy receipt was observed."
        )

    crash_journal = ExecutionJournal(workspace / "shadow-crash-receipts.json")
    python_crash = ExecutionCoordinator(
        crash_journal,
        PythonExecutionProvider(),
    ).execute(crash, process_binding_started=crash_python)
    if python_crash.status != "outcome-unknown":
        raise RuntimeError("Python crash cut did not produce an uncertain outcome")

    def crash_rust_provider(process: object) -> None:
        pid = getattr(process, "pid")

        def terminate() -> None:
            time.sleep(0.05)
            try:
                if os.name == "posix":
                    os.killpg(pid, signal.SIGKILL)
                else:
                    process.kill()
            except (ProcessLookupError, PermissionError):
                pass

        threading.Thread(target=terminate, daemon=True).start()

    rust_crash = _run_rust_sample(
        crash.request_id,
        governed_roots,
        lambda: rust.execute(
            crash, provider_process_started=crash_rust_provider
        ),
    )
    crash_comparison = compare_execution_receipts(python_crash, rust_crash)
    if crash_comparison.mismatches:
        raise RuntimeError(
            "packaged provider crash-cut normalized parity failed: "
            f"{crash_comparison.mismatches}"
        )
    cases.append(
        {
            "request_id": crash.request_id,
            "request_sha256": crash.request_digest,
            "status": rust_crash.status,
            "store_unchanged": True,
            "normalized_parity": True,
        }
    )

    missing_resource = _request(
        bwrap=bwrap,
        system_roots=system_roots,
        workspace=workspace,
        request_id="packaged-shadow-resource-validation",
        command=("/usr/bin/true",),
    )
    resource_argv = list(missing_resource.argv)
    boundary = resource_argv.index("--")
    resource_argv[boundary + 1 :] = ["/usr/bin/true"]
    cases.append(
        _require_validation_parity(
            missing_resource.with_updates(argv=tuple(resource_argv)),
            rust,
            governed_roots,
        )
    )

    host_data = workspace / "shadow-host-data.txt"
    host_data.write_text("not executable\n", encoding="utf-8")
    undeclared_read = _request(
        bwrap=bwrap,
        system_roots=system_roots,
        workspace=workspace,
        request_id="packaged-shadow-sandbox-validation",
        command=("/usr/bin/printf", "safe"),
    )
    undeclared_argv = list(undeclared_read.argv)
    chdir = undeclared_argv.index("--chdir")
    undeclared_argv[chdir:chdir] = [
        "--ro-bind",
        str(host_data),
        str(host_data),
    ]
    undeclared_argv.append(str(host_data))
    cases.append(
        _require_validation_parity(
            undeclared_read.with_updates(argv=tuple(undeclared_argv)),
            rust,
            governed_roots,
        )
    )
    cases.append(
        _require_state_version_parity(
            _request(
                bwrap=bwrap,
                system_roots=system_roots,
                workspace=workspace,
                request_id="packaged-shadow-state-version",
                command=("/usr/bin/true",),
            ),
            provider,
            governed_roots,
        )
    )

    store_after = _capture_stores(governed_roots)
    if store_before != store_after:
        changed = sorted(
            path for path in set(store_before) | set(store_after)
            if store_before.get(path) != store_after.get(path)
        )
        raise RuntimeError(f"installed Rust shadow provider changed canonical stores: {changed}")
    suite_digest = hashlib.sha256(
        "\n".join(case["request_sha256"] for case in cases).encode("ascii")
    ).hexdigest()
    return {
        "status": "pass",
        "request_id": "packaged-shadow-contract-suite",
        "request_sha256": suite_digest,
        "provider_sha256": _sha256(provider),
        "receipt_status": "completed",
        "cohorts": cases,
        "canonical_store_roots": [str(root) for root in governed_roots],
        "store_unchanged": True,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", type=Path, required=True)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--canonical-root", type=Path, action="append", default=[])
    args = parser.parse_args()
    print(
        json.dumps(
            verify(args.provider, args.workspace, tuple(args.canonical_root)),
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
