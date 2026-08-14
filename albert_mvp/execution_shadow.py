"""Python-owned shadow orchestration for the Rust host-effects candidate.

The Python execution provider remains the canonical writer.  This module only
feeds explicitly bounded, production-equivalent fixtures to a Rust candidate,
compares transient results, and records whether the candidate is still eligible
for a future cutover.  Canonical Mission stores are treated as immutable
observation inputs during every shadow sample.
"""

from __future__ import annotations

import fcntl
import hashlib
import json
import math
import os
import re
import subprocess
import tempfile
from collections.abc import Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass, replace
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .execution import ExecutionReceipt, ExecutionRequest

SHADOW_SCHEMA_VERSION = 1
_DIGEST_PATTERN = re.compile(r"^[0-9a-f]{64}$")
_IDENTITY_PATTERN = re.compile(r"^[A-Za-z0-9._:/=-]+$")
_MAX_STORE_BYTES = 64 * 1024 * 1024
_MAX_PROVIDER_OUTPUT_BYTES = 16 * 1024 * 1024
_MAX_PROVIDER_TIMEOUT_SECONDS = 3_600.0
_FORBIDDEN_EVIDENCE_KINDS = {"reducer", "sidecar", "microbenchmark"}


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


def compare_execution_receipts(
    python_receipt: ExecutionReceipt | Mapping[str, Any],
    rust_receipt: ExecutionReceipt | Mapping[str, Any],
) -> ShadowParity:
    python_projection = normalize_execution_receipt(python_receipt)
    rust_projection = normalize_execution_receipt(rust_receipt)
    mismatches = tuple(
        f"{key}: Python={python_projection[key]!r}, Rust={rust_projection[key]!r}"
        for key in python_projection
        if python_projection[key] != rust_projection[key]
    )
    return ShadowParity(
        passed=not mismatches,
        mismatches=mismatches,
        python=python_projection,
        rust=rust_projection,
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
    evidence_kind: str = "production-equivalent"
    schema_version: int = SHADOW_SCHEMA_VERSION

    def __post_init__(self) -> None:
        if self.schema_version != SHADOW_SCHEMA_VERSION:
            raise ShadowContractError("unsupported shadow cohort schema")
        _validate_identity(self.cohort_id, label="shadow cohort id")
        _validate_identity(self.fixture_id, label="shadow fixture id")
        _canonical_path(Path(self.fixture_root), label="shadow fixture root")
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
        if not self.required_stages or len(self.required_stages) != len(set(self.required_stages)):
            raise ShadowContractError("shadow cohort stages must be non-empty and unique")
        if any(
            not isinstance(stage, str)
            or not stage.strip()
            or stage in _FORBIDDEN_EVIDENCE_KINDS
            for stage in self.required_stages
        ):
            raise ShadowContractError("shadow cohort stages must be production-equivalent")


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


@dataclass(frozen=True)
class ShadowSampleResult:
    metadata: ShadowSampleMetadata
    python_receipt: dict[str, Any]
    rust_receipt: dict[str, Any] | None
    parity: ShadowParity
    changed_store_paths: tuple[str, ...]
    store_unchanged: bool
    failure_codes: tuple[str, ...]

    @property
    def eligible(self) -> bool:
        return self.store_unchanged and self.parity.passed and not self.failure_codes


class ShadowSampleRunner:
    """Run one explicitly fixture-bound Rust shadow sample."""

    def __init__(
        self,
        rust_provider: Any,
        cohort: ShadowCohortDefinition,
        *,
        canonical_store_paths: Sequence[Path],
        approved_observation_paths: Sequence[Path] = (),
    ) -> None:
        self.rust_provider = rust_provider
        self.cohort = cohort
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

    def run(
        self,
        request: ExecutionRequest,
        python_receipt: ExecutionReceipt,
        metadata: ShadowSampleMetadata,
    ) -> ShadowSampleResult:
        self._validate_sample(request, metadata)
        before = self.guard.capture()
        rust_receipt: ExecutionReceipt | None = None
        failure_codes: list[str] = []
        try:
            rust_receipt = self.rust_provider.execute(request)
        except RustShadowProviderError as exc:
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
        after = self.guard.capture()
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
            )
        else:
            parity = compare_execution_receipts(python_receipt, rust_receipt)
            if not parity.passed:
                failure_codes.append("receipt-parity-failure")
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
            store_unchanged=not unauthorized,
            failure_codes=tuple(failure_codes),
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
        self.provider_id = "rust-shadow"

    def execute(self, request: ExecutionRequest) -> ExecutionReceipt:
        request.validate()
        payload = json.dumps(
            {"request": request.to_dict(include_input=True)},
            ensure_ascii=True,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        env = os.environ.copy()
        env.update(self.environment)
        try:
            completed = subprocess.run(
                self.command,
                input=payload,
                capture_output=True,
                cwd=request.working_directory,
                env=env,
                timeout=min(self.timeout_seconds, request.limits.timeout_seconds + 5.0),
                check=False,
            )
        except subprocess.TimeoutExpired as exc:
            return replace(
                ExecutionReceipt.unknown(
                    request,
                    error_message=f"Rust shadow provider timed out: {exc}",
                ),
                provider=self.provider_id,
            )
        except OSError as exc:
            return replace(
                ExecutionReceipt.unknown(
                    request,
                    error_message=f"Rust shadow provider could not start: {exc}",
                ),
                provider=self.provider_id,
            )
        if len(completed.stdout) > _MAX_PROVIDER_OUTPUT_BYTES:
            raise RustShadowProviderError(
                "rust-provider-output-limit",
                "Rust shadow provider response exceeded its bounded output.",
            )
        try:
            response = json.loads(completed.stdout.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            return replace(
                ExecutionReceipt.unknown(
                    request,
                    error_message=(
                        "Rust shadow provider crashed or returned invalid JSON: "
                        f"{exc}"
                    ),
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
            raise RustShadowProviderError(
                str(failure.get("code", "rust-provider-failure")),
                str(failure.get("message", "Rust shadow provider failed.")),
                bool(failure.get("recoverable", True)),
            )
        raw_receipt = response.get("receipt")
        if not isinstance(raw_receipt, Mapping):
            raise RustShadowProviderError(
                "rust-provider-contract-failure",
                "Rust shadow provider did not return a typed receipt.",
            )
        receipt = ExecutionReceipt.from_dict(raw_receipt)
        if receipt.request_id != request.request_id or receipt.request_digest != request.request_digest:
            raise RustShadowProviderError(
                "rust-provider-contract-failure",
                "Rust shadow receipt does not match the request identity.",
            )
        return replace(receipt, provider=self.provider_id)


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
        if len(self.stages) != len(set(self.stages)):
            raise ShadowContractError("Rust eligibility stages must be unique")
        if not isinstance(self.failure_reasons, tuple):
            raise ShadowContractError("Rust eligibility failure reasons are invalid")
        for reason in self.failure_reasons:
            _validate_identity(reason, label="Rust eligibility failure reason")

    @classmethod
    def all_passed(
        cls, *, sample_id: str, cohort_id: str, stages: Sequence[str]
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
        self.lock_path = self.path.with_name(f".{self.path.name}.lock")

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
            )
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
                stages=tuple(raw_evidence["stages"]),
                failure_reasons=tuple(failure_reasons),
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
        except (OSError, json.JSONDecodeError) as exc:
            raise ShadowContractError(f"Rust eligibility read failed: {exc}") from exc
        if not isinstance(payload, Mapping):
            raise ShadowContractError("Rust eligibility state must be an object")
        return self._decode(payload)

    def record(self, evidence: RustEligibilityEvidence) -> RustEligibilityDecision:
        failed = list(evidence.failure_reasons)
        gates = (
            ("contract-parity", evidence.contract_parity_passed),
            ("canonical-store-integrity", evidence.store_integrity_passed),
            ("crash-cut", evidence.crash_cut_passed),
            ("state-version", evidence.state_version_passed),
            ("packaging", evidence.packaging_passed),
            ("release-gate", evidence.release_gate_passed),
            ("production-equivalent-cohort", evidence.production_equivalent),
            ("stage-measurements", evidence.stages_complete),
        )
        for code, passed in gates:
            if not passed and code not in failed:
                failed.append(code)
        with self._lock():
            decision = self._read_unlocked()
            next_decision = RustEligibilityDecision(
                schema_version=SHADOW_SCHEMA_VERSION,
                revision=decision.revision + 1,
                eligible=not failed,
                disabled_reason="" if not failed else failed[0],
                evidence=replace(evidence, failure_reasons=tuple(failed)),
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
    "RustShadowProvider",
    "RustShadowProviderError",
    "ShadowCohortDefinition",
    "ShadowContractError",
    "ShadowParity",
    "ShadowSampleMetadata",
    "ShadowSampleResult",
    "ShadowSampleRunner",
    "compare_execution_receipts",
    "normalize_execution_receipt",
]
