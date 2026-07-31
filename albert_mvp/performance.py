from __future__ import annotations

from dataclasses import dataclass
from functools import wraps
import json
import os
from pathlib import Path
import stat
import time
from typing import Any, Callable, ParamSpec, TypeVar


_ENVIRONMENT_FIELDS = {
    "jsonl_path": "ALFREDO_MEASUREMENT_JSONL",
    "run_id": "ALFREDO_MEASUREMENT_RUN_ID",
    "sample_id": "ALFREDO_MEASUREMENT_SAMPLE_ID",
    "cohort_id": "ALFREDO_MEASUREMENT_COHORT_ID",
    "correlation_id": "ALFREDO_MEASUREMENT_CORRELATION_ID",
    "fixture_id": "ALFREDO_MEASUREMENT_FIXTURE_ID",
    "fixture_sha256": "ALFREDO_MEASUREMENT_FIXTURE_SHA256",
    "source_sha256": "ALFREDO_MEASUREMENT_SOURCE_SHA256",
    "artifact_sha256": "ALFREDO_MEASUREMENT_ARTIFACT_SHA256",
    "variant": "ALFREDO_MEASUREMENT_VARIANT",
    "workflow": "ALFREDO_MEASUREMENT_WORKFLOW",
    "mode": "ALFREDO_MEASUREMENT_MODE",
}
_CONTROL_PATH = "ALFREDO_MEASUREMENT_CONTROL_PATH"
_CONTROL_FIELDS = set(_ENVIRONMENT_FIELDS) | {"desktop_pid", "desktop_session_id"}
_SHA256_LENGTH = 64
_STAGES = {f"S{index}" for index in range(10)} | {
    f"R{index}" for index in range(7)
}
_P = ParamSpec("_P")
_R = TypeVar("_R")


@dataclass(frozen=True)
class PerformanceIdentity:
    jsonl_path: Path
    run_id: str
    sample_id: str
    cohort_id: str
    correlation_id: str
    fixture_id: str
    fixture_sha256: str
    source_sha256: str
    artifact_sha256: str
    variant: str
    workflow: str
    mode: str
    desktop_pid: int | None = None
    desktop_session_id: str | None = None

    @classmethod
    def from_environment(
        cls, environment: dict[str, str] | os._Environ[str] = os.environ
    ) -> PerformanceIdentity | None:
        control_value = environment.get(_CONTROL_PATH, "")
        legacy_present = [
            variable
            for variable in _ENVIRONMENT_FIELDS.values()
            if isinstance(environment.get(variable), str)
            and bool(environment.get(variable, "").strip())
        ]
        if isinstance(control_value, str) and control_value.strip():
            if legacy_present:
                raise ValueError(
                    "ALFREDO_MEASUREMENT_CONTROL_PATH must not be combined with legacy measurement identity"
                )
            control_path = Path(control_value)
            if not control_path.is_absolute():
                raise ValueError("ALFREDO_MEASUREMENT_CONTROL_PATH must be absolute")
            control_path.parent.resolve(strict=True)
            try:
                mode = control_path.lstat().st_mode
            except FileNotFoundError:
                return None
            if stat.S_ISLNK(mode) or not stat.S_ISREG(mode):
                raise ValueError(
                    "ALFREDO_MEASUREMENT_CONTROL_PATH must be a regular non-symlink file"
                )
            try:
                payload = json.loads(control_path.read_text(encoding="utf-8"))
            except (OSError, UnicodeError, json.JSONDecodeError) as error:
                raise ValueError(f"measurement control file is invalid: {error}") from error
            if not isinstance(payload, dict):
                raise ValueError("measurement control file must contain one JSON object")
            unknown = sorted(set(payload) - _CONTROL_FIELDS)
            if unknown:
                raise ValueError(
                    f"measurement control file has unknown fields: {', '.join(unknown)}"
                )
            return cls._from_values(payload)

        values: dict[str, str] = {}
        missing: list[str] = []
        for field, variable in _ENVIRONMENT_FIELDS.items():
            value = environment.get(variable, "")
            if not isinstance(value, str) or not value.strip():
                missing.append(variable)
                continue
            values[field] = value
        if len(missing) == len(_ENVIRONMENT_FIELDS):
            return None
        if missing:
            raise ValueError(
                f"measurement environment is incomplete: missing {', '.join(missing)}"
            )
        return cls._from_values(values)

    @classmethod
    def _from_values(cls, raw_values: dict[str, Any]) -> PerformanceIdentity:
        missing = [
            field
            for field in _ENVIRONMENT_FIELDS
            if not isinstance(raw_values.get(field), str)
            or not raw_values[field].strip()
        ]
        if missing:
            raise ValueError(
                f"measurement control identity is incomplete: missing {', '.join(missing)}"
            )
        values = {field: raw_values[field] for field in _ENVIRONMENT_FIELDS}
        path = Path(values.pop("jsonl_path"))
        if not path.is_absolute():
            raise ValueError("ALFREDO_MEASUREMENT_JSONL must be absolute")
        path.parent.resolve(strict=True)
        if path.exists():
            mode = path.lstat().st_mode
            if stat.S_ISLNK(mode) or not stat.S_ISREG(mode):
                raise ValueError(
                    "ALFREDO_MEASUREMENT_JSONL must be a regular non-symlink file"
                )
        for field in ("fixture_sha256", "source_sha256", "artifact_sha256"):
            value = values[field]
            if (
                len(value) != _SHA256_LENGTH
                or value.lower() != value
                or any(character not in "0123456789abcdef" for character in value)
            ):
                raise ValueError(f"{field} must be a lowercase SHA-256")
        if values["mode"] not in {"process-cold", "process-warm"}:
            raise ValueError("measurement mode must be process-cold or process-warm")
        desktop_pid = raw_values.get("desktop_pid")
        desktop_session_id = raw_values.get("desktop_session_id")
        if desktop_pid is not None or desktop_session_id is not None:
            try:
                desktop_pid = int(desktop_pid)
            except (TypeError, ValueError) as error:
                raise ValueError("desktop_pid must be a positive integer") from error
            if desktop_pid <= 0:
                raise ValueError("desktop_pid must be a positive integer")
            if not isinstance(desktop_session_id, str) or not desktop_session_id.strip():
                raise ValueError("desktop_session_id must be a non-empty string")
        return cls(
            jsonl_path=path,
            desktop_pid=desktop_pid,
            desktop_session_id=desktop_session_id,
            **values,
        )


class PerformanceRecorder:
    def __init__(
        self,
        identity: PerformanceIdentity,
        *,
        source: str,
        monotonic_now_ns: Callable[[], int] = time.perf_counter_ns,
    ):
        if not source.strip():
            raise ValueError("measurement source must not be empty")
        self._identity = identity
        self._source = source
        self._clock_id = f"{source}:{os.getpid()}"
        self._monotonic_now_ns = monotonic_now_ns
        self._last_tick: int | None = None

    @classmethod
    def from_environment(
        cls,
        *,
        source: str,
        environment: dict[str, str] | os._Environ[str] = os.environ,
    ) -> PerformanceRecorder | None:
        identity = PerformanceIdentity.from_environment(environment)
        return cls(identity, source=source) if identity is not None else None

    @property
    def workflow(self) -> str:
        return self._identity.workflow

    def mark(
        self,
        stage: str,
        boundary: str,
        *,
        detail: dict[str, Any] | None = None,
    ) -> None:
        if stage not in _STAGES:
            raise ValueError(f"unknown measurement stage: {stage}")
        if boundary not in {"start", "end"}:
            raise ValueError(f"unknown measurement boundary: {boundary}")
        tick = self._monotonic_now_ns()
        if not isinstance(tick, int) or tick < 0:
            raise ValueError("monotonic clock must return a non-negative integer")
        if self._last_tick is not None and tick < self._last_tick:
            raise ValueError("monotonic clock moved backwards")
        self._last_tick = tick
        identity = self._identity
        record = {
            "schema_version": 1,
            "record_type": "stage-mark",
            "run_id": identity.run_id,
            "sample_id": identity.sample_id,
            "cohort_id": identity.cohort_id,
            "correlation_id": identity.correlation_id,
            "fixture_id": identity.fixture_id,
            "fixture_sha256": identity.fixture_sha256,
            "source_sha256": identity.source_sha256,
            "artifact_sha256": identity.artifact_sha256,
            "variant": identity.variant,
            "workflow": identity.workflow,
            "mode": identity.mode,
            **(
                {
                    "desktop_pid": identity.desktop_pid,
                    "desktop_session_id": identity.desktop_session_id,
                }
                if identity.desktop_pid is not None
                else {}
            ),
            "source": self._source,
            "clock_id": self._clock_id,
            "stage": stage,
            "boundary": boundary,
            "monotonic_ns": str(tick),
            "detail": detail or {},
        }
        encoded = (json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n").encode(
            "utf-8"
        )
        if len(encoded) > 16_384:
            raise ValueError("measurement stage mark exceeds 16 KiB")
        descriptor = os.open(
            identity.jsonl_path,
            os.O_APPEND | os.O_CREAT | os.O_WRONLY | os.O_CLOEXEC,
            0o600,
        )
        try:
            written = os.write(descriptor, encoded)
            if written != len(encoded):
                raise OSError("measurement stage mark write was incomplete")
        finally:
            os.close(descriptor)


def measured_stage(
    stage: str,
    *,
    workflows: set[str],
    source: str = "python-authority",
) -> Callable[[Callable[_P, _R]], Callable[_P, _R]]:
    if stage not in _STAGES:
        raise ValueError(f"unknown measurement stage: {stage}")

    def decorate(method: Callable[_P, _R]) -> Callable[_P, _R]:
        @wraps(method)
        def measured(*args: _P.args, **kwargs: _P.kwargs) -> _R:
            recorder = PerformanceRecorder.from_environment(source=source)
            if recorder is None or recorder.workflow not in workflows:
                return method(*args, **kwargs)
            recorder.mark(stage, "start", detail={"outcome": "pass"})
            try:
                result = method(*args, **kwargs)
            except BaseException:
                recorder.mark(stage, "end", detail={"outcome": "fail"})
                raise
            recorder.mark(stage, "end", detail={"outcome": "pass"})
            return result

        return measured

    return decorate
