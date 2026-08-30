"""Provider selection for effect-specific Rust execution cutovers.

Python owns authorization, the durable intent journal, canonical projections,
and receipt reconciliation.  This module selects one provider for a prepared
request and integrity-binds the packaged Rust JSONL process before it can be
used for an external effect.
"""

from __future__ import annotations

from dataclasses import replace
import hashlib
import os
from pathlib import Path
import re
import stat
from types import SimpleNamespace
from typing import Any, Callable, Mapping

from .execution import (
    ExecutionContractError,
    ExecutionExecutor,
    ExecutionProvider,
    ExecutionReceipt,
    ExecutionRequest,
    PythonExecutionProvider,
)
from .execution_shadow import RustShadowProvider, RustShadowProviderError


RUST_CANDIDATE_ENABLED = "ALFREDO_RUST_CANDIDATE_ENABLED"
RUST_SHELL_ENABLED = "ALFREDO_RUST_SHELL_ENABLED"
RUST_EXECUTION_PROVIDER = "ALFREDO_RUST_EXECUTION_PROVIDER"
RUST_EXECUTION_PROVIDER_SHA256 = "ALFREDO_RUST_EXECUTION_PROVIDER_SHA256"
_DIGEST_PATTERN = re.compile(r"^[0-9a-f]{64}$")
_MAX_PROVIDER_BYTES = 64 * 1024 * 1024


class RustExecutionProvider:
    """Integrity-bound canonical adapter for the qualified Rust JSONL process."""

    provider_id = "rust"

    def __init__(self, provider_path: Path, expected_sha256: str) -> None:
        self.provider_path = Path(provider_path)
        self.expected_sha256 = expected_sha256
        self._transport = RustShadowProvider((str(self.provider_path),))

    def _verify_artifact(self) -> None:
        if (
            not self.provider_path.is_absolute()
            or not isinstance(self.expected_sha256, str)
            or not _DIGEST_PATTERN.fullmatch(self.expected_sha256)
        ):
            raise ExecutionContractError(
                "Rust execution provider configuration is invalid"
            )
        try:
            entry = self.provider_path.lstat()
            resolved = self.provider_path.resolve(strict=True)
        except OSError as exc:
            raise ExecutionContractError(
                f"Rust execution provider is unavailable: {exc}"
            ) from exc
        if (
            stat.S_ISLNK(entry.st_mode)
            or not stat.S_ISREG(entry.st_mode)
            or not entry.st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
            or resolved != self.provider_path
            or entry.st_size > _MAX_PROVIDER_BYTES
        ):
            raise ExecutionContractError(
                "Rust execution provider must be a canonical executable regular file"
            )
        digest = hashlib.sha256()
        try:
            with self.provider_path.open("rb") as source:
                while chunk := source.read(1024 * 1024):
                    digest.update(chunk)
        except OSError as exc:
            raise ExecutionContractError(
                f"Rust execution provider could not be verified: {exc}"
            ) from exc
        if digest.hexdigest() != self.expected_sha256:
            raise ExecutionContractError(
                "Rust execution provider integrity does not match the packaged identity"
            )

    def validate_request(self, request: ExecutionRequest) -> None:
        self._verify_artifact()
        # Python repeats the exact prepared Bubblewrap/resource validation before
        # the journal claim; Rust independently validates the same request again.
        PythonExecutionProvider().validate_request(request)

    def verify_available(self) -> None:
        """Prove artifact identity before any request claim or external effect."""

        self._verify_artifact()

    def execute(
        self,
        request: ExecutionRequest,
        *,
        process_binding_started: Callable[[Any, str], None] | None = None,
        poll_callback: Callable[[], None] | None = None,
        output_callback: Callable[[str, bytes], None] | None = None,
    ) -> ExecutionReceipt:
        self.validate_request(request)
        effect_binding: dict[str, int | str] = {}

        def bind_effect(process_pid: int, process_identity: str) -> None:
            effect_binding.update(
                {"pid": process_pid, "identity": process_identity}
            )
            if process_binding_started is not None:
                process_binding_started(
                    SimpleNamespace(
                        pid=process_pid,
                        execution_identity=process_identity,
                    ),
                    "rust-effect",
                )

        try:
            receipt = self._transport.execute(
                request,
                effect_process_started=bind_effect,
                control_poll_callback=poll_callback,
            )
        except RustShadowProviderError as exc:
            # Once the provider subprocess accepted the request, a malformed or
            # lost response is not positive proof that no effect started.  Only
            # preflight validation and a typed start-failed receipt may make
            # that claim; every transport-contract failure is uncertain.
            return ExecutionReceipt.unknown(
                request,
                error_message=exc.message,
                process_pid=(
                    int(effect_binding["pid"]) if effect_binding else None
                ),
                process_identity=(
                    str(effect_binding["identity"]) if effect_binding else ""
                ),
                provider=self.provider_id,
            )
        if effect_binding and (
            receipt.process_pid != int(effect_binding["pid"])
            or receipt.process_identity != str(effect_binding["identity"])
        ):
            return ExecutionReceipt.unknown(
                request,
                error_message=(
                    "Rust execution receipt disagrees with the bound effect process."
                ),
                process_pid=int(effect_binding["pid"]),
                process_identity=str(effect_binding["identity"]),
                provider=self.provider_id,
            )
        canonical_receipt = replace(receipt, provider=self.provider_id)
        if output_callback is not None:
            if canonical_receipt.stdout:
                output_callback("stdout", canonical_receipt.stdout.encode("utf-8"))
            if canonical_receipt.stderr:
                output_callback("stderr", canonical_receipt.stderr.encode("utf-8"))
        return canonical_receipt


class _RejectedRustExecutionProvider:
    """A selected Rust boundary that proves configuration failed pre-effect."""

    provider_id = "rust"

    def __init__(self, message: str) -> None:
        self.message = message

    def validate_request(self, request: ExecutionRequest) -> None:
        request.validate()
        raise ExecutionContractError(self.message)

    def execute(
        self,
        request: ExecutionRequest,
        *,
        process_binding_started: Callable[[Any, str], None] | None = None,
        poll_callback: Callable[[], None] | None = None,
        output_callback: Callable[[str, bytes], None] | None = None,
    ) -> ExecutionReceipt:
        self.validate_request(request)
        raise AssertionError("rejected Rust provider executed after failed validation")


def shell_execution_provider_from_environment(
    environment: Mapping[str, str] | None = None,
    *,
    python_executor: ExecutionExecutor | None = None,
) -> ExecutionProvider:
    """Select exactly one Shell provider from the process launch boundary."""

    values = os.environ if environment is None else environment
    candidate_enabled = values.get(RUST_CANDIDATE_ENABLED, "0")
    shell_enabled = values.get(RUST_SHELL_ENABLED, "0")
    if candidate_enabled not in {"0", "1"}:
        return _RejectedRustExecutionProvider(
            f"{RUST_CANDIDATE_ENABLED} must be 0 or 1"
        )
    if shell_enabled not in {"0", "1"}:
        return _RejectedRustExecutionProvider(
            f"{RUST_SHELL_ENABLED} must be 0 or 1"
        )
    if candidate_enabled == "0" or shell_enabled == "0":
        return PythonExecutionProvider(executor=python_executor)
    provider_path = values.get(RUST_EXECUTION_PROVIDER, "")
    provider_sha256 = values.get(RUST_EXECUTION_PROVIDER_SHA256, "")
    return RustExecutionProvider(Path(provider_path), provider_sha256)


__all__ = [
    "RUST_CANDIDATE_ENABLED",
    "RUST_EXECUTION_PROVIDER",
    "RUST_EXECUTION_PROVIDER_SHA256",
    "RUST_SHELL_ENABLED",
    "RustExecutionProvider",
    "shell_execution_provider_from_environment",
]
