"""Bounded, provider-neutral local inference contracts.

The Orchestrator owns governance. This module owns only model transport,
resource admission, and the non-authoritative lease used to serialize local
model work on the supported single-GPU baseline.
"""

from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass, field, replace
from datetime import datetime, timezone
import fcntl
import json
import math
import os
from pathlib import Path
import secrets
import socket
import time
from typing import Any, Callable, Iterator
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


INFERENCE_SCHEMA_VERSION = 1
_DEFAULT_CONTEXT_BUDGET = 8_192
_DEFAULT_OUTPUT_BUDGET = 1_024
_DEFAULT_MAX_OUTPUT_BYTES = 3_000_000
_DEFAULT_TIMEOUT_SECONDS = 120.0
_DEFAULT_QUEUE_WAIT_SECONDS = 30.0
_DEFAULT_QUEUE_LIMIT = 8
_DEFAULT_LEASE_AUDIT_LIMIT = 128
_DEFAULT_SAMPLING = {"temperature": 0.2, "top_p": 0.9}
_MAX_CONTEXT_BUDGET = 1_048_576
_MAX_OUTPUT_BUDGET = 1_048_576
_MAX_QUEUE_LIMIT = 64
_MAX_QUEUE_WAIT_SECONDS = 3_600.0
_MAX_TIMEOUT_SECONDS = 3_600.0
_MAX_PROFILE_SCHEMA_BYTES = 256_000
_MAX_PROFILE_OUTPUT_BYTES = 16_000_000
_MAX_LEASE_STATE_BYTES = 512_000


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def estimate_prompt_tokens(prompt: str) -> int:
    """Return the deterministic token estimate used by qualification planning."""

    if not isinstance(prompt, str):
        raise TypeError("inference prompt must be a string")
    encoded_bytes = len(prompt.encode("utf-8"))
    return math.ceil(encoded_bytes / 4) if encoded_bytes else 0


def _bound_prompt_tokens(prompt: str) -> int:
    """Return a tokenizer-independent pre-inference token upper bound.

    Ollama reports exact usage only after inference. Alfredo sends the prompt
    with ``raw`` enabled, so treating every UTF-8 byte as one token is a strict,
    deterministic admission bound without depending on a mutable model
    tokenizer or hidden prompt-template expansion.
    """

    if not isinstance(prompt, str):
        raise TypeError("inference prompt must be a string")
    encoded_bytes = len(prompt.encode("utf-8"))
    return encoded_bytes


def _copy_json(value: Any) -> Any:
    return json.loads(json.dumps(value, ensure_ascii=True))


def _processor_placement_is_valid(value: Any) -> bool:
    if value in {"auto", "cpu", "gpu"}:
        return True
    if not isinstance(value, str):
        return False
    parts = value.split(";")
    if len(parts) != 3 or not parts[0].startswith("requested="):
        return False
    requested = parts[0].removeprefix("requested=")
    if requested not in {"auto", "cpu", "gpu"}:
        return False
    if not parts[1].startswith("gpu_bytes=") or not parts[2].startswith("total_bytes="):
        return False
    try:
        gpu_bytes = int(parts[1].removeprefix("gpu_bytes="))
        total_bytes = int(parts[2].removeprefix("total_bytes="))
    except ValueError:
        return False
    return total_bytes > 0 and 0 <= gpu_bytes <= total_bytes


@dataclass(frozen=True)
class LocalInferenceProfile:
    """Versioned, exact model/runtime configuration for one inference turn."""

    profile_id: str = "default-v1"
    version: int = INFERENCE_SCHEMA_VERSION
    model: str = ""
    model_digest: str = "auto"
    keep_alive: str | int = "5m"
    context_budget: int = _DEFAULT_CONTEXT_BUDGET
    output_budget: int = _DEFAULT_OUTPUT_BUDGET
    thinking: bool = False
    sampling: dict[str, int | float] = field(
        default_factory=lambda: dict(_DEFAULT_SAMPLING)
    )
    schema: Any = "json"
    quantization: str = "auto"
    residency: str = "normal"
    processor_placement: str = "gpu"
    qualified: bool = True
    priority: int = 50
    queue_limit: int = _DEFAULT_QUEUE_LIMIT
    max_queue_wait_seconds: float = _DEFAULT_QUEUE_WAIT_SECONDS
    timeout_seconds: float = _DEFAULT_TIMEOUT_SECONDS
    max_output_bytes: int = _DEFAULT_MAX_OUTPUT_BYTES

    def __post_init__(self) -> None:
        if not isinstance(self.profile_id, str) or not self.profile_id.strip():
            raise ValueError("Local Inference Profile id must be non-empty")
        if (
            not isinstance(self.version, int)
            or isinstance(self.version, bool)
            or self.version != INFERENCE_SCHEMA_VERSION
        ):
            raise ValueError("unsupported Local Inference Profile version")
        if not isinstance(self.model, str) or not self.model.strip():
            raise ValueError("Local Inference Profile model must be non-empty")
        if not isinstance(self.model_digest, str):
            raise ValueError("Local Inference Profile model digest must be a string")
        if isinstance(self.keep_alive, bool) or not isinstance(
            self.keep_alive, (str, int)
        ):
            raise ValueError("Local Inference Profile keep_alive is invalid")
        if isinstance(self.keep_alive, str) and not self.keep_alive.strip():
            raise ValueError("Local Inference Profile keep_alive must be non-empty")
        if isinstance(self.keep_alive, int) and self.keep_alive <= 0:
            raise ValueError("Local Inference Profile keep_alive must be positive")
        for field_name in (
            "context_budget",
            "output_budget",
            "queue_limit",
            "max_output_bytes",
        ):
            value = getattr(self, field_name)
            if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
                raise ValueError(
                    f"Local Inference Profile {field_name} must be a positive integer"
                )
        if self.context_budget > _MAX_CONTEXT_BUDGET:
            raise ValueError("Local Inference Profile context budget is too large")
        if self.output_budget > _MAX_OUTPUT_BUDGET:
            raise ValueError("Local Inference Profile output budget is too large")
        if self.queue_limit > _MAX_QUEUE_LIMIT:
            raise ValueError("Local Inference Profile queue limit is too large")
        if self.max_output_bytes > _MAX_PROFILE_OUTPUT_BYTES:
            raise ValueError("Local Inference Profile output byte limit is too large")
        if self.output_budget >= self.context_budget:
            raise ValueError(
                "Local Inference Profile output budget must leave prompt headroom"
            )
        for field_name in (
            "max_queue_wait_seconds",
            "timeout_seconds",
        ):
            value = getattr(self, field_name)
            if (
                not isinstance(value, (int, float))
                or isinstance(value, bool)
                or value <= 0
            ):
                raise ValueError(
                    f"Local Inference Profile {field_name} must be positive"
                )
            if not math.isfinite(float(value)):
                raise ValueError(f"Local Inference Profile {field_name} must be finite")
        if self.max_queue_wait_seconds > _MAX_QUEUE_WAIT_SECONDS:
            raise ValueError("Local Inference Profile queue wait bound is too large")
        if self.timeout_seconds > _MAX_TIMEOUT_SECONDS:
            raise ValueError("Local Inference Profile timeout is too large")
        if (
            not isinstance(self.priority, int)
            or isinstance(self.priority, bool)
            or not 0 <= self.priority <= 100
        ):
            raise ValueError(
                "Local Inference Profile priority must be between 0 and 100"
            )
        if not isinstance(self.thinking, bool) or not isinstance(self.qualified, bool):
            raise ValueError("Local Inference Profile boolean fields are invalid")
        if self.residency not in {"normal", "ephemeral"}:
            raise ValueError("Local Inference Profile residency is invalid")
        if not isinstance(self.quantization, str) or not self.quantization.strip():
            raise ValueError("Local Inference Profile quantization must be non-empty")
        if not _processor_placement_is_valid(self.processor_placement):
            raise ValueError("Local Inference Profile processor placement is invalid")
        if not isinstance(self.sampling, dict):
            raise ValueError("Local Inference Profile sampling must be an object")
        for key, value in self.sampling.items():
            if not isinstance(key, str) or not key.strip():
                raise ValueError("Local Inference Profile sampling keys are invalid")
            if (
                not isinstance(value, (int, float))
                or isinstance(value, bool)
                or not math.isfinite(float(value))
            ):
                raise ValueError("Local Inference Profile sampling values are invalid")
        if not isinstance(self.schema, (dict, str)):
            raise ValueError(
                "Local Inference Profile schema must be an object or 'json'"
            )
        if isinstance(self.schema, str) and self.schema != "json":
            raise ValueError("Local Inference Profile schema string must be 'json'")
        try:
            encoded_schema = json.dumps(self.schema, allow_nan=False)
        except (TypeError, ValueError, RecursionError) as exc:
            raise ValueError(
                "Local Inference Profile schema must be JSON-compatible"
            ) from exc
        if len(encoded_schema.encode("utf-8")) > _MAX_PROFILE_SCHEMA_BYTES:
            raise ValueError("Local Inference Profile schema is too large")

    @classmethod
    def from_dict(
        cls,
        data: dict[str, Any] | None,
        *,
        model: str | None = None,
        profile_id: str | None = None,
    ) -> "LocalInferenceProfile":
        if data is None:
            data = {}
        if not isinstance(data, dict):
            raise ValueError("Local Inference Profile must be an object")
        raw_model = data.get("model", model or "")
        raw_profile_id = data.get(
            "profile_id", data.get("id", profile_id or "default-v1")
        )
        if not isinstance(raw_model, str) or not isinstance(raw_profile_id, str):
            raise ValueError("Local Inference Profile identity fields must be strings")
        selected_model = raw_model.strip()
        selected_profile_id = raw_profile_id.strip()
        raw_sampling = data.get("sampling", _DEFAULT_SAMPLING)
        if not isinstance(raw_sampling, dict):
            raise ValueError("Local Inference Profile sampling must be an object")
        try:
            schema = _copy_json(data.get("schema", "json"))
        except (TypeError, ValueError, RecursionError) as exc:
            raise ValueError(
                "Local Inference Profile schema must be JSON-compatible"
            ) from exc
        string_defaults = {
            "model_digest": "auto",
            "quantization": "auto",
            "residency": "normal",
            "processor_placement": "gpu",
        }
        string_values: dict[str, str] = {}
        for field_name, default in string_defaults.items():
            value = data.get(field_name, default)
            if not isinstance(value, str):
                raise ValueError(
                    f"Local Inference Profile {field_name} must be a string"
                )
            string_values[field_name] = value.strip() or default
        return cls(
            profile_id=selected_profile_id,
            version=data.get(
                "version", data.get("schema_version", INFERENCE_SCHEMA_VERSION)
            ),
            model=selected_model,
            model_digest=string_values["model_digest"],
            keep_alive=data.get("keep_alive", "5m"),
            context_budget=data.get("context_budget", _DEFAULT_CONTEXT_BUDGET),
            output_budget=data.get("output_budget", _DEFAULT_OUTPUT_BUDGET),
            thinking=data.get("thinking", False),
            sampling=dict(raw_sampling),
            schema=schema,
            quantization=string_values["quantization"],
            residency=string_values["residency"],
            processor_placement=string_values["processor_placement"],
            qualified=data.get("qualified", True),
            priority=data.get("priority", 50),
            queue_limit=data.get("queue_limit", _DEFAULT_QUEUE_LIMIT),
            max_queue_wait_seconds=data.get(
                "max_queue_wait_seconds", _DEFAULT_QUEUE_WAIT_SECONDS
            ),
            timeout_seconds=data.get("timeout_seconds", _DEFAULT_TIMEOUT_SECONDS),
            max_output_bytes=data.get("max_output_bytes", _DEFAULT_MAX_OUTPUT_BYTES),
        )

    @classmethod
    def default(
        cls, model: str, *, profile_id: str = "default-v1"
    ) -> "LocalInferenceProfile":
        return cls(profile_id=profile_id, model=model)

    def with_schema(self, schema: Any) -> "LocalInferenceProfile":
        return replace(self, schema=_copy_json(schema))

    def to_dict(self) -> dict[str, Any]:
        return {
            "profile_id": self.profile_id,
            "version": self.version,
            "model": self.model,
            "model_digest": self.model_digest,
            "keep_alive": self.keep_alive,
            "context_budget": self.context_budget,
            "output_budget": self.output_budget,
            "thinking": self.thinking,
            "sampling": _copy_json(self.sampling),
            "schema": _copy_json(self.schema),
            "quantization": self.quantization,
            "residency": self.residency,
            "processor_placement": self.processor_placement,
            "qualified": self.qualified,
            "priority": self.priority,
            "queue_limit": self.queue_limit,
            "max_queue_wait_seconds": self.max_queue_wait_seconds,
            "timeout_seconds": self.timeout_seconds,
            "max_output_bytes": self.max_output_bytes,
        }


class LocalInferenceLeaseError(RuntimeError):
    """Raised when a non-authoritative Local Inference Lease cannot be acquired."""

    def __init__(self, outcome: str, message: str):
        super().__init__(message)
        self.outcome = outcome


@dataclass
class LocalInferenceLeaseHandle:
    _lease: "LocalInferenceLease"
    _lock_file: Any
    snapshot: dict[str, Any]
    _outcome: str = "released"
    _resident_model: str = ""
    _resident_digest: str = ""
    _released: bool = False

    def mark_resident(self, *, model: str, digest: str) -> None:
        self._resident_model = model
        self._resident_digest = digest

    def finish(
        self,
        outcome: str,
        *,
        model: str = "",
        digest: str = "",
        residency: str = "normal",
    ) -> None:
        self._outcome = outcome
        if residency == "normal" and model and digest:
            self.mark_resident(model=model, digest=digest)

    def release(self) -> None:
        if self._released:
            return
        self._released = True
        self._lease._release(self)


class _LeaseAttempt:
    def __init__(
        self,
        lease: "LocalInferenceLease",
        cancellation_requested: Callable[[], bool] | None,
    ):
        self.lease = lease
        self.cancellation_requested = cancellation_requested
        self.handle: LocalInferenceLeaseHandle | None = None

    def __enter__(self) -> LocalInferenceLeaseHandle:
        self.handle = self.lease._acquire(self.cancellation_requested)
        return self.handle

    def __exit__(self, *_args: object) -> None:
        if self.handle is not None:
            self.handle.release()


class LocalInferenceLease:
    """A cross-process, bounded, priority-aware non-authoritative model lease."""

    def __init__(
        self,
        *,
        runtime_root: Path,
        mission_id: str,
        session_id: str,
        request_id: str,
        priority: int = 50,
        queue_limit: int = _DEFAULT_QUEUE_LIMIT,
        max_queue_wait_seconds: float = _DEFAULT_QUEUE_WAIT_SECONDS,
        model: str = "",
        model_digest: str = "",
        residency: str = "normal",
        poll_seconds: float = 0.01,
    ):
        if (
            not isinstance(mission_id, str)
            or not isinstance(session_id, str)
            or not isinstance(request_id, str)
            or not mission_id.strip()
            or not session_id.strip()
            or not request_id.strip()
        ):
            raise ValueError("Local Inference Lease identity fields must be non-empty")
        if (
            not isinstance(priority, int)
            or isinstance(priority, bool)
            or not 0 <= priority <= 100
        ):
            raise ValueError("Local Inference Lease priority must be between 0 and 100")
        if (
            not isinstance(queue_limit, int)
            or isinstance(queue_limit, bool)
            or queue_limit <= 0
        ):
            raise ValueError("Local Inference Lease queue limit must be positive")
        if (
            not isinstance(max_queue_wait_seconds, (int, float))
            or isinstance(max_queue_wait_seconds, bool)
            or max_queue_wait_seconds <= 0
        ):
            raise ValueError("Local Inference Lease wait bound must be positive")
        if queue_limit > _MAX_QUEUE_LIMIT:
            raise ValueError("Local Inference Lease queue limit is too large")
        if (
            not math.isfinite(float(max_queue_wait_seconds))
            or max_queue_wait_seconds > _MAX_QUEUE_WAIT_SECONDS
        ):
            raise ValueError("Local Inference Lease wait bound is invalid")
        if not isinstance(model, str) or not isinstance(model_digest, str):
            raise ValueError("Local Inference Lease model identity must be strings")
        if residency not in {"normal", "ephemeral"}:
            raise ValueError("Local Inference Lease residency is invalid")
        if (
            not isinstance(poll_seconds, (int, float))
            or isinstance(poll_seconds, bool)
            or not math.isfinite(float(poll_seconds))
            or poll_seconds <= 0
        ):
            raise ValueError("Local Inference Lease poll interval is invalid")
        self.runtime_root = Path(runtime_root).resolve()
        self.mission_id = mission_id
        self.session_id = session_id
        self.request_id = request_id
        self.priority = priority
        self.queue_limit = queue_limit
        self.max_queue_wait_seconds = float(max_queue_wait_seconds)
        self.model = model
        self.model_digest = model_digest
        self.residency = residency
        self.poll_seconds = max(0.001, float(poll_seconds))
        self.lease_id = f"lease:{secrets.token_hex(12)}"
        self._root = self.runtime_root / "inference"
        self._state_path = self._root / "lease-state.json"
        self._state_lock_path = self._root / "lease-state.lock"
        self._exclusive_lock_path = self._root / "lease.lock"

    @property
    def state_path(self) -> Path:
        return self._state_path

    def acquire(
        self,
        cancellation_requested: Callable[[], bool] | None = None,
    ) -> _LeaseAttempt:
        return _LeaseAttempt(self, cancellation_requested)

    @classmethod
    def inspect(cls, runtime_root: Path) -> dict[str, Any]:
        root = Path(runtime_root).resolve() / "inference"
        state_path = root / "lease-state.json"
        if not state_path.exists():
            return cls._empty_state()
        try:
            if state_path.stat().st_size > _MAX_LEASE_STATE_BYTES:
                raise ValueError("ledger exceeds bounded size")
            data = json.loads(state_path.read_text(encoding="utf-8"))
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            ) from exc
        return cls._validated_state(data)

    @staticmethod
    def _empty_state() -> dict[str, Any]:
        return {
            "schema_version": INFERENCE_SCHEMA_VERSION,
            "active": None,
            "queued": [],
            "resident": None,
            "audit": [],
            "next_sequence": 1,
        }

    @classmethod
    def _validated_state(cls, data: Any) -> dict[str, Any]:
        if (
            not isinstance(data, dict)
            or data.get("schema_version") != INFERENCE_SCHEMA_VERSION
        ):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        queued = data.get("queued")
        audit = data.get("audit")
        if not isinstance(queued, list) or not isinstance(audit, list):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        active = data.get("active")
        if active is not None and not isinstance(active, dict):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        resident = data.get("resident")
        if resident is not None and not isinstance(resident, dict):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        if len(queued) > _MAX_QUEUE_LIMIT or len(audit) > _DEFAULT_LEASE_AUDIT_LIMIT:
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        if active is not None and len(queued) + 1 > _MAX_QUEUE_LIMIT:
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        if (
            not isinstance(data.get("next_sequence"), int)
            or isinstance(data["next_sequence"], bool)
            or data["next_sequence"] < 1
        ):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        entries = [cls._validated_entry(item, active=False) for item in queued]
        if active is not None:
            entries.append(cls._validated_entry(active, active=True))
        lease_ids = [entry["lease_id"] for entry in entries]
        if len(lease_ids) != len(set(lease_ids)):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        if resident is not None:
            cls._validated_resident(resident)
        for item in audit:
            cls._validated_audit(item)
        try:
            encoded = json.dumps(data, ensure_ascii=True, allow_nan=False)
            if len(encoded.encode("utf-8")) > _MAX_LEASE_STATE_BYTES:
                raise ValueError("ledger exceeds bounded size")
            return json.loads(encoded)
        except (TypeError, ValueError, json.JSONDecodeError) as exc:
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            ) from exc

    @staticmethod
    def _validated_entry(entry: Any, *, active: bool) -> dict[str, Any]:
        if not isinstance(entry, dict):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        required_strings = (
            "lease_id",
            "request_id",
            "mission_id",
            "session_id",
            "model",
            "model_digest",
            "residency",
            "enqueued_at",
        )
        if any(
            not isinstance(entry.get(field_name), str)
            for field_name in required_strings
        ):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        if any(
            not isinstance(entry.get(field_name), str) or not entry[field_name].strip()
            for field_name in (
                "lease_id",
                "request_id",
                "mission_id",
                "session_id",
                "enqueued_at",
            )
        ):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        if entry["residency"] not in {"normal", "ephemeral"}:
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        if (
            not isinstance(entry.get("priority"), int)
            or isinstance(entry["priority"], bool)
            or not 0 <= entry["priority"] <= 100
            or not isinstance(entry.get("sequence"), int)
            or isinstance(entry["sequence"], bool)
            or entry["sequence"] < 1
        ):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        if active:
            if (
                not isinstance(entry.get("started_at"), str)
                or not entry["started_at"].strip()
                or not isinstance(entry.get("resident_match"), bool)
                or not isinstance(entry.get("model_swap"), bool)
            ):
                raise LocalInferenceLeaseError(
                    "ledger-invalid", "Local Inference Lease ledger is invalid"
                )
        return entry

    @staticmethod
    def _validated_resident(resident: Any) -> None:
        if (
            not isinstance(resident, dict)
            or any(
                not isinstance(resident.get(field_name), str)
                or not resident[field_name].strip()
                for field_name in ("model", "digest", "residency", "updated_at")
            )
            or resident.get("residency") != "normal"
        ):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )

    @staticmethod
    def _validated_audit(item: Any) -> None:
        if not isinstance(item, dict):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        required_strings = (
            "event",
            "recorded_at",
            "lease_id",
            "request_id",
            "mission_id",
            "session_id",
        )
        if any(
            not isinstance(item.get(field_name), str) for field_name in required_strings
        ):
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            )
        for field_name in ("outcome",):
            if field_name in item and not isinstance(item[field_name], str):
                raise LocalInferenceLeaseError(
                    "ledger-invalid", "Local Inference Lease ledger is invalid"
                )
        for field_name in ("resident_match", "model_swap"):
            if field_name in item and not isinstance(item[field_name], bool):
                raise LocalInferenceLeaseError(
                    "ledger-invalid", "Local Inference Lease ledger is invalid"
                )

    @contextmanager
    def _state_lock(self) -> Iterator[None]:
        self._root.mkdir(parents=True, exist_ok=True)
        with self._state_lock_path.open("a+", encoding="utf-8") as lock_file:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)

    def _read_state(self) -> dict[str, Any]:
        if not self._state_path.exists():
            return self._empty_state()
        try:
            if self._state_path.stat().st_size > _MAX_LEASE_STATE_BYTES:
                raise ValueError("ledger exceeds bounded size")
            return self._validated_state(
                json.loads(self._state_path.read_text(encoding="utf-8"))
            )
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise LocalInferenceLeaseError(
                "ledger-invalid", "Local Inference Lease ledger is invalid"
            ) from exc

    def _write_state(self, state: dict[str, Any]) -> None:
        state = self._validated_state(state)
        self._root.mkdir(parents=True, exist_ok=True)
        temporary = self._state_path.with_name(
            f".{self._state_path.name}.{os.getpid()}.tmp"
        )
        temporary.write_text(
            json.dumps(state, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, self._state_path)

    @staticmethod
    def _audit(
        state: dict[str, Any], event: str, entry: dict[str, Any], **extra: Any
    ) -> None:
        record = {
            "event": event,
            "recorded_at": _utc_now(),
            "lease_id": entry.get("lease_id", ""),
            "request_id": entry.get("request_id", ""),
            "mission_id": entry.get("mission_id", ""),
            "session_id": entry.get("session_id", ""),
            **extra,
        }
        state["audit"] = [*state.get("audit", []), record][-_DEFAULT_LEASE_AUDIT_LIMIT:]

    def _entry(self, sequence: int) -> dict[str, Any]:
        return {
            "lease_id": self.lease_id,
            "request_id": self.request_id,
            "mission_id": self.mission_id,
            "session_id": self.session_id,
            "model": self.model,
            "model_digest": self.model_digest,
            "residency": self.residency,
            "priority": self.priority,
            "sequence": sequence,
            "enqueued_at": _utc_now(),
        }

    @staticmethod
    def _queue_sort_key(
        entry: dict[str, Any], resident: dict[str, Any] | None
    ) -> tuple[int, int, int]:
        resident_match = bool(
            resident
            and entry.get("residency") == "normal"
            and entry.get("model")
            and entry.get("model") == resident.get("model")
            and entry.get("model_digest")
            and entry.get("model_digest") == resident.get("digest")
        )
        return (
            -int(entry.get("priority", 0)),
            0 if resident_match else 1,
            int(entry.get("sequence", 0)),
        )

    def _register(self) -> dict[str, Any]:
        with self._state_lock():
            state = self._read_state()
            active_count = 1 if state.get("active") else 0
            if active_count + len(state["queued"]) >= self.queue_limit:
                entry = self._entry(int(state["next_sequence"]))
                self._audit(state, "queue-full", entry, outcome="queue-full")
                self._write_state(state)
                raise LocalInferenceLeaseError(
                    "queue-full",
                    "Local Inference Lease queue is full; model work remains non-authoritative.",
                )
            sequence = int(state["next_sequence"])
            state["next_sequence"] = sequence + 1
            entry = self._entry(sequence)
            state["queued"].append(entry)
            self._audit(state, "queued", entry, outcome="queued")
            self._write_state(state)
            return entry

    def _remove_queued(self, outcome: str) -> None:
        with self._state_lock():
            state = self._read_state()
            entry = next(
                (
                    item
                    for item in state["queued"]
                    if item.get("lease_id") == self.lease_id
                ),
                self._entry(0),
            )
            state["queued"] = [
                item
                for item in state["queued"]
                if item.get("lease_id") != self.lease_id
            ]
            self._audit(state, outcome, entry, outcome=outcome)
            self._write_state(state)

    def _acquire(
        self,
        cancellation_requested: Callable[[], bool] | None,
    ) -> LocalInferenceLeaseHandle:
        entry = self._register()
        deadline = time.monotonic() + self.max_queue_wait_seconds
        while True:
            if cancellation_requested is not None and cancellation_requested():
                self._remove_queued("cancelled")
                raise LocalInferenceLeaseError(
                    "cancelled", "Local Inference Lease request was cancelled"
                )
            if time.monotonic() >= deadline:
                self._remove_queued("lease-timeout")
                raise LocalInferenceLeaseError(
                    "lease-timeout",
                    "Local Inference Lease queue wait exceeded its bound",
                )
            lock_file = self._exclusive_lock_path.open("a+")
            try:
                try:
                    fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                except OSError:
                    lock_file.close()
                    time.sleep(self.poll_seconds)
                    continue
                with self._state_lock():
                    state = self._read_state()
                    stale_active = state.get("active")
                    if stale_active is not None:
                        self._audit(
                            state,
                            "recovered-stale-active",
                            stale_active,
                            outcome="recovered-stale-active",
                        )
                        state["active"] = None
                    queued = [
                        item
                        for item in state["queued"]
                        if item.get("lease_id") != self.lease_id
                    ]
                    if not any(
                        item.get("lease_id") == self.lease_id
                        for item in state["queued"]
                    ):
                        self._write_state(state)
                        fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
                        lock_file.close()
                        raise LocalInferenceLeaseError(
                            "lease-lost",
                            "Local Inference Lease queue entry disappeared before acquisition",
                        )
                    chosen = min(
                        state["queued"],
                        key=lambda item: self._queue_sort_key(
                            item, state.get("resident")
                        ),
                    )
                    if chosen.get("lease_id") != self.lease_id:
                        state["active"] = None
                        self._write_state(state)
                        fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
                        lock_file.close()
                        time.sleep(self.poll_seconds)
                        continue
                    resident = state.get("resident")
                    resident_match = bool(
                        resident
                        and self.residency == "normal"
                        and self.model == resident.get("model")
                        and self.model_digest == resident.get("digest")
                    )
                    active = {
                        **entry,
                        "started_at": _utc_now(),
                        "resident_match": resident_match,
                        "model_swap": bool(resident and not resident_match),
                    }
                    state["queued"] = queued
                    state["active"] = active
                    self._audit(
                        state,
                        "acquired",
                        active,
                        outcome="acquired",
                        resident_match=resident_match,
                        model_swap=active["model_swap"],
                    )
                    self._write_state(state)
                    snapshot = _copy_json(active)
                return LocalInferenceLeaseHandle(self, lock_file, snapshot)
            except LocalInferenceLeaseError:
                raise
            except Exception:
                try:
                    fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
                finally:
                    lock_file.close()
                raise

    def _release(self, handle: LocalInferenceLeaseHandle) -> None:
        with self._state_lock():
            state = self._read_state()
            active = state.get("active")
            if isinstance(active, dict) and active.get("lease_id") == self.lease_id:
                state["active"] = None
                if handle._resident_model and handle._resident_digest:
                    state["resident"] = {
                        "model": handle._resident_model,
                        "digest": handle._resident_digest,
                        "residency": "normal",
                        "updated_at": _utc_now(),
                    }
                self._audit(state, "released", active, outcome=handle._outcome)
                self._write_state(state)
        try:
            fcntl.flock(handle._lock_file.fileno(), fcntl.LOCK_UN)
        finally:
            handle._lock_file.close()


@dataclass
class LocalInferenceResult:
    receipt: dict[str, Any]
    value: Any = None
    raw_output: str = ""

    @property
    def authoritative(self) -> bool:
        return bool(self.receipt.get("authoritative", False))


class _InferenceTimeout(RuntimeError):
    pass


class _InferenceCancelled(RuntimeError):
    pass


class _SchemaValidationError(ValueError):
    pass


def _validate_schema_value(
    value: Any, schema: Any, *, path: str = "value", depth: int = 0
) -> None:
    """Validate the bounded JSON subset used by Ollama's ``format`` contract."""

    if depth > 32:
        raise _SchemaValidationError("schema nesting exceeds the bounded limit")
    if schema == "json":
        return
    if not isinstance(schema, dict):
        raise _SchemaValidationError("declared schema is not an object")
    if "enum" in schema:
        enum = schema["enum"]
        if not isinstance(enum, list) or not any(
            value == candidate for candidate in enum
        ):
            raise _SchemaValidationError(
                f"{path} is not one of the declared enum values"
            )
    schema_type = schema.get("type")
    if schema_type is not None:
        allowed_types = schema_type if isinstance(schema_type, list) else [schema_type]
        if not all(isinstance(item, str) for item in allowed_types):
            raise _SchemaValidationError(f"{path} has an invalid schema type")

        def matches_type(candidate: str) -> bool:
            if candidate == "object":
                return isinstance(value, dict)
            if candidate == "array":
                return isinstance(value, list)
            if candidate == "string":
                return isinstance(value, str)
            if candidate == "integer":
                return isinstance(value, int) and not isinstance(value, bool)
            if candidate == "number":
                return isinstance(value, (int, float)) and not isinstance(value, bool)
            if candidate == "boolean":
                return isinstance(value, bool)
            if candidate == "null":
                return value is None
            raise _SchemaValidationError(
                f"{path} has unsupported schema type {candidate!r}"
            )

        if not any(matches_type(candidate) for candidate in allowed_types):
            raise _SchemaValidationError(
                f"{path} does not match the declared schema type"
            )
    object_keywords = {"required", "properties", "additionalProperties"}
    if any(keyword in schema for keyword in object_keywords) and not isinstance(
        value, dict
    ):
        raise _SchemaValidationError(f"{path} is not an object")
    array_keywords = {"items", "minItems", "maxItems"}
    if any(keyword in schema for keyword in array_keywords) and not isinstance(
        value, list
    ):
        raise _SchemaValidationError(f"{path} is not an array")
    string_keywords = {"minLength", "maxLength"}
    if any(keyword in schema for keyword in string_keywords) and not isinstance(
        value, str
    ):
        raise _SchemaValidationError(f"{path} is not a string")
    number_keywords = {"minimum", "maximum"}
    if any(keyword in schema for keyword in number_keywords) and (
        not isinstance(value, (int, float)) or isinstance(value, bool)
    ):
        raise _SchemaValidationError(f"{path} is not a number")
    if isinstance(value, dict):
        required = schema.get("required", [])
        if not isinstance(required, list) or not all(
            isinstance(item, str) for item in required
        ):
            raise _SchemaValidationError(f"{path} has an invalid required field list")
        missing = [item for item in required if item not in value]
        if missing:
            raise _SchemaValidationError(
                f"{path} is missing required field(s): {', '.join(missing)}"
            )
        properties = schema.get("properties", {})
        if not isinstance(properties, dict):
            raise _SchemaValidationError(f"{path} has invalid properties")
        if schema.get("additionalProperties") is False:
            unexpected = [key for key in value if key not in properties]
            if unexpected:
                raise _SchemaValidationError(
                    f"{path} contains undeclared field(s): {', '.join(map(str, unexpected))}"
                )
        for key, child_schema in properties.items():
            if key in value:
                _validate_schema_value(
                    value[key],
                    child_schema,
                    path=f"{path}.{key}",
                    depth=depth + 1,
                )
    if isinstance(value, list):
        for index, item in enumerate(value):
            if "items" in schema:
                _validate_schema_value(
                    item,
                    schema["items"],
                    path=f"{path}[{index}]",
                    depth=depth + 1,
                )
        for field_name, comparator in (
            ("minItems", lambda actual, bound: actual < bound),
            ("maxItems", lambda actual, bound: actual > bound),
        ):
            if field_name in schema:
                bound = schema[field_name]
                if (
                    not isinstance(bound, int)
                    or isinstance(bound, bool)
                    or comparator(len(value), bound)
                ):
                    raise _SchemaValidationError(f"{path} violates {field_name}")
    if isinstance(value, str):
        for field_name, comparator in (
            ("minLength", lambda actual, bound: actual < bound),
            ("maxLength", lambda actual, bound: actual > bound),
        ):
            if field_name in schema:
                bound = schema[field_name]
                if (
                    not isinstance(bound, int)
                    or isinstance(bound, bool)
                    or comparator(len(value), bound)
                ):
                    raise _SchemaValidationError(f"{path} violates {field_name}")
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        for field_name, comparator in (
            ("minimum", lambda actual, bound: actual < bound),
            ("maximum", lambda actual, bound: actual > bound),
        ):
            if field_name in schema:
                bound = schema[field_name]
                if (
                    not isinstance(bound, (int, float))
                    or isinstance(bound, bool)
                    or not math.isfinite(float(bound))
                    or comparator(value, bound)
                ):
                    raise _SchemaValidationError(f"{path} violates {field_name}")


class LocalInferenceAdapter:
    """Bounded Ollama HTTP adapter with complete-result admission."""

    def __init__(
        self,
        *,
        runtime_root: Path,
        base_url: str | None = None,
        opener: Callable[..., Any] | None = None,
        clock: Callable[[], float] = time.monotonic,
    ):
        configured_host = base_url or os.environ.get(
            "OLLAMA_HOST", "http://127.0.0.1:11434"
        )
        if "://" not in configured_host:
            configured_host = f"http://{configured_host}"
        self.base_url = configured_host.rstrip("/")
        self.runtime_root = Path(runtime_root).resolve()
        self.opener = opener or urlopen
        self.clock = clock

    def infer(
        self,
        *,
        prompt: str,
        profile: LocalInferenceProfile,
        mission_id: str,
        session_id: str,
        turn_kind: str,
        validator: Callable[[str], Any],
        cancellation_requested: Callable[[], bool] | None = None,
        request_id: str | None = None,
    ) -> LocalInferenceResult:
        if not mission_id.strip() or not turn_kind.strip():
            raise ValueError("Local Inference turn identity is incomplete")
        if not isinstance(prompt, str):
            raise TypeError("Local Inference prompt must be a string")
        request_id = request_id or f"inference-turn:{secrets.token_hex(12)}"
        prompt_tokens = _bound_prompt_tokens(prompt)
        admission = {
            "admitted": prompt_tokens + profile.output_budget <= profile.context_budget,
            "prompt_tokens": prompt_tokens,
            "output_headroom": profile.context_budget - prompt_tokens,
            "context_budget": profile.context_budget,
            "output_budget": profile.output_budget,
        }
        if not admission["admitted"]:
            return LocalInferenceResult(
                receipt=self._receipt(
                    request_id=request_id,
                    mission_id=mission_id,
                    session_id=session_id,
                    turn_kind=turn_kind,
                    profile=profile,
                    admission=admission,
                    outcome="rejected-over-budget",
                    error="Prompt admission leaves insufficient output headroom.",
                )
            )
        if not profile.qualified:
            return LocalInferenceResult(
                receipt=self._receipt(
                    request_id=request_id,
                    mission_id=mission_id,
                    session_id=session_id,
                    turn_kind=turn_kind,
                    profile=profile,
                    admission=admission,
                    outcome="profile-not-qualified",
                    error="Local Inference Profile is not qualified for governed work.",
                )
            )
        try:
            resolved_profile = self._resolve_profile(profile)
        except _MetadataError as exc:
            return LocalInferenceResult(
                receipt=self._receipt(
                    request_id=request_id,
                    mission_id=mission_id,
                    session_id=session_id,
                    turn_kind=turn_kind,
                    profile=profile,
                    admission=admission,
                    outcome=exc.outcome,
                    error=str(exc),
                )
            )
        lease = LocalInferenceLease(
            runtime_root=self.runtime_root,
            mission_id=mission_id,
            session_id=session_id or "mission-turn",
            request_id=request_id,
            priority=resolved_profile.priority,
            queue_limit=resolved_profile.queue_limit,
            max_queue_wait_seconds=resolved_profile.max_queue_wait_seconds,
            model=resolved_profile.model,
            model_digest=resolved_profile.model_digest,
            residency=resolved_profile.residency,
        )
        try:
            with lease.acquire(cancellation_requested) as handle:
                result = self._stream(
                    prompt=prompt,
                    profile=resolved_profile,
                    mission_id=mission_id,
                    session_id=session_id,
                    turn_kind=turn_kind,
                    request_id=request_id,
                    admission=admission,
                    validator=validator,
                    cancellation_requested=cancellation_requested,
                    lease_snapshot=handle.snapshot,
                )
                runtime_profile: LocalInferenceProfile | None = None
                if result.authoritative:
                    try:
                        runtime_profile = self._resolve_runtime_profile(
                            resolved_profile
                        )
                    except _MetadataError as exc:
                        result.receipt["outcome"] = exc.outcome
                        result.receipt["authoritative"] = False
                        result.receipt["error"] = str(exc)
                        result.value = None
                    else:
                        result.receipt["profile"] = runtime_profile.to_dict()
                handle.finish(
                    str(result.receipt["outcome"]),
                    model=runtime_profile.model if runtime_profile is not None else "",
                    digest=(
                        runtime_profile.model_digest
                        if runtime_profile is not None
                        else ""
                    ),
                    residency=(
                        runtime_profile.residency
                        if runtime_profile is not None
                        else resolved_profile.residency
                    ),
                )
                return result
        except LocalInferenceLeaseError as exc:
            return LocalInferenceResult(
                receipt=self._receipt(
                    request_id=request_id,
                    mission_id=mission_id,
                    session_id=session_id,
                    turn_kind=turn_kind,
                    profile=resolved_profile,
                    admission=admission,
                    outcome=exc.outcome,
                    error=str(exc),
                )
            )

    def _resolve_profile(self, profile: LocalInferenceProfile) -> LocalInferenceProfile:
        request = Request(f"{self.base_url}/api/tags", method="GET")
        try:
            with self.opener(
                request, timeout=min(profile.timeout_seconds, 10.0)
            ) as response:
                raw = response.read(_DEFAULT_MAX_OUTPUT_BYTES + 1)
            if (
                not isinstance(raw, (bytes, bytearray))
                or len(raw) > _DEFAULT_MAX_OUTPUT_BYTES
            ):
                raise ValueError("Local model metadata exceeded the bounded byte limit")
            data = json.loads(raw.decode("utf-8"))
        except (
            OSError,
            TimeoutError,
            socket.timeout,
            HTTPError,
            URLError,
            ValueError,
            json.JSONDecodeError,
        ) as exc:
            raise _MetadataError(
                "metadata-error", f"Local model metadata unavailable: {exc}"
            ) from exc
        models = data.get("models") if isinstance(data, dict) else None
        if not isinstance(models, list):
            raise _MetadataError(
                "metadata-error", "Local model metadata did not include models"
            )
        match = next(
            (
                item
                for item in models
                if isinstance(item, dict)
                and str(item.get("name", item.get("model", ""))).strip()
                == profile.model
            ),
            None,
        )
        if match is None:
            raise _MetadataError(
                "metadata-error", f"Local model {profile.model!r} is not installed"
            )
        digest = str(match.get("digest", "")).strip()
        if not digest:
            raise _MetadataError(
                "metadata-error", "Local model metadata did not include an exact digest"
            )
        if profile.model_digest not in {"", "auto"} and profile.model_digest != digest:
            raise _MetadataError(
                "digest-mismatch",
                f"Local model digest changed from {profile.model_digest} to {digest}",
            )
        details = match.get("details", {})
        if not isinstance(details, dict):
            details = {}
        quantization = profile.quantization
        if quantization == "auto":
            quantization = (
                str(
                    details.get(
                        "quantization_level", details.get("quantization", "unknown")
                    )
                ).strip()
                or "unknown"
            )
        return replace(
            profile,
            model_digest=digest,
            quantization=quantization,
        )

    def _resolve_runtime_profile(
        self,
        profile: LocalInferenceProfile,
    ) -> LocalInferenceProfile:
        """Bind a completed turn to the model Ollama actually kept resident."""

        request = Request(f"{self.base_url}/api/ps", method="GET")
        try:
            with self.opener(
                request,
                timeout=min(profile.timeout_seconds, 10.0),
            ) as response:
                raw = response.read(_DEFAULT_MAX_OUTPUT_BYTES + 1)
            if (
                not isinstance(raw, (bytes, bytearray))
                or len(raw) > _DEFAULT_MAX_OUTPUT_BYTES
            ):
                raise ValueError(
                    "running model metadata exceeded the bounded byte limit"
                )
            data = json.loads(raw.decode("utf-8"))
        except (
            OSError,
            TimeoutError,
            socket.timeout,
            HTTPError,
            URLError,
            UnicodeDecodeError,
            ValueError,
            json.JSONDecodeError,
        ) as exc:
            raise _MetadataError(
                "metadata-error",
                f"Local running model metadata unavailable: {exc}",
            ) from exc
        models = data.get("models") if isinstance(data, dict) else None
        if not isinstance(models, list):
            raise _MetadataError(
                "metadata-error",
                "Local running model metadata did not include models",
            )
        named_matches = [
            item
            for item in models
            if isinstance(item, dict)
            and str(item.get("name", item.get("model", ""))).strip() == profile.model
        ]
        if not named_matches:
            raise _MetadataError(
                "metadata-error",
                f"Local running model metadata did not include {profile.model!r}",
            )
        digest_matches = [
            item
            for item in named_matches
            if str(item.get("digest", "")).strip() == profile.model_digest
        ]
        if len(digest_matches) != 1:
            raise _MetadataError(
                "digest-mismatch",
                "Local running model digest did not match the resolved Profile",
            )
        match = digest_matches[0]
        size = match.get("size")
        size_vram = match.get("size_vram")
        if (
            not isinstance(size, int)
            or isinstance(size, bool)
            or size <= 0
            or not isinstance(size_vram, int)
            or isinstance(size_vram, bool)
            or size_vram < 0
            or size_vram > size
        ):
            raise _MetadataError(
                "metadata-error",
                "Local running model processor placement is invalid",
            )
        requested_placement = profile.processor_placement
        if requested_placement == "gpu" and size_vram == 0:
            raise _MetadataError(
                "metadata-error",
                "Local running model did not use the requested GPU placement",
            )
        if requested_placement == "cpu" and size_vram != 0:
            raise _MetadataError(
                "metadata-error",
                "Local running model did not use the requested CPU placement",
            )
        return replace(
            profile,
            processor_placement=(
                f"requested={requested_placement};"
                f"gpu_bytes={size_vram};total_bytes={size}"
            ),
        )

    def _stream(
        self,
        *,
        prompt: str,
        profile: LocalInferenceProfile,
        mission_id: str,
        session_id: str,
        turn_kind: str,
        request_id: str,
        admission: dict[str, Any],
        validator: Callable[[str], Any],
        cancellation_requested: Callable[[], bool] | None,
        lease_snapshot: dict[str, Any],
    ) -> LocalInferenceResult:
        payload = {
            "model": profile.model,
            "prompt": prompt,
            "stream": True,
            "raw": True,
            "keep_alive": profile.keep_alive,
            "think": profile.thinking,
            "format": _copy_json(profile.schema),
            "options": {
                "num_ctx": profile.context_budget,
                "num_predict": profile.output_budget,
                **_copy_json(profile.sampling),
            },
        }
        if profile.processor_placement == "gpu":
            payload["options"]["num_gpu"] = -1
        elif profile.processor_placement == "cpu":
            payload["options"]["num_gpu"] = 0
        request = Request(
            f"{self.base_url}/api/generate",
            data=json.dumps(payload, ensure_ascii=True).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        started = self.clock()
        first_token_ms: float | None = None
        chunks: list[str] = []
        output_bytes = 0
        final_data: dict[str, Any] = {}
        stream_complete = False
        stream_line_limit = min(
            16 * 1024 * 1024,
            max(64 * 1024, profile.max_output_bytes * 8 + 4_096),
        )
        try:
            with self.opener(request, timeout=profile.timeout_seconds) as response:
                while True:
                    if cancellation_requested is not None and cancellation_requested():
                        raise _InferenceCancelled()
                    if self.clock() - started > profile.timeout_seconds:
                        raise _InferenceTimeout()
                    line = response.readline(stream_line_limit + 1)
                    if not isinstance(line, (bytes, bytearray)):
                        return self._stream_result(
                            request_id,
                            mission_id,
                            session_id,
                            turn_kind,
                            profile,
                            admission,
                            lease_snapshot,
                            "malformed-stream",
                            "Local model stream line was not bytes.",
                            "".join(chunks),
                            first_token_ms,
                            final_data,
                            output_bytes,
                        )
                    if len(line) > stream_line_limit:
                        return self._stream_result(
                            request_id,
                            mission_id,
                            session_id,
                            turn_kind,
                            profile,
                            admission,
                            lease_snapshot,
                            "oversized-output",
                            "Local model stream line exceeded the bounded byte limit.",
                            "".join(chunks),
                            first_token_ms,
                            final_data,
                            output_bytes,
                        )
                    if not line:
                        break
                    try:
                        item = json.loads(line.decode("utf-8"))
                    except (UnicodeDecodeError, json.JSONDecodeError):
                        return self._stream_result(
                            request_id,
                            mission_id,
                            session_id,
                            turn_kind,
                            profile,
                            admission,
                            lease_snapshot,
                            "malformed-stream",
                            "Local model stream contained invalid JSON.",
                            "".join(chunks),
                            first_token_ms,
                            final_data,
                            output_bytes,
                        )
                    if not isinstance(item, dict):
                        return self._stream_result(
                            request_id,
                            mission_id,
                            session_id,
                            turn_kind,
                            profile,
                            admission,
                            lease_snapshot,
                            "malformed-stream",
                            "Local model stream item was not an object.",
                            "".join(chunks),
                            first_token_ms,
                            final_data,
                            output_bytes,
                        )
                    chunk = item.get("response", "")
                    if chunk is not None and not isinstance(chunk, str):
                        return self._stream_result(
                            request_id,
                            mission_id,
                            session_id,
                            turn_kind,
                            profile,
                            admission,
                            lease_snapshot,
                            "malformed-stream",
                            "Local model stream response chunk was not text.",
                            "".join(chunks),
                            first_token_ms,
                            final_data,
                            output_bytes,
                        )
                    chunk = chunk or ""
                    thinking = item.get("thinking", "")
                    if thinking is not None and not isinstance(thinking, str):
                        return self._stream_result(
                            request_id,
                            mission_id,
                            session_id,
                            turn_kind,
                            profile,
                            admission,
                            lease_snapshot,
                            "malformed-stream",
                            "Local model stream thinking chunk was not text.",
                            "".join(chunks),
                            first_token_ms,
                            final_data,
                            output_bytes,
                        )
                    thinking = thinking or ""
                    if chunk and first_token_ms is None:
                        first_token_ms = round((self.clock() - started) * 1000, 3)
                    output_bytes += len(chunk.encode("utf-8")) + len(
                        thinking.encode("utf-8")
                    )
                    if output_bytes > profile.max_output_bytes:
                        return self._stream_result(
                            request_id,
                            mission_id,
                            session_id,
                            turn_kind,
                            profile,
                            admission,
                            lease_snapshot,
                            "oversized-output",
                            "Local model output exceeded the bounded byte limit.",
                            "".join(chunks),
                            first_token_ms,
                            item,
                            output_bytes,
                        )
                    chunks.append(chunk)
                    final_data = item
                    if item.get("done") is True:
                        stream_complete = True
                        break
        except _InferenceCancelled:
            return self._stream_result(
                request_id,
                mission_id,
                session_id,
                turn_kind,
                profile,
                admission,
                lease_snapshot,
                "cancelled",
                "Local model inference was cancelled before a complete result.",
                "".join(chunks),
                first_token_ms,
                final_data,
                output_bytes,
            )
        except _InferenceTimeout:
            return self._stream_result(
                request_id,
                mission_id,
                session_id,
                turn_kind,
                profile,
                admission,
                lease_snapshot,
                "timed-out",
                "Local model inference exceeded its bounded timeout.",
                "".join(chunks),
                first_token_ms,
                final_data,
                output_bytes,
            )
        except (OSError, TimeoutError, socket.timeout, HTTPError, URLError) as exc:
            return self._stream_result(
                request_id,
                mission_id,
                session_id,
                turn_kind,
                profile,
                admission,
                lease_snapshot,
                "transport-error",
                f"Local model HTTP adapter failed: {exc}",
                "".join(chunks),
                first_token_ms,
                final_data,
                output_bytes,
            )
        if not stream_complete:
            return self._stream_result(
                request_id,
                mission_id,
                session_id,
                turn_kind,
                profile,
                admission,
                lease_snapshot,
                "partial-stream",
                "Local model stream ended before its complete marker.",
                "".join(chunks),
                first_token_ms,
                final_data,
                output_bytes,
            )
        if cancellation_requested is not None and cancellation_requested():
            return self._stream_result(
                request_id,
                mission_id,
                session_id,
                turn_kind,
                profile,
                admission,
                lease_snapshot,
                "cancelled",
                "Local model inference was cancelled before schema validation.",
                "".join(chunks),
                first_token_ms,
                final_data,
                output_bytes,
            )
        output = "".join(chunks)
        output_tokens = final_data.get("eval_count")
        if not isinstance(output_tokens, int) or isinstance(output_tokens, bool):
            output_tokens = estimate_prompt_tokens(output)
        if output_tokens < 0 or output_tokens > profile.output_budget:
            return self._stream_result(
                request_id,
                mission_id,
                session_id,
                turn_kind,
                profile,
                admission,
                lease_snapshot,
                "oversized-output",
                "Local model output exceeded the explicit token budget.",
                output,
                first_token_ms,
                final_data,
                output_bytes,
            )
        try:
            parsed_output = json.loads(output)
            _validate_schema_value(parsed_output, profile.schema)
        except (
            UnicodeDecodeError,
            json.JSONDecodeError,
            _SchemaValidationError,
        ) as exc:
            return self._stream_result(
                request_id,
                mission_id,
                session_id,
                turn_kind,
                profile,
                admission,
                lease_snapshot,
                "malformed-output",
                f"Local model result failed declared schema validation: {exc}",
                output,
                first_token_ms,
                final_data,
                output_bytes,
            )
        try:
            value = validator(output)
        except Exception as exc:
            return self._stream_result(
                request_id,
                mission_id,
                session_id,
                turn_kind,
                profile,
                admission,
                lease_snapshot,
                "malformed-output",
                f"Local model result failed schema validation: {exc}",
                output,
                first_token_ms,
                final_data,
                output_bytes,
            )
        required_metrics = (
            "load_duration",
            "prompt_eval_duration",
            "eval_duration",
            "prompt_eval_count",
            "eval_count",
        )
        if any(
            not isinstance(final_data.get(field_name), int)
            or isinstance(final_data[field_name], bool)
            or final_data[field_name] < 0
            for field_name in required_metrics
        ):
            return self._stream_result(
                request_id,
                mission_id,
                session_id,
                turn_kind,
                profile,
                admission,
                lease_snapshot,
                "malformed-stream",
                "Local model result omitted required usage and timing metrics.",
                output,
                first_token_ms,
                final_data,
                output_bytes,
            )
        return self._stream_result(
            request_id,
            mission_id,
            session_id,
            turn_kind,
            profile,
            admission,
            lease_snapshot,
            "completed",
            "",
            output,
            first_token_ms,
            final_data,
            output_bytes,
            value=value,
        )

    def _stream_result(
        self,
        request_id: str,
        mission_id: str,
        session_id: str,
        turn_kind: str,
        profile: LocalInferenceProfile,
        admission: dict[str, Any],
        lease_snapshot: dict[str, Any],
        outcome: str,
        error: str,
        output: str,
        first_token_ms: float | None,
        final_data: dict[str, Any],
        output_bytes: int,
        *,
        value: Any = None,
    ) -> LocalInferenceResult:
        receipt = self._receipt(
            request_id=request_id,
            mission_id=mission_id,
            session_id=session_id,
            turn_kind=turn_kind,
            profile=profile,
            admission=admission,
            outcome=outcome,
            error=error,
            lease=lease_snapshot,
            first_token_ms=first_token_ms,
            final_data=final_data,
            output_bytes=output_bytes,
        )
        return LocalInferenceResult(
            receipt=receipt,
            value=value if outcome == "completed" else None,
            raw_output=output,
        )

    def _receipt(
        self,
        *,
        request_id: str,
        mission_id: str,
        session_id: str,
        turn_kind: str,
        profile: LocalInferenceProfile,
        admission: dict[str, Any],
        outcome: str,
        error: str = "",
        lease: dict[str, Any] | None = None,
        first_token_ms: float | None = None,
        final_data: dict[str, Any] | None = None,
        output_bytes: int = 0,
    ) -> dict[str, Any]:
        final_data = final_data or {}
        timings = {
            "load_ms": self._duration_ms(final_data.get("load_duration")),
            "prompt_evaluation_ms": self._duration_ms(
                final_data.get("prompt_eval_duration")
            ),
            "first_token_ms": first_token_ms,
            "decoding_ms": self._duration_ms(final_data.get("eval_duration")),
        }
        usage = {
            "prompt_tokens": final_data.get("prompt_eval_count"),
            "output_tokens": (
                final_data.get("eval_count")
                if isinstance(final_data.get("eval_count"), int)
                and not isinstance(final_data.get("eval_count"), bool)
                else math.ceil(output_bytes / 4)
                if output_bytes
                else 0
            ),
            "output_bytes": output_bytes,
        }
        receipt: dict[str, Any] = {
            "schema_version": INFERENCE_SCHEMA_VERSION,
            "request_id": request_id,
            "mission_id": mission_id,
            "session_id": session_id,
            "turn_kind": turn_kind,
            "recorded_at": _utc_now(),
            "profile": profile.to_dict(),
            "admission": _copy_json(admission),
            "outcome": outcome,
            "authoritative": outcome == "completed",
            "timings": timings,
            "usage": usage,
            "lease": _copy_json(lease or {}),
        }
        if error:
            receipt["error"] = error
        return receipt

    @staticmethod
    def _duration_ms(value: Any) -> float | None:
        if not isinstance(value, (int, float)) or isinstance(value, bool) or value < 0:
            return None
        return round(float(value) / 1_000_000, 3)


class _MetadataError(RuntimeError):
    def __init__(self, outcome: str, message: str):
        super().__init__(message)
        self.outcome = outcome
