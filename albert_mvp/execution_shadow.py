"""Python-owned shadow orchestration for the Rust host-effects candidate.

The Python execution provider remains the canonical writer.  This module only
feeds explicitly bounded, production-equivalent fixtures to a Rust candidate,
compares transient results, and records whether the candidate is still eligible
for a future cutover.  Canonical Mission stores are treated as immutable
observation inputs during every shadow sample.
"""

from __future__ import annotations

import base64
import fcntl
import hashlib
import json
import math
import os
import re
import signal
import subprocess
import tarfile
import tempfile
import threading
import time
from collections.abc import Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass, field, replace
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

from .execution import ExecutionReceipt, ExecutionRequest

SHADOW_SCHEMA_VERSION = 1
_DIGEST_PATTERN = re.compile(r"^[0-9a-f]{64}$")
_IDENTITY_PATTERN = re.compile(r"^[A-Za-z0-9._:/=-]+$")
_MAX_STORE_BYTES = 64 * 1024 * 1024
_MAX_PROVIDER_OUTPUT_BYTES = 16 * 1024 * 1024
_MAX_PROVIDER_TIMEOUT_SECONDS = 3_600.0
_FORBIDDEN_EVIDENCE_KINDS = {"reducer", "sidecar", "microbenchmark"}
_MAX_SHADOW_TREE_FILES = 4_096
_MAX_SHADOW_TREE_BYTES = 128 * 1024 * 1024
_MAX_RELEASE_METADATA_BYTES = 1024 * 1024
_MAX_STAGE_MARKS = 64
_PRODUCTION_STAGES = {f"S{index}" for index in range(10)} | {
    f"R{index}" for index in range(7)
}
_PROVIDER_ENVIRONMENT_ALLOWLIST = {
    "CI",
    "COLORTERM",
    "HOME",
    "LANG",
    "LC_ALL",
    "LOGNAME",
    "NO_COLOR",
    "OLLAMA_HOST",
    "PATH",
    "PYTHONPATH",
    "SHELL",
    "TERM",
    "TMPDIR",
    "USER",
}
_PROVIDER_ENVIRONMENT_OVERRIDE_ALLOWLIST = {
    "ALBERT_SESSION_ID",
    "ALBERT_TASK_PACKET",
}


class ShadowContractError(ValueError):
    """Raised when shadow evidence cannot satisfy the production contract."""


class RustShadowProviderError(ShadowContractError):
    """A structured failure returned by, or observed at, the Rust adapter."""

    def __init__(self, code: str, message: str, recoverable: bool = True) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.recoverable = recoverable


def _utc_now() -> str:
    return (
        datetime.now(timezone.utc)
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z")
    )


def _validate_identity(value: str, *, label: str) -> str:
    if not isinstance(value, str) or not value.strip() or not _IDENTITY_PATTERN.fullmatch(value):
        raise ShadowContractError(f"{label} is invalid")
    return value


def _validate_digest(value: str, *, label: str) -> str:
    if not isinstance(value, str) or not _DIGEST_PATTERN.fullmatch(value):
        raise ShadowContractError(f"{label} must be a lowercase SHA-256 digest")
    return value


def _canonical_path(path: Path, *, label: str) -> Path:
    if not isinstance(path, Path):
        path = Path(path)
    if not path.is_absolute():
        raise ShadowContractError(f"{label} must be absolute")
    return path.resolve(strict=False)


def _hash_regular_file(path: Path, *, label: str) -> str:
    declared = Path(path)
    try:
        if declared.is_symlink() or not declared.is_file():
            raise ShadowContractError(
                f"{label} must be a regular non-symlink file"
            )
        if declared.stat().st_size > _MAX_SHADOW_TREE_BYTES:
            raise ShadowContractError(f"{label} exceeds the bounded artifact size")
    except OSError as exc:
        raise ShadowContractError(f"{label} could not be inspected: {exc}") from exc
    digest = hashlib.sha256()
    try:
        with declared.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                digest.update(chunk)
    except OSError as exc:
        raise ShadowContractError(f"{label} could not be hashed: {exc}") from exc
    return digest.hexdigest()


def _sha512_integrity(path: Path, *, label: str) -> str:
    declared = Path(path)
    digest = hashlib.sha512()
    try:
        with declared.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                digest.update(chunk)
    except OSError as exc:
        raise ShadowContractError(f"{label} could not be hashed: {exc}") from exc
    encoded = base64.b64encode(digest.digest()).decode("ascii")
    return f"sha512-{encoded}"


def _hash_bound_path(path: Path, *, label: str) -> str:
    """Hash one immutable file or deterministic directory manifest."""

    declared = Path(path)
    try:
        if declared.is_symlink():
            raise ShadowContractError(f"{label} must not be a symlink")
        if declared.is_file():
            return _hash_regular_file(declared, label=label)
        if not declared.is_dir():
            raise ShadowContractError(f"{label} must be a regular file or directory")
    except OSError as exc:
        raise ShadowContractError(f"{label} could not be inspected: {exc}") from exc

    entries: list[tuple[str, Path]] = []
    total_bytes = 0
    for root, directories, files in os.walk(declared, topdown=True, followlinks=False):
        root_path = Path(root)
        for directory in list(directories):
            if (root_path / directory).is_symlink():
                raise ShadowContractError(f"{label} contains a symlink directory")
        directories.sort()
        for name in sorted(files):
            file_path = root_path / name
            if file_path.is_symlink() or not file_path.is_file():
                raise ShadowContractError(f"{label} contains a non-regular file")
            size = file_path.stat().st_size
            total_bytes += size
            if total_bytes > _MAX_SHADOW_TREE_BYTES:
                raise ShadowContractError(f"{label} exceeds the bounded tree size")
            entries.append((file_path.relative_to(declared).as_posix(), file_path))
            if len(entries) > _MAX_SHADOW_TREE_FILES:
                raise ShadowContractError(f"{label} contains too many files")
    digest = hashlib.sha256()
    for relative, file_path in sorted(entries):
        relative_bytes = relative.encode("utf-8")
        digest.update(len(relative_bytes).to_bytes(8, "big"))
        digest.update(relative_bytes)
        try:
            with file_path.open("rb") as source:
                while chunk := source.read(1024 * 1024):
                    digest.update(chunk)
        except OSError as exc:
            raise ShadowContractError(f"{label} could not be hashed: {exc}") from exc
    return digest.hexdigest()


def shadow_artifact_sha256(path: Path) -> str:
    """Compute the immutable digest format used by shadow cohorts."""

    return _hash_bound_path(Path(path), label="shadow artifact")


@dataclass(frozen=True)
class StoreFileDigest:
    path: str
    exists: bool
    size_bytes: int
    sha256: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "path": self.path,
            "exists": self.exists,
            "size_bytes": self.size_bytes,
            "sha256": self.sha256,
        }


def _hash_store_file(path: Path) -> StoreFileDigest:
    declared = Path(path)
    if declared.is_symlink():
        raise ShadowContractError(
            f"canonical store {declared} must be a regular non-symlink file"
        )
    resolved = _canonical_path(declared, label="canonical store path")
    if not resolved.exists():
        return StoreFileDigest(str(resolved), False, 0, "")
    if resolved.is_symlink() or not resolved.is_file():
        raise ShadowContractError(
            f"canonical store {resolved} must be a regular non-symlink file"
        )
    try:
        size = resolved.stat().st_size
        if size > _MAX_STORE_BYTES:
            raise ShadowContractError(f"canonical store {resolved} exceeds the bounded size")
        digest = hashlib.sha256(resolved.read_bytes()).hexdigest()
    except OSError as exc:
        raise ShadowContractError(f"canonical store {resolved} could not be hashed: {exc}") from exc
    return StoreFileDigest(str(resolved), True, size, digest)


@dataclass(frozen=True)
class CanonicalStoreSnapshot:
    entries: tuple[StoreFileDigest, ...]
    approved_observation_paths: tuple[str, ...] = ()

    def _by_path(self) -> dict[str, StoreFileDigest]:
        return {entry.path: entry for entry in self.entries}

    def changed_paths(self, other: CanonicalStoreSnapshot) -> tuple[str, ...]:
        current = self._by_path()
        previous = other._by_path()
        return tuple(
            sorted(
                path
                for path in set(current) | set(previous)
                if current.get(path) != previous.get(path)
            )
        )

    def unauthorized_changes(self, other: CanonicalStoreSnapshot) -> tuple[str, ...]:
        approved = set(self.approved_observation_paths) | set(
            other.approved_observation_paths
        )
        return tuple(path for path in self.changed_paths(other) if path not in approved)


class CanonicalStoreHashGuard:
    """Hash exact canonical store bytes around one Rust shadow sample."""

    def __init__(
        self,
        canonical_store_paths: Sequence[Path],
        *,
        approved_observation_paths: Sequence[Path] = (),
    ) -> None:
        declared = tuple(Path(path) for path in canonical_store_paths)
        if any(path.is_symlink() for path in declared):
            raise ShadowContractError(
                "canonical store paths must be regular non-symlink files"
            )
        resolved = tuple(
            _canonical_path(path, label="canonical store path") for path in declared
        )
        if not resolved or len(resolved) != len(set(resolved)):
            raise ShadowContractError("canonical store paths must be non-empty and unique")
        approved_declared = tuple(Path(path) for path in approved_observation_paths)
        if any(path.is_symlink() for path in approved_declared):
            raise ShadowContractError(
                "approved observation paths must be regular non-symlink files"
            )
        approved = tuple(
            _canonical_path(path, label="approved observation path")
            for path in approved_declared
        )
        if len(approved) != len(set(approved)) or any(path not in resolved for path in approved):
            raise ShadowContractError(
                "approved observation paths must be unique canonical stores"
            )
        # Retain the declared lexical paths so a file replaced by a symlink between
        # captures is rejected rather than resolved to an attacker-controlled target.
        self.paths = declared
        self.approved_observation_paths = tuple(str(path) for path in approved)

    def capture(self) -> CanonicalStoreSnapshot:
        return CanonicalStoreSnapshot(
            tuple(_hash_store_file(path) for path in self.paths),
            self.approved_observation_paths,
        )

    @contextmanager
    def sample(self) -> Iterator[CanonicalStoreSnapshot]:
        before = self.capture()
        try:
            yield before
        finally:
            after = self.capture()
            unauthorized = after.unauthorized_changes(before)
            if unauthorized:
                raise ShadowContractError(
                    "Rust shadow sample changed canonical stores: "
                    + ", ".join(unauthorized)
                )


def normalize_execution_receipt(value: ExecutionReceipt | Mapping[str, Any]) -> dict[str, Any]:
    """Return the cross-provider, transient parity projection.

    Provider ids, generated receipt ids, timestamps, and process identities are
    intentionally excluded.  Output bytes and digests remain because they are
    externally observable and are also retained by the canonical receipt.
    """

    payload = (
        value.to_dict(include_output=True)
        if isinstance(value, ExecutionReceipt)
        else dict(value)
    )
    required = (
        "request_id",
        "request_digest",
        "effect",
        "status",
        "exit_code",
        "stdout",
        "stderr",
        "stdout_bytes",
        "stderr_bytes",
        "stdout_sha256",
        "stderr_sha256",
        "effect_started",
        "reconciliation_required",
        "error_code",
        "error_message",
    )
    missing = [key for key in required if key not in payload]
    if missing:
        raise ShadowContractError(
            "execution receipt parity projection is missing: " + ", ".join(missing)
        )
    return {key: payload[key] for key in required}


@dataclass(frozen=True)
class ShadowParity:
    passed: bool
    mismatches: tuple[str, ...]
    python: dict[str, Any]
    rust: dict[str, Any] | None
    python_failure: dict[str, Any] | None = None
    rust_failure: dict[str, Any] | None = None
    python_process_outcome: dict[str, Any] = field(default_factory=dict)
    rust_process_outcome: dict[str, Any] | None = None
    python_projection: dict[str, Any] = field(default_factory=dict)
    rust_projection: dict[str, Any] | None = None


def normalize_structured_failure(
    failure: RustShadowProviderError | Mapping[str, Any] | None,
) -> dict[str, Any] | None:
    if failure is None:
        return None
    if isinstance(failure, RustShadowProviderError):
        payload: Mapping[str, Any] = {
            "code": failure.code,
            "message": failure.message,
            "recoverable": failure.recoverable,
        }
    else:
        payload = failure
    if set(payload) != {"code", "message", "recoverable"}:
        raise ShadowContractError("structured failure projection is invalid")
    if (
        not isinstance(payload["code"], str)
        or not isinstance(payload["message"], str)
        or not isinstance(payload["recoverable"], bool)
    ):
        raise ShadowContractError("structured failure projection is invalid")
    _validate_identity(payload["code"], label="structured failure code")
    if "\0" in payload["message"]:
        raise ShadowContractError("structured failure message is invalid")
    return {
        "code": payload["code"],
        "message": payload["message"],
        "recoverable": payload["recoverable"],
    }


def _process_outcome_projection(receipt: Mapping[str, Any]) -> dict[str, Any]:
    return {
        key: receipt[key]
        for key in (
            "status",
            "exit_code",
            "effect_started",
            "reconciliation_required",
            "error_code",
        )
    }


def _visible_projection(receipt: Mapping[str, Any]) -> dict[str, Any]:
    return {
        key: receipt[key]
        for key in (
            "effect",
            "status",
            "exit_code",
            "stdout",
            "stderr",
            "error_code",
            "error_message",
        )
    }


def compare_execution_receipts(
    python_receipt: ExecutionReceipt | Mapping[str, Any],
    rust_receipt: ExecutionReceipt | Mapping[str, Any],
    *,
    python_failure: RustShadowProviderError | Mapping[str, Any] | None = None,
    rust_failure: RustShadowProviderError | Mapping[str, Any] | None = None,
) -> ShadowParity:
    python_projection = normalize_execution_receipt(python_receipt)
    rust_projection = normalize_execution_receipt(rust_receipt)
    python_failure_projection = normalize_structured_failure(python_failure)
    rust_failure_projection = normalize_structured_failure(rust_failure)
    python_process_outcome = _process_outcome_projection(python_projection)
    rust_process_outcome = _process_outcome_projection(rust_projection)
    python_visible_projection = _visible_projection(python_projection)
    rust_visible_projection = _visible_projection(rust_projection)
    mismatches = tuple(
        f"{key}: Python={python_projection[key]!r}, Rust={rust_projection[key]!r}"
        for key in python_projection
        if python_projection[key] != rust_projection[key]
    )
    if python_failure_projection != rust_failure_projection:
        mismatches += (
            (
                f"structured-failure: Python={python_failure_projection!r}, "
                f"Rust={rust_failure_projection!r}"
            ),
        )
    if python_process_outcome != rust_process_outcome:
        mismatches += (
            (
                f"process-outcome: Python={python_process_outcome!r}, "
                f"Rust={rust_process_outcome!r}"
            ),
        )
    if python_visible_projection != rust_visible_projection:
        mismatches += (
            (
                f"visible-projection: Python={python_visible_projection!r}, "
                f"Rust={rust_visible_projection!r}"
            ),
        )
    return ShadowParity(
        passed=not mismatches,
        mismatches=mismatches,
        python=python_projection,
        rust=rust_projection,
        python_failure=python_failure_projection,
        rust_failure=rust_failure_projection,
        python_process_outcome=python_process_outcome,
        rust_process_outcome=rust_process_outcome,
        python_projection=python_visible_projection,
        rust_projection=rust_visible_projection,
    )


@dataclass(frozen=True)
class ShadowCohortDefinition:
    cohort_id: str
    fixture_id: str
    fixture_root: str
    fixture_sha256: str
    source_sha256: str
    artifact_sha256: str
    required_stages: tuple[str, ...]
    source_root: str
    artifact_path: str
    evidence_kind: str = "production-equivalent"
    schema_version: int = SHADOW_SCHEMA_VERSION

    def __post_init__(self) -> None:
        if self.schema_version != SHADOW_SCHEMA_VERSION:
            raise ShadowContractError("unsupported shadow cohort schema")
        _validate_identity(self.cohort_id, label="shadow cohort id")
        _validate_identity(self.fixture_id, label="shadow fixture id")
        _canonical_path(Path(self.fixture_root), label="shadow fixture root")
        _canonical_path(Path(self.source_root), label="shadow source root")
        _canonical_path(Path(self.artifact_path), label="shadow artifact path")
        for value, label in (
            (self.fixture_sha256, "shadow fixture"),
            (self.source_sha256, "shadow source"),
            (self.artifact_sha256, "shadow artifact"),
        ):
            _validate_digest(value, label=label)
        if self.evidence_kind != "production-equivalent" or self.evidence_kind in _FORBIDDEN_EVIDENCE_KINDS:
            raise ShadowContractError(
                "shadow cohorts must use production-equivalent fixtures and stages"
            )
        if (
            not self.required_stages
            or len(self.required_stages) != len(set(self.required_stages))
            or len(self.required_stages) > len(_PRODUCTION_STAGES)
        ):
            raise ShadowContractError("shadow cohort stages must be non-empty and unique")
        if any(
            not isinstance(stage, str)
            or not stage.strip()
            or stage in _FORBIDDEN_EVIDENCE_KINDS
            or stage not in _PRODUCTION_STAGES
            for stage in self.required_stages
        ):
            raise ShadowContractError("shadow cohort stages must be production-equivalent")

    def verify_artifacts(self) -> None:
        for value, path, label in (
            (self.fixture_sha256, self.fixture_root, "shadow fixture"),
            (self.source_sha256, self.source_root, "shadow source"),
            (self.artifact_sha256, self.artifact_path, "shadow artifact"),
        ):
            actual = _hash_bound_path(Path(path), label=label)
            if actual != value:
                raise ShadowContractError(
                    f"{label} digest does not match the immutable cohort identity"
                )


@dataclass(frozen=True)
class ShadowSampleMetadata:
    sample_id: str
    cohort_id: str
    fixture_id: str
    fixture_sha256: str
    source_sha256: str
    artifact_sha256: str
    fixture_root: str
    stage: str

    def __post_init__(self) -> None:
        for value, label in (
            (self.sample_id, "shadow sample id"),
            (self.cohort_id, "shadow cohort id"),
            (self.fixture_id, "shadow fixture id"),
            (self.stage, "shadow stage"),
        ):
            _validate_identity(value, label=label)
        for value, label in (
            (self.fixture_sha256, "shadow fixture"),
            (self.source_sha256, "shadow source"),
            (self.artifact_sha256, "shadow artifact"),
        ):
            _validate_digest(value, label=label)
        _canonical_path(Path(self.fixture_root), label="shadow fixture root")
        if self.stage not in _PRODUCTION_STAGES:
            raise ShadowContractError("shadow sample stage is not production-equivalent")


@dataclass(frozen=True)
class ShadowStageMark:
    sample_id: str
    cohort_id: str
    fixture_id: str
    stage: str
    boundary: str
    outcome: str

    def __post_init__(self) -> None:
        for value, label in (
            (self.sample_id, "shadow stage sample id"),
            (self.cohort_id, "shadow stage cohort id"),
            (self.fixture_id, "shadow stage fixture id"),
            (self.stage, "shadow stage"),
            (self.boundary, "shadow stage boundary"),
            (self.outcome, "shadow stage outcome"),
        ):
            _validate_identity(value, label=label)
        if self.stage not in _PRODUCTION_STAGES:
            raise ShadowContractError("shadow stage is not production-equivalent")
        if self.boundary not in {"start", "end"}:
            raise ShadowContractError("shadow stage boundary is invalid")
        if self.outcome not in {"pass", "fail"}:
            raise ShadowContractError("shadow stage outcome is invalid")


@dataclass(frozen=True)
class ShadowSampleResult:
    metadata: ShadowSampleMetadata
    python_receipt: dict[str, Any]
    rust_receipt: dict[str, Any] | None
    parity: ShadowParity
    changed_store_paths: tuple[str, ...]
    store_unchanged: bool
    failure_codes: tuple[str, ...]
    rust_failure: dict[str, Any] | None = None
    eligibility: RustEligibilityDecision | None = None

    @property
    def eligible(self) -> bool:
        return bool(self.eligibility and self.eligibility.eligible)


class ShadowSampleRunner:
    """Run one explicitly fixture-bound Rust shadow sample."""

    def __init__(
        self,
        rust_provider: Any,
        cohort: ShadowCohortDefinition,
        *,
        canonical_store_paths: Sequence[Path],
        approved_observation_paths: Sequence[Path] = (),
        eligibility_store: RustEligibilityStore | None = None,
    ) -> None:
        self.rust_provider = rust_provider
        self.cohort = cohort
        if eligibility_store is None:
            raise ShadowContractError(
                "Rust shadow samples require a persistent eligibility store"
            )
        self.eligibility_store = eligibility_store
        self.guard = CanonicalStoreHashGuard(
            canonical_store_paths,
            approved_observation_paths=approved_observation_paths,
        )

    def _validate_sample(
        self, request: ExecutionRequest, metadata: ShadowSampleMetadata
    ) -> None:
        if metadata.cohort_id != self.cohort.cohort_id:
            raise ShadowContractError("shadow sample cohort identity does not match")
        if metadata.fixture_id != self.cohort.fixture_id:
            raise ShadowContractError("shadow sample fixture identity does not match")
        if metadata.fixture_root != self.cohort.fixture_root:
            raise ShadowContractError("shadow sample fixture root does not match")
        if metadata.fixture_sha256 != self.cohort.fixture_sha256:
            raise ShadowContractError("shadow sample fixture digest does not match")
        if metadata.source_sha256 != self.cohort.source_sha256:
            raise ShadowContractError("shadow sample source digest does not match")
        if metadata.artifact_sha256 != self.cohort.artifact_sha256:
            raise ShadowContractError("shadow sample artifact digest does not match")
        if metadata.stage not in self.cohort.required_stages:
            raise ShadowContractError("shadow sample stage is not in the cohort contract")
        root = _canonical_path(Path(self.cohort.fixture_root), label="shadow fixture root")
        worktree = _canonical_path(
            Path(request.working_directory), label="shadow request working directory"
        )
        if not worktree.is_relative_to(root):
            raise ShadowContractError(
                "shadow request working directory escapes the fixture root"
            )

    def _validate_stage_marks(
        self,
        metadata: ShadowSampleMetadata,
        stage_marks: Sequence[ShadowStageMark],
    ) -> bool:
        if len(stage_marks) > _MAX_STAGE_MARKS:
            raise ShadowContractError("shadow stage measurements are too numerous")
        expected = set(self.cohort.required_stages)
        observed: dict[tuple[str, str], ShadowStageMark] = {}
        for mark in stage_marks:
            if (
                mark.sample_id != metadata.sample_id
                or mark.cohort_id != metadata.cohort_id
                or mark.fixture_id != metadata.fixture_id
            ):
                raise ShadowContractError("shadow stage measurement identity does not match")
            if mark.stage not in expected:
                raise ShadowContractError("shadow stage measurement is outside the cohort")
            key = (mark.stage, mark.boundary)
            if key in observed:
                raise ShadowContractError("shadow stage measurements are duplicated")
            observed[key] = mark
        for stage in expected:
            if (
                (stage, "start") not in observed
                or (stage, "end") not in observed
                or observed[(stage, "start")].outcome != "pass"
                or observed[(stage, "end")].outcome != "pass"
            ):
                return False
        return True

    def run(
        self,
        request: ExecutionRequest,
        python_receipt: ExecutionReceipt,
        metadata: ShadowSampleMetadata,
        *,
        stage_marks: Sequence[ShadowStageMark] = (),
        python_failure: RustShadowProviderError | Mapping[str, Any] | None = None,
    ) -> ShadowSampleResult:
        request.validate()
        if not isinstance(python_receipt, ExecutionReceipt):
            raise ShadowContractError("Python shadow baseline must be a typed receipt")
        if (
            python_receipt.provider != "python"
            or python_receipt.request_id != request.request_id
            or python_receipt.request_digest != request.request_digest
            or python_receipt.effect != request.effect
        ):
            raise ShadowContractError(
                "Python shadow baseline is not the canonical receipt for the request"
            )
        self._validate_sample(request, metadata)
        self.cohort.verify_artifacts()
        provider_command = getattr(self.rust_provider, "command", ())
        if provider_command:
            provider_artifact = _canonical_path(
                Path(provider_command[0]), label="Rust shadow provider artifact"
            )
            if str(provider_artifact) != self.cohort.artifact_path:
                raise ShadowContractError(
                    "Rust shadow provider command is not bound to the cohort artifact"
                )
        stages_complete = self._validate_stage_marks(metadata, stage_marks)
        before = self.guard.capture()
        rust_receipt: ExecutionReceipt | None = None
        rust_failure: RustShadowProviderError | None = None
        failure_codes: list[str] = []
        snapshot_failed = False
        try:
            rust_receipt = self.rust_provider.execute(request)
        except RustShadowProviderError as exc:
            rust_failure = exc
            failure_codes.append(exc.code)
        except Exception as exc:  # noqa: BLE001 - provider crashes become typed shadow uncertainty
            failure_codes.append("rust-provider-crash")
            rust_receipt = replace(
                ExecutionReceipt.unknown(
                    request,
                    error_message=f"Rust shadow provider failed: {exc}",
                ),
                provider="rust-shadow",
            )
        if rust_receipt is not None and rust_receipt.status == "outcome-unknown":
            failure_codes.append("rust-provider-crash")
        try:
            after = self.guard.capture()
        except ShadowContractError:
            failure_codes.append("canonical-store-snapshot-failure")
            snapshot_failed = True
            after = before
        changed = after.changed_paths(before)
        unauthorized = after.unauthorized_changes(before)
        if unauthorized:
            failure_codes.insert(0, "canonical-store-mutated")
        if rust_receipt is None:
            parity = ShadowParity(
                passed=False,
                mismatches=("Rust shadow provider returned no receipt",),
                python=normalize_execution_receipt(python_receipt),
                rust=None,
                python_failure=normalize_structured_failure(python_failure),
                rust_failure=normalize_structured_failure(rust_failure),
            )
        else:
            parity = compare_execution_receipts(
                python_receipt,
                rust_receipt,
                python_failure=python_failure,
                rust_failure=rust_failure,
            )
            if not parity.passed:
                failure_codes.append("receipt-parity-failure")
        if not stages_complete:
            failure_codes.append("stage-measurements")
        eligibility = None
        evidence = RustEligibilityEvidence(
            sample_id=metadata.sample_id,
            cohort_id=metadata.cohort_id,
            contract_parity_passed=parity.passed,
            store_integrity_passed=not unauthorized and not snapshot_failed,
            crash_cut_passed="rust-provider-crash" not in failure_codes,
            state_version_passed="state-version-failure" not in failure_codes,
            packaging_passed=False,
            release_gate_passed=False,
            production_equivalent=True,
            stages_complete=stages_complete,
            stages=self.cohort.required_stages,
            failure_reasons=tuple(failure_codes),
        )
        eligibility = self.eligibility_store.record(evidence)
        return ShadowSampleResult(
            metadata=metadata,
            python_receipt=normalize_execution_receipt(python_receipt),
            rust_receipt=(
                normalize_execution_receipt(rust_receipt)
                if rust_receipt is not None
                else None
            ),
            parity=parity,
            changed_store_paths=changed,
            store_unchanged=not unauthorized and not snapshot_failed,
            failure_codes=tuple(failure_codes),
            rust_failure=normalize_structured_failure(rust_failure),
            eligibility=eligibility,
        )


class RustShadowProvider:
    """Adapter for the Rust JSONL provider; never a canonical coordinator."""

    def __init__(
        self,
        command: Sequence[str],
        *,
        timeout_seconds: float = 3_600.0,
        environment: Mapping[str, str] | None = None,
    ) -> None:
        if not command or any(not isinstance(part, str) or not part for part in command):
            raise ShadowContractError("Rust shadow provider command is invalid")
        if (
            not isinstance(timeout_seconds, (int, float))
            or isinstance(timeout_seconds, bool)
            or not math.isfinite(float(timeout_seconds))
            or timeout_seconds <= 0
            or timeout_seconds > _MAX_PROVIDER_TIMEOUT_SECONDS
        ):
            raise ShadowContractError("Rust shadow provider timeout is invalid")
        self.command = tuple(command)
        self.timeout_seconds = float(timeout_seconds)
        self.environment = dict(environment or {})
        if any(
            not isinstance(key, str)
            or key not in _PROVIDER_ENVIRONMENT_ALLOWLIST
            | _PROVIDER_ENVIRONMENT_OVERRIDE_ALLOWLIST
            or not isinstance(value, str)
            or "\0" in value
            or len(value.encode("utf-8")) > _MAX_PROVIDER_OUTPUT_BYTES
            for key, value in self.environment.items()
        ):
            raise ShadowContractError("Rust shadow provider environment is invalid")
        self.provider_id = "rust-shadow"

    def execute(
        self,
        request: ExecutionRequest,
        *,
        cancel_after_seconds: float | None = None,
        provider_process_started: Callable[[subprocess.Popen[bytes]], None]
        | None = None,
        effect_process_started: Callable[[int, str], None] | None = None,
        control_poll_callback: Callable[[], None] | None = None,
    ) -> ExecutionReceipt:
        request.validate()
        if cancel_after_seconds is not None and (
            not isinstance(cancel_after_seconds, (int, float))
            or isinstance(cancel_after_seconds, bool)
            or not math.isfinite(float(cancel_after_seconds))
            or cancel_after_seconds <= 0
            or cancel_after_seconds > _MAX_PROVIDER_TIMEOUT_SECONDS
        ):
            raise ShadowContractError(
                "Rust shadow provider cancellation control is invalid"
            )
        envelope: dict[str, Any] = {
            "request": request.to_dict(include_input=True)
        }
        if cancel_after_seconds is not None:
            envelope["control"] = {
                "cancel_after_milliseconds": max(
                    1, math.ceil(float(cancel_after_seconds) * 1_000)
                )
            }
        stream_events = (
            effect_process_started is not None or control_poll_callback is not None
        )
        payload = json.dumps(
            envelope,
            ensure_ascii=True,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        source = os.environ.copy()
        env = {
            key: value
            for key, value in source.items()
            if key in _PROVIDER_ENVIRONMENT_ALLOWLIST
        }
        env.update(
            {
                key: value
                for key, value in self.environment.items()
                if key in _PROVIDER_ENVIRONMENT_ALLOWLIST
                | _PROVIDER_ENVIRONMENT_OVERRIDE_ALLOWLIST
            }
        )
        env.setdefault("PATH", os.defpath)
        env["HOME"] = "/tmp"
        env["TMPDIR"] = "/tmp"
        env["PYTHONDONTWRITEBYTECODE"] = "1"
        if stream_events:
            # The current provider opts into child-binding events through this
            # internal transport flag.  The immediately previous provider
            # ignores it and retains its one-response JSON contract.
            env["ALFREDO_EXECUTION_STREAM_EVENTS"] = "1"
        stdout_buffer: list[bytes] = []
        stderr_buffer: list[bytes] = []
        output_lock = threading.Lock()
        output_size = 0
        output_overflow = threading.Event()
        protocol_failed = threading.Event()
        protocol_failures: list[BaseException] = []
        effect_callback_failures: list[BaseException] = []
        effect_binding: dict[str, Any] = {}

        def drain(stream: Any, target: list[bytes]) -> None:
            nonlocal output_size
            try:
                while True:
                    chunk = stream.read(64 * 1024)
                    if not chunk:
                        return
                    with output_lock:
                        remaining = max(0, _MAX_PROVIDER_OUTPUT_BYTES - output_size)
                        if remaining:
                            target.append(chunk[:remaining])
                        output_size += len(chunk)
                        if output_size > _MAX_PROVIDER_OUTPUT_BYTES:
                            output_overflow.set()
                            return
            except OSError:
                return
            finally:
                try:
                    stream.close()
                except OSError:
                    pass

        def drain_provider_stdout(stream: Any) -> None:
            nonlocal output_size
            try:
                while True:
                    line = stream.readline(_MAX_PROVIDER_OUTPUT_BYTES + 1)
                    if not line:
                        return
                    with output_lock:
                        output_size += len(line)
                        if output_size > _MAX_PROVIDER_OUTPUT_BYTES:
                            output_overflow.set()
                            return
                    try:
                        message = json.loads(line.decode("utf-8"))
                    except (UnicodeDecodeError, json.JSONDecodeError):
                        stdout_buffer.append(line)
                        continue
                    if not isinstance(message, Mapping) or message.get("event") != "process-started":
                        stdout_buffer.append(line)
                        continue
                    if set(message) != {"event", "process_pid", "process_identity"}:
                        raise RustShadowProviderError(
                            "rust-provider-contract-failure",
                            "Rust provider process-started event fields are invalid.",
                        )
                    process_pid = message.get("process_pid")
                    process_identity = message.get("process_identity")
                    if (
                        effect_binding
                        or not isinstance(process_pid, int)
                        or isinstance(process_pid, bool)
                        or process_pid <= 0
                        or not isinstance(process_identity, str)
                        or not process_identity
                        or "\0" in process_identity
                    ):
                        raise RustShadowProviderError(
                            "rust-provider-contract-failure",
                            "Rust provider process-started event is invalid or duplicated.",
                        )
                    effect_binding.update(
                        {"pid": process_pid, "identity": process_identity}
                    )
                    if effect_process_started is not None:
                        try:
                            effect_process_started(process_pid, process_identity)
                        except BaseException as exc:
                            # The coordinator has already observed the effect
                            # boundary. Ask Rust to clean it up, drain the typed
                            # receipt, then re-raise the original control/store
                            # failure so Python decides cancelled vs uncertain.
                            effect_callback_failures.append(exc)
                            if os.name == "posix" and process is not None:
                                try:
                                    os.kill(process.pid, signal.SIGUSR1)
                                except ProcessLookupError:
                                    pass
            except BaseException as exc:
                protocol_failures.append(exc)
                protocol_failed.set()
            finally:
                try:
                    stream.close()
                except OSError:
                    pass

        process: subprocess.Popen[bytes] | None = None
        reader_threads: tuple[threading.Thread, threading.Thread] | None = None
        writer_thread: threading.Thread | None = None
        timed_out = False
        cancellation_requested = False
        try:
            process = subprocess.Popen(
                self.command,
                cwd=request.working_directory,
                env=env,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=os.name == "posix",
            )
            if provider_process_started is not None:
                provider_process_started(process)
            assert process.stdout is not None and process.stderr is not None
            reader_threads = (
                threading.Thread(target=drain_provider_stdout, args=(process.stdout,), daemon=True),
                threading.Thread(target=drain, args=(process.stderr, stderr_buffer), daemon=True),
            )
            for thread in reader_threads:
                thread.start()
            if process.stdin is not None:
                def write_request() -> None:
                    try:
                        process.stdin.write(payload)
                    except (BrokenPipeError, OSError):
                        pass
                    finally:
                        try:
                            process.stdin.close()
                        except OSError:
                            pass

                writer_thread = threading.Thread(target=write_request, daemon=True)
                writer_thread.start()
            deadline = time.monotonic() + min(
                self.timeout_seconds,
                request.limits.timeout_seconds
                + max(5.0, request.limits.descendant_grace_seconds + 5.0),
            )
            while process.poll() is None:
                if output_overflow.is_set() or protocol_failed.is_set():
                    break
                if control_poll_callback is not None and not cancellation_requested:
                    try:
                        control_poll_callback()
                    except BaseException:
                        cancellation_requested = True
                        if os.name == "posix":
                            try:
                                os.kill(process.pid, signal.SIGUSR1)
                            except ProcessLookupError:
                                pass
                if time.monotonic() >= deadline:
                    timed_out = True
                    break
                time.sleep(0.005)
            if process.poll() is None and (
                timed_out or output_overflow.is_set() or protocol_failed.is_set()
            ):
                self._terminate(process)
            returncode = process.wait(timeout=5.0)
        except subprocess.TimeoutExpired:
            if process is not None:
                self._terminate(process)
                returncode = process.wait(timeout=5.0)
            else:
                returncode = None
        except OSError as exc:
            return replace(
                ExecutionReceipt.start_failed(
                    request,
                    exit_code=127,
                    error_message=f"Rust shadow provider could not start: {exc}",
                ),
                provider=self.provider_id,
            )
        finally:
            if reader_threads is not None:
                for thread in reader_threads:
                    thread.join(timeout=2.0)
            if writer_thread is not None:
                writer_thread.join(timeout=2.0)
        if timed_out:
            return replace(
                ExecutionReceipt.unknown(
                    request,
                    error_message="Rust shadow provider timed out before a receipt was observed.",
                    process_pid=effect_binding.get("pid"),
                    process_identity=effect_binding.get("identity", ""),
                ),
                provider=self.provider_id,
            )
        if protocol_failures:
            raise protocol_failures[0]
        if output_overflow.is_set():
            raise RustShadowProviderError(
                "rust-provider-output-limit",
                "Rust shadow provider response exceeded its bounded output.",
            )
        stdout = b"".join(stdout_buffer)
        if returncode != 0:
            return replace(
                ExecutionReceipt.unknown(
                    request,
                    error_message="Rust shadow provider exited before a trustworthy receipt was observed.",
                    process_pid=effect_binding.get("pid"),
                    process_identity=effect_binding.get("identity", ""),
                ),
                provider=self.provider_id,
            )
        try:
            response = json.loads(stdout.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            return replace(
                ExecutionReceipt.unknown(
                    request,
                    error_message=(
                        "Rust shadow provider crashed or returned invalid JSON: "
                        f"{exc}"
                    ),
                    process_pid=effect_binding.get("pid"),
                    process_identity=effect_binding.get("identity", ""),
                ),
                provider=self.provider_id,
            )
        if not isinstance(response, Mapping):
            raise RustShadowProviderError(
                "rust-provider-contract-failure",
                "Rust shadow provider response must be an object.",
            )
        if response.get("ok") is not True:
            failure = response.get("failure")
            if not isinstance(failure, Mapping):
                raise RustShadowProviderError(
                    "rust-provider-contract-failure",
                    "Rust shadow provider returned an invalid structured failure.",
                )
            if set(failure) != {"code", "message", "recoverable"}:
                raise RustShadowProviderError(
                    "rust-provider-contract-failure",
                    "Rust shadow provider returned an invalid structured failure.",
                )
            code = failure.get("code")
            message = failure.get("message")
            recoverable = failure.get("recoverable")
            if (
                not isinstance(code, str)
                or not isinstance(message, str)
                or not isinstance(recoverable, bool)
            ):
                raise RustShadowProviderError(
                    "rust-provider-contract-failure",
                    "Rust shadow provider returned an invalid structured failure.",
                )
            raise RustShadowProviderError(code, message, recoverable)
        raw_receipt = response.get("receipt")
        if not isinstance(raw_receipt, Mapping):
            raise RustShadowProviderError(
                "rust-provider-contract-failure",
                "Rust shadow provider did not return a typed receipt.",
            )
        try:
            receipt = ExecutionReceipt.from_dict(raw_receipt)
        except Exception as exc:
            raise RustShadowProviderError(
                "rust-provider-contract-failure",
                f"Rust shadow provider returned an invalid receipt: {exc}",
            ) from exc
        if receipt.request_id != request.request_id or receipt.request_digest != request.request_digest:
            raise RustShadowProviderError(
                "rust-provider-contract-failure",
                "Rust shadow receipt does not match the request identity.",
            )
        if effect_callback_failures:
            raise effect_callback_failures[0]
        return replace(receipt, provider=self.provider_id)

    @staticmethod
    def _terminate(process: subprocess.Popen[bytes]) -> None:
        if os.name == "posix":
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
            time.sleep(0.05)
            if process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
        elif process.poll() is None:
            process.kill()


@dataclass(frozen=True)
class RustReleaseGateEvidence:
    """Recomputed proof that a packaged Rust artifact passed its release gate."""

    provider_path: str
    provider_sha256: str
    package_manifest_path: str
    package_manifest_sha256: str
    schema_version: int = SHADOW_SCHEMA_VERSION

    def __post_init__(self) -> None:
        if self.schema_version != SHADOW_SCHEMA_VERSION:
            raise ShadowContractError("unsupported Rust release evidence schema")
        for value, label in (
            (self.provider_sha256, "Rust provider artifact"),
            (self.package_manifest_sha256, "Rust package manifest"),
        ):
            _validate_digest(value, label=label)
        _canonical_path(Path(self.provider_path), label="Rust provider artifact")
        _canonical_path(
            Path(self.package_manifest_path), label="Rust package manifest"
        )

    @classmethod
    def from_verified_artifacts(
        cls,
        *,
        provider_path: Path,
        package_manifest_path: Path,
    ) -> RustReleaseGateEvidence:
        provider = _canonical_path(provider_path, label="Rust provider artifact")
        manifest = _canonical_path(
            package_manifest_path, label="Rust package manifest"
        )
        provider_digest = _hash_regular_file(provider, label="Rust provider artifact")
        manifest_digest = _hash_regular_file(manifest, label="Rust package manifest")
        try:
            payload = json.loads(manifest.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as exc:
            raise ShadowContractError(f"Rust package manifest is invalid: {exc}") from exc
        expected_manifest_fields = {
            "schema_version",
            "status",
            "verification_kind",
            "publishable",
            "package_version",
            "install_spec",
            "publish_order",
            "packages",
            "shadow_execution_provider",
        }
        if not isinstance(payload, Mapping) or set(payload) != expected_manifest_fields:
            raise ShadowContractError("Rust release manifest fields are invalid")
        version = payload["package_version"]
        packages = payload["packages"]
        publish_order = payload["publish_order"]
        if (
            payload["schema_version"] != SHADOW_SCHEMA_VERSION
            or payload["status"] != "verified"
            or payload["verification_kind"] != "production-appimage"
            or payload["publishable"] is not True
            or not isinstance(version, str)
            or not version
            or not isinstance(packages, list)
            or len(packages) != 2
            or not isinstance(publish_order, list)
            or len(publish_order) != 2
        ):
            raise ShadowContractError("Rust release manifest is not production-verified")
        package_fields = {
            "role",
            "name",
            "version",
            "filename",
            "bytes",
            "sha256",
            "integrity",
        }
        if any(
            not isinstance(entry, Mapping) or set(entry) != package_fields
            for entry in packages
        ):
            raise ShadowContractError("Rust release package fields are invalid")
        platform, meta = packages
        if (
            platform["role"] != "platform"
            or meta["role"] != "meta"
            or platform["name"] != "alfredo-agent-linux-x64-gnu"
            or meta["name"] != "alfredo-agent"
            or publish_order != [platform["name"], meta["name"]]
            or payload["install_spec"] != f'{meta["name"]}@{version}'
        ):
            raise ShadowContractError("Rust release package order is invalid")
        manifest_root = manifest.parent.resolve(strict=True)
        checked_tarballs: dict[str, Path] = {}
        for entry in packages:
            name = entry["name"]
            filename = entry["filename"]
            declared_bytes = entry["bytes"]
            declared_sha256 = entry["sha256"]
            declared_integrity = entry["integrity"]
            if (
                not isinstance(name, str)
                or not name
                or entry["version"] != version
                or filename != f"{name}-{version}.tgz"
                or not isinstance(declared_bytes, int)
                or isinstance(declared_bytes, bool)
                or declared_bytes <= 0
                or not isinstance(declared_integrity, str)
                or not declared_integrity.startswith("sha512-")
            ):
                raise ShadowContractError("Rust release package metadata is invalid")
            _validate_digest(declared_sha256, label="Rust release package")
            tarball = manifest_root / filename
            if tarball.parent != manifest_root or tarball.is_symlink():
                raise ShadowContractError("Rust release package escapes its verified root")
            try:
                resolved_tarball = tarball.resolve(strict=True)
                if (
                    resolved_tarball.parent != manifest_root
                    or not resolved_tarball.is_file()
                    or resolved_tarball.stat().st_size != declared_bytes
                ):
                    raise ShadowContractError("Rust release package artifact is invalid")
            except OSError as exc:
                raise ShadowContractError(
                    f"Rust release package could not be inspected: {exc}"
                ) from exc
            if _hash_regular_file(
                resolved_tarball, label="Rust release package"
            ) != declared_sha256:
                raise ShadowContractError("Rust release package digest changed")
            if _sha512_integrity(
                resolved_tarball, label="Rust release package"
            ) != declared_integrity:
                raise ShadowContractError(
                    "Rust release package SHA-512 integrity changed"
                )
            checked_tarballs[entry["role"]] = resolved_tarball

        shadow = payload["shadow_execution_provider"]
        expected_shadow_fields = {
            "package",
            "path",
            "sha256",
            "contract",
            "verification",
            "request_sha256",
            "store_unchanged",
        }
        if not isinstance(shadow, Mapping) or set(shadow) != expected_shadow_fields:
            raise ShadowContractError("Rust release provider evidence fields are invalid")
        if (
            shadow["package"] != platform["name"]
            or shadow["path"] != "bin/alfredo-execution-provider"
            or shadow["sha256"] != provider_digest
            or shadow["contract"] != "python-rust-production-parity"
            or shadow["verification"] != "installed-package"
            or shadow["store_unchanged"] is not True
        ):
            raise ShadowContractError("Rust release manifest does not verify the provider")
        _validate_digest(shadow["request_sha256"], label="Rust release parity request")

        platform_tarball = checked_tarballs["platform"]
        meta_tarball = checked_tarballs["meta"]
        provider_member = f'package/{shadow["path"]}'
        executable_member = "package/bin/alfredo-desktop.AppImage"
        platform_manifest_member = "package/package.json"
        try:
            with tarfile.open(platform_tarball, mode="r:gz") as archive:
                provider_members = [
                    member for member in archive.getmembers() if member.name == provider_member
                ]
                desktop_members = [
                    member
                    for member in archive.getmembers()
                    if member.name == "package/desktop.json"
                ]
                executable_members = [
                    member
                    for member in archive.getmembers()
                    if member.name == executable_member
                ]
                platform_manifest_members = [
                    member
                    for member in archive.getmembers()
                    if member.name == platform_manifest_member
                ]
                if (
                    len(provider_members) != 1
                    or not provider_members[0].isfile()
                    or provider_members[0].size > _MAX_SHADOW_TREE_BYTES
                    or len(desktop_members) != 1
                    or not desktop_members[0].isfile()
                    or desktop_members[0].size > _MAX_RELEASE_METADATA_BYTES
                    or len(executable_members) != 1
                    or not executable_members[0].isfile()
                    or executable_members[0].size > _MAX_SHADOW_TREE_BYTES
                    or len(platform_manifest_members) != 1
                    or not platform_manifest_members[0].isfile()
                    or platform_manifest_members[0].size > _MAX_RELEASE_METADATA_BYTES
                ):
                    raise ShadowContractError(
                        "Rust release package provider members are invalid"
                    )
                provider_file = archive.extractfile(provider_members[0])
                desktop_file = archive.extractfile(desktop_members[0])
                executable_file = archive.extractfile(executable_members[0])
                platform_manifest_file = archive.extractfile(
                    platform_manifest_members[0]
                )
                if (
                    provider_file is None
                    or desktop_file is None
                    or executable_file is None
                    or platform_manifest_file is None
                ):
                    raise ShadowContractError("Rust release package provider is unreadable")
                packaged_provider_digest = hashlib.sha256(provider_file.read()).hexdigest()
                packaged_executable_digest = hashlib.sha256(
                    executable_file.read()
                ).hexdigest()
                desktop_payload = json.loads(desktop_file.read().decode("utf-8"))
                platform_manifest_payload = json.loads(
                    platform_manifest_file.read().decode("utf-8")
                )
            with tarfile.open(meta_tarball, mode="r:gz") as archive:
                meta_manifest_members = [
                    member
                    for member in archive.getmembers()
                    if member.name == "package/package.json"
                ]
                if (
                    len(meta_manifest_members) != 1
                    or not meta_manifest_members[0].isfile()
                    or meta_manifest_members[0].size > _MAX_RELEASE_METADATA_BYTES
                ):
                    raise ShadowContractError(
                        "Rust release meta package manifest is invalid"
                    )
                meta_manifest_file = archive.extractfile(meta_manifest_members[0])
                if meta_manifest_file is None:
                    raise ShadowContractError(
                        "Rust release meta package manifest is unreadable"
                    )
                meta_manifest_payload = json.loads(
                    meta_manifest_file.read().decode("utf-8")
                )
        except (OSError, tarfile.TarError, UnicodeError, json.JSONDecodeError) as exc:
            raise ShadowContractError(f"Rust release package is invalid: {exc}") from exc
        if (
            not isinstance(platform_manifest_payload, Mapping)
            or platform_manifest_payload.get("name") != platform["name"]
            or platform_manifest_payload.get("version") != version
            or platform_manifest_payload.get("os") != ["linux"]
            or platform_manifest_payload.get("cpu") != ["x64"]
            or platform_manifest_payload.get("libc") != ["glibc"]
        ):
            raise ShadowContractError(
                "Rust release platform package identity is invalid"
            )
        if (
            not isinstance(meta_manifest_payload, Mapping)
            or meta_manifest_payload.get("name") != meta["name"]
            or meta_manifest_payload.get("version") != version
            or not isinstance(meta_manifest_payload.get("optionalDependencies"), Mapping)
            or meta_manifest_payload["optionalDependencies"].get(platform["name"])
            != version
        ):
            raise ShadowContractError(
                "Rust release meta package does not require the exact platform version"
            )
        if meta_manifest_payload.get("bin") != {
            "alfredo": "bin/alfredo.js",
            "albert": "bin/alfredo.js",
        }:
            raise ShadowContractError("Rust release meta package CLI identity is invalid")
        expected_desktop_fields = {
            "schema_version",
            "package",
            "version",
            "platform",
            "arch",
            "libc",
            "format",
            "executable",
            "executable_sha256",
            "shadow_provider",
            "shadow_provider_sha256",
        }
        if (
            packaged_provider_digest != provider_digest
            or not isinstance(desktop_payload, Mapping)
            or set(desktop_payload) != expected_desktop_fields
            or desktop_payload.get("schema_version") != SHADOW_SCHEMA_VERSION
            or desktop_payload.get("package") != platform["name"]
            or desktop_payload.get("version") != version
            or desktop_payload.get("platform") != "linux"
            or desktop_payload.get("arch") != "x64"
            or desktop_payload.get("libc") != "glibc"
            or desktop_payload.get("format") != "appimage"
            or desktop_payload.get("executable")
            != "bin/alfredo-desktop.AppImage"
            or desktop_payload.get("executable_sha256")
            != packaged_executable_digest
            or desktop_payload.get("shadow_provider") != shadow["path"]
            or desktop_payload.get("shadow_provider_sha256") != provider_digest
        ):
            raise ShadowContractError(
                "Rust release package does not contain the verified provider"
            )
        return cls(
            provider_path=str(provider),
            provider_sha256=provider_digest,
            package_manifest_path=str(manifest),
            package_manifest_sha256=manifest_digest,
        )

    def verify(self) -> None:
        provider = Path(self.provider_path)
        manifest = Path(self.package_manifest_path)
        actual_provider = _hash_regular_file(provider, label="Rust provider artifact")
        actual_manifest = _hash_regular_file(manifest, label="Rust package manifest")
        if actual_provider != self.provider_sha256 or actual_manifest != self.package_manifest_sha256:
            raise ShadowContractError("Rust release evidence no longer matches its artifacts")
        type(self).from_verified_artifacts(
            provider_path=provider, package_manifest_path=manifest
        )

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": self.schema_version,
            "provider_path": self.provider_path,
            "provider_sha256": self.provider_sha256,
            "package_manifest_path": self.package_manifest_path,
            "package_manifest_sha256": self.package_manifest_sha256,
        }

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any]) -> RustReleaseGateEvidence:
        if set(payload) != {
            "schema_version",
            "provider_path",
            "provider_sha256",
            "package_manifest_path",
            "package_manifest_sha256",
        }:
            raise ShadowContractError("Rust release evidence fields are invalid")
        evidence = cls(
            schema_version=payload["schema_version"],
            provider_path=payload["provider_path"],
            provider_sha256=payload["provider_sha256"],
            package_manifest_path=payload["package_manifest_path"],
            package_manifest_sha256=payload["package_manifest_sha256"],
        )
        evidence.verify()
        return evidence


@dataclass(frozen=True)
class RustEligibilityEvidence:
    sample_id: str
    cohort_id: str
    contract_parity_passed: bool
    store_integrity_passed: bool
    crash_cut_passed: bool
    state_version_passed: bool
    packaging_passed: bool
    release_gate_passed: bool
    production_equivalent: bool
    stages_complete: bool
    stages: tuple[str, ...]
    failure_reasons: tuple[str, ...] = ()
    release_evidence: RustReleaseGateEvidence | None = None

    def __post_init__(self) -> None:
        _validate_identity(self.sample_id, label="Rust eligibility sample id")
        _validate_identity(self.cohort_id, label="Rust eligibility cohort id")
        for field_name in (
            "contract_parity_passed",
            "store_integrity_passed",
            "crash_cut_passed",
            "state_version_passed",
            "packaging_passed",
            "release_gate_passed",
            "production_equivalent",
            "stages_complete",
        ):
            if not isinstance(getattr(self, field_name), bool):
                raise ShadowContractError(f"Rust eligibility {field_name} is invalid")
        if not isinstance(self.stages, tuple) or not self.stages:
            raise ShadowContractError("Rust eligibility stages are invalid")
        for stage in self.stages:
            _validate_identity(stage, label="Rust eligibility stage")
            if stage not in _PRODUCTION_STAGES:
                raise ShadowContractError("Rust eligibility stage is not production-equivalent")
        if len(self.stages) != len(set(self.stages)):
            raise ShadowContractError("Rust eligibility stages must be unique")
        if not isinstance(self.failure_reasons, tuple):
            raise ShadowContractError("Rust eligibility failure reasons are invalid")
        for reason in self.failure_reasons:
            _validate_identity(reason, label="Rust eligibility failure reason")
        if self.release_evidence is not None and not isinstance(
            self.release_evidence, RustReleaseGateEvidence
        ):
            raise ShadowContractError("Rust release evidence is invalid")

    @classmethod
    def all_passed(
        cls,
        *,
        sample_id: str,
        cohort_id: str,
        stages: Sequence[str],
        release_evidence: RustReleaseGateEvidence,
    ) -> RustEligibilityEvidence:
        return cls(
            sample_id=sample_id,
            cohort_id=cohort_id,
            contract_parity_passed=True,
            store_integrity_passed=True,
            crash_cut_passed=True,
            state_version_passed=True,
            packaging_passed=True,
            release_gate_passed=True,
            production_equivalent=True,
            stages_complete=True,
            stages=tuple(stages),
            release_evidence=release_evidence,
        )

    def to_dict(self) -> dict[str, Any]:
        return {
            "sample_id": self.sample_id,
            "cohort_id": self.cohort_id,
            "contract_parity_passed": self.contract_parity_passed,
            "store_integrity_passed": self.store_integrity_passed,
            "crash_cut_passed": self.crash_cut_passed,
            "state_version_passed": self.state_version_passed,
            "packaging_passed": self.packaging_passed,
            "release_gate_passed": self.release_gate_passed,
            "production_equivalent": self.production_equivalent,
            "stages_complete": self.stages_complete,
            "stages": list(self.stages),
            "failure_reasons": list(self.failure_reasons),
            "release_evidence": (
                self.release_evidence.to_dict() if self.release_evidence else None
            ),
        }


@dataclass(frozen=True)
class RustEligibilityDecision:
    schema_version: int
    revision: int
    eligible: bool
    disabled_reason: str
    evidence: RustEligibilityEvidence | None
    updated_at: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": self.schema_version,
            "revision": self.revision,
            "eligible": self.eligible,
            "disabled_reason": self.disabled_reason,
            "evidence": self.evidence.to_dict() if self.evidence else None,
            "updated_at": self.updated_at,
        }


class RustEligibilityStore:
    """Separate app-local shadow eligibility state with fail-closed reload."""

    def __init__(self, path: Path) -> None:
        self.path = _canonical_path(Path(path), label="Rust eligibility path")
        if self.path.name != "rust-eligibility.json":
            raise ShadowContractError(
                "Rust eligibility state must use the app-local rust-eligibility.json name"
            )
        self.lock_path = self.path.with_name(f".{self.path.name}.lock")

    @classmethod
    def from_runtime_root(cls, runtime_root: Path) -> RustEligibilityStore:
        root = _canonical_path(Path(runtime_root), label="Rust runtime root")
        return cls(root / "shadow" / "rust-eligibility.json")

    @contextmanager
    def _lock(self) -> Iterator[None]:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.lock_path.open("a+", encoding="utf-8") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock.fileno(), fcntl.LOCK_UN)

    @staticmethod
    def _empty() -> RustEligibilityDecision:
        return RustEligibilityDecision(
            schema_version=SHADOW_SCHEMA_VERSION,
            revision=0,
            eligible=False,
            disabled_reason="no-shadow-evidence",
            evidence=None,
            updated_at=_utc_now(),
        )

    @staticmethod
    def _decode(payload: Mapping[str, Any]) -> RustEligibilityDecision:
        if set(payload) != {
            "schema_version",
            "revision",
            "eligible",
            "disabled_reason",
            "evidence",
            "updated_at",
        }:
            raise ShadowContractError("Rust eligibility state fields are invalid")
        if payload.get("schema_version") != SHADOW_SCHEMA_VERSION:
            raise ShadowContractError("unsupported Rust eligibility schema")
        revision = payload.get("revision")
        if not isinstance(revision, int) or isinstance(revision, bool) or revision < 0:
            raise ShadowContractError("Rust eligibility revision is invalid")
        eligible = payload.get("eligible")
        if not isinstance(eligible, bool):
            raise ShadowContractError("Rust eligibility flag is invalid")
        reason = payload.get("disabled_reason")
        if not isinstance(reason, str):
            raise ShadowContractError("Rust eligibility disabled reason is invalid")
        raw_evidence = payload.get("evidence")
        evidence = None
        if raw_evidence is not None:
            if not isinstance(raw_evidence, Mapping):
                raise ShadowContractError("Rust eligibility evidence is invalid")
            required_evidence_fields = (
                "sample_id",
                "cohort_id",
                "contract_parity_passed",
                "store_integrity_passed",
                "crash_cut_passed",
                "state_version_passed",
                "packaging_passed",
                "release_gate_passed",
                "production_equivalent",
                "stages_complete",
                "stages",
                "release_evidence",
            )
            allowed_evidence_fields = set(required_evidence_fields) | {
                "failure_reasons"
            }
            if set(raw_evidence) != allowed_evidence_fields:
                raise ShadowContractError("Rust eligibility evidence fields are invalid")
            missing = [
                field for field in required_evidence_fields if field not in raw_evidence
            ]
            if missing:
                raise ShadowContractError(
                    "Rust eligibility evidence is missing: " + ", ".join(missing)
                )
            failure_reasons = raw_evidence.get("failure_reasons", [])
            if not isinstance(failure_reasons, list):
                raise ShadowContractError("Rust eligibility failure reasons are invalid")
            raw_stages = raw_evidence["stages"]
            if not isinstance(raw_stages, list):
                raise ShadowContractError("Rust eligibility stages are invalid")
            release_evidence = raw_evidence["release_evidence"]
            if release_evidence is not None and not isinstance(release_evidence, Mapping):
                raise ShadowContractError("Rust release evidence is invalid")
            evidence = RustEligibilityEvidence(
                sample_id=raw_evidence["sample_id"],
                cohort_id=raw_evidence["cohort_id"],
                contract_parity_passed=raw_evidence["contract_parity_passed"],
                store_integrity_passed=raw_evidence["store_integrity_passed"],
                crash_cut_passed=raw_evidence["crash_cut_passed"],
                state_version_passed=raw_evidence["state_version_passed"],
                packaging_passed=raw_evidence["packaging_passed"],
                release_gate_passed=raw_evidence["release_gate_passed"],
                production_equivalent=raw_evidence["production_equivalent"],
                stages_complete=raw_evidence["stages_complete"],
                stages=tuple(raw_stages),
                failure_reasons=tuple(failure_reasons),
                release_evidence=(
                    RustReleaseGateEvidence.from_dict(release_evidence)
                    if release_evidence is not None
                    else None
                ),
            )
        if eligible and (evidence is None or reason or evidence.failure_reasons):
            raise ShadowContractError("eligible Rust state has failing evidence")
        if eligible and evidence is not None and not all(
            (
                evidence.contract_parity_passed,
                evidence.store_integrity_passed,
                evidence.crash_cut_passed,
                evidence.state_version_passed,
                evidence.packaging_passed,
                evidence.release_gate_passed,
                evidence.production_equivalent,
                evidence.stages_complete,
            )
        ):
            raise ShadowContractError("eligible Rust state has a failed gate")
        updated_at = payload.get("updated_at")
        if not isinstance(updated_at, str) or not updated_at.endswith("Z"):
            raise ShadowContractError("Rust eligibility timestamp is invalid")
        try:
            datetime.fromisoformat(updated_at.removesuffix("Z") + "+00:00")
        except ValueError as exc:
            raise ShadowContractError("Rust eligibility timestamp is invalid") from exc
        if eligible:
            if evidence is None or evidence.release_evidence is None:
                raise ShadowContractError("eligible Rust state lacks release evidence")
            evidence.release_evidence.verify()
        return RustEligibilityDecision(
            schema_version=SHADOW_SCHEMA_VERSION,
            revision=revision,
            eligible=eligible,
            disabled_reason=reason,
            evidence=evidence,
            updated_at=updated_at,
        )

    def load(self) -> RustEligibilityDecision:
        with self._lock():
            return self._read_unlocked()

    def _read_unlocked(self) -> RustEligibilityDecision:
        if not self.path.exists():
            return self._empty()
        try:
            payload = json.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return RustEligibilityDecision(
                schema_version=SHADOW_SCHEMA_VERSION,
                revision=0,
                eligible=False,
                disabled_reason="invalid-shadow-state",
                evidence=None,
                updated_at=_utc_now(),
            )
        if not isinstance(payload, Mapping):
            return RustEligibilityDecision(
                schema_version=SHADOW_SCHEMA_VERSION,
                revision=0,
                eligible=False,
                disabled_reason="invalid-shadow-state",
                evidence=None,
                updated_at=_utc_now(),
            )
        try:
            return self._decode(payload)
        except ShadowContractError:
            return RustEligibilityDecision(
                schema_version=SHADOW_SCHEMA_VERSION,
                revision=0,
                eligible=False,
                disabled_reason="invalid-shadow-state",
                evidence=None,
                updated_at=_utc_now(),
            )

    def record(self, evidence: RustEligibilityEvidence) -> RustEligibilityDecision:
        failed = list(evidence.failure_reasons)
        verified_release = False
        if evidence.release_evidence is not None:
            try:
                evidence.release_evidence.verify()
                verified_release = True
            except ShadowContractError:
                verified_release = False
        effective_packaging = evidence.packaging_passed and verified_release
        effective_release_gate = evidence.release_gate_passed and verified_release
        if evidence.packaging_passed and not effective_packaging:
            failed.append("packaging")
        if evidence.release_gate_passed and not effective_release_gate:
            failed.append("release-gate")
        gates = (
            ("contract-parity", evidence.contract_parity_passed),
            ("canonical-store-integrity", evidence.store_integrity_passed),
            ("crash-cut", evidence.crash_cut_passed),
            ("state-version", evidence.state_version_passed),
            ("packaging", effective_packaging),
            ("release-gate", effective_release_gate),
            ("production-equivalent-cohort", evidence.production_equivalent),
            ("stage-measurements", evidence.stages_complete),
        )
        for code, passed in gates:
            if not passed and code not in failed:
                failed.append(code)
        effective_evidence = replace(
            evidence,
            packaging_passed=effective_packaging,
            release_gate_passed=effective_release_gate,
            failure_reasons=tuple(dict.fromkeys(failed)),
        )
        with self._lock():
            decision = self._read_unlocked()
            next_decision = RustEligibilityDecision(
                schema_version=SHADOW_SCHEMA_VERSION,
                revision=decision.revision + 1,
                eligible=not effective_evidence.failure_reasons,
                disabled_reason=(
                    "" if not effective_evidence.failure_reasons else effective_evidence.failure_reasons[0]
                ),
                evidence=effective_evidence,
                updated_at=_utc_now(),
            )
            self.path.parent.mkdir(parents=True, exist_ok=True)
            temporary_path: Path | None = None
            try:
                with tempfile.NamedTemporaryFile(
                    mode="w",
                    encoding="utf-8",
                    dir=self.path.parent,
                    prefix=f".{self.path.name}.",
                    suffix=".tmp",
                    delete=False,
                ) as temporary:
                    json.dump(next_decision.to_dict(), temporary, indent=2, sort_keys=True)
                    temporary.write("\n")
                    temporary.flush()
                    os.fsync(temporary.fileno())
                    temporary_path = Path(temporary.name)
                temporary_path.replace(self.path)
                directory_fd = os.open(self.path.parent, os.O_RDONLY)
                try:
                    os.fsync(directory_fd)
                finally:
                    os.close(directory_fd)
            finally:
                if temporary_path is not None and temporary_path.exists():
                    temporary_path.unlink()
        return next_decision


__all__ = [
    "CanonicalStoreHashGuard",
    "CanonicalStoreSnapshot",
    "RustEligibilityDecision",
    "RustEligibilityEvidence",
    "RustEligibilityStore",
    "RustReleaseGateEvidence",
    "RustShadowProvider",
    "RustShadowProviderError",
    "ShadowCohortDefinition",
    "ShadowContractError",
    "ShadowParity",
    "ShadowSampleMetadata",
    "ShadowSampleResult",
    "ShadowSampleRunner",
    "ShadowStageMark",
    "compare_execution_receipts",
    "normalize_execution_receipt",
    "normalize_structured_failure",
    "shadow_artifact_sha256",
]
