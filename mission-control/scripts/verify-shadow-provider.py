"""Verify one installed Rust shadow provider against the installed Python contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from albert_mvp.execution import (
    ExecutionLimits,
    ExecutionRequest,
    ExecutionSandbox,
    LocalAgentExecutionAuthority,
    PythonExecutionProvider,
)
from albert_mvp.execution_shadow import RustShadowProvider, compare_execution_receipts


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def verify(provider: Path, workspace: Path) -> dict[str, object]:
    provider = provider.resolve(strict=True)
    workspace = workspace.resolve(strict=True)
    sentinel = workspace / "shadow-release-sentinel"
    sentinel.write_text("canonical\n", encoding="utf-8")
    store_before = _sha256(sentinel)

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
    request = ExecutionRequest(
        request_id="packaged-shadow-release-parity",
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
            "--as=8589934592",
            "--fsize=2147483648",
            "--nofile=1024",
            "--nproc=256",
            "--",
            "/usr/bin/printf",
            "packaged parity",
        ),
        working_directory=str(workspace),
        authority=LocalAgentExecutionAuthority(
            mission_id="release-shadow-mission",
            session_id="release-shadow-session",
            session_revision=1,
            runner_operation_id="runner:release-shadow-session:1",
            worktree_identity="managed:release-shadow-session",
        ),
        limits=ExecutionLimits(timeout_seconds=5, output_limit_bytes=4096),
        sandbox=ExecutionSandbox(
            mode="bubblewrap",
            readable_roots=(str(workspace),),
            writable_roots=(str(workspace),),
        ),
    )
    python_receipt = PythonExecutionProvider().execute(request)
    rust_receipt = RustShadowProvider((str(provider),)).execute(request)
    comparison = compare_execution_receipts(python_receipt, rust_receipt)
    store_after = _sha256(sentinel)
    if comparison.mismatches:
        raise RuntimeError(
            f"installed Python/Rust receipt parity failed: {comparison.mismatches}"
        )
    if store_before != store_after:
        raise RuntimeError("installed Rust shadow provider changed the sentinel store")
    return {
        "status": "pass",
        "request_id": request.request_id,
        "request_sha256": request.request_digest,
        "provider_sha256": _sha256(provider),
        "receipt_status": rust_receipt.status,
        "stdout_sha256": rust_receipt.stdout_sha256,
        "store_unchanged": True,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", type=Path, required=True)
    parser.add_argument("--workspace", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.provider, args.workspace), sort_keys=True))


if __name__ == "__main__":
    main()
