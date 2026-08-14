"""Versioned request/receipt boundary for bounded host effects.

The Orchestrator remains responsible for authorization and canonical Mission
state.  This module owns the stable shape used to hand one already-authorized
argv effect to a provider, and the crash-safe intent ledger that prevents a
transport retry from launching the same external effect twice.

Raw stdout/stderr are deliberately transient.  The durable ledger retains
their byte counts and digests, together with enough process and authority
identity to explain an uncertain effect and require reconciliation.
"""

from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass, field, replace
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from typing import Any, Callable, Literal, Mapping, Sequence

try:
    import fcntl
except ImportError:  # pragma: no cover - exercised only on unsupported hosts
    fcntl = None


EXECUTION_SCHEMA_VERSION = 1
_MAX_REQUEST_ID_BYTES = 256
_MAX_ARGUMENT_BYTES = 128_000
_MAX_INPUT_BYTES = 96_000
_MAX_OUTPUT_BYTES = 8_000_000
_MAX_TIMEOUT_SECONDS = 3_600.0
_MAX_ENVIRONMENT_ENTRIES = 128
_MAX_ENVIRONMENT_VALUE_BYTES = 16 * 1024
_MAX_ENVIRONMENT_TOTAL_BYTES = 64 * 1024
_MAX_PATH_ENTRIES = 256
_MAX_ADDRESS_SPACE_BYTES = 64 * 1024 * 1024 * 1024
_MAX_FILE_SIZE_BYTES = 16 * 1024 * 1024 * 1024
_MAX_OPEN_FILE_LIMIT = 65_536
_MAX_PROCESS_COUNT_LIMIT = 4_096
_IMPLICIT_READONLY_ROOTS = {
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/etc",
}
_PROTECTED_WRITABLE_ROOTS = {
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/etc",
    "/dev",
    "/proc",
}
_IDENTITY_PATTERN = re.compile(r"^[A-Za-z0-9._:/=-]+$")
_ENVIRONMENT_KEY_PATTERN = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_EXECUTION_ENVIRONMENT_ALLOWLIST = {
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
    "PIP_CACHE_DIR",
    "PYTHONDONTWRITEBYTECODE",
    "npm_config_cache",
    "ALBERT_SESSION_ID",
    "ALBERT_TASK_PACKET",
}

ExecutionEffect = Literal["local-agent", "shell"]
ExecutionStatus = Literal[
    "executing",
    "completed",
    "failed",
    "cancelled",
    "timed-out",
    "output-limit",
    "start-failed",
    "outcome-unknown",
]


class ExecutionContractError(ValueError):
    """Raised when a request or receipt cannot satisfy the stable contract."""


class ExecutionReplayConflict(ExecutionContractError):
    """Raised when one request identity is reused for a different effect."""


def _utc_now() -> str:
    return (
        datetime.now(timezone.utc)
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z")
    )


def _sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _json_bytes(value: object) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=True,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def _canonical_path(value: str, *, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ExecutionContractError(f"{label} must be a non-empty path")
    path = Path(value)
    if not path.is_absolute() or str(path.resolve(strict=False)) != value:
        raise ExecutionContractError(f"{label} must be canonical and absolute")
    return value


def _validate_identity(value: str, *, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ExecutionContractError(f"{label} must be named")
    if len(value.encode("utf-8")) > _MAX_REQUEST_ID_BYTES:
        raise ExecutionContractError(f"{label} is too long")
    if not _IDENTITY_PATTERN.fullmatch(value):
        raise ExecutionContractError(f"{label} contains unsupported characters")
    return value


def _validate_timestamp(value: str, *, label: str) -> str:
    if not isinstance(value, str) or not value.endswith("Z"):
        raise ExecutionContractError(f"execution receipt {label} is invalid")
    try:
        parsed = datetime.fromisoformat(value.removesuffix("Z") + "+00:00")
    except ValueError as exc:
        raise ExecutionContractError(f"execution receipt {label} is invalid") from exc
    if parsed.utcoffset() != timezone.utc.utcoffset(parsed):
        raise ExecutionContractError(f"execution receipt {label} is invalid")
    return value


def _validate_paths(
    values: Sequence[str], *, label: str, absolute: bool
) -> tuple[str, ...]:
    if len(values) > _MAX_PATH_ENTRIES:
        raise ExecutionContractError(f"{label} exceeds the bounded path count")
    result: list[str] = []
    for value in values:
        if not isinstance(value, str) or not value.strip() or "\0" in value:
            raise ExecutionContractError(f"{label} contains an invalid path")
        if absolute:
            result.append(_canonical_path(value, label=label))
        else:
            relative = Path(value)
            if relative.is_absolute() or ".." in relative.parts:
                raise ExecutionContractError(f"{label} must contain relative paths")
            result.append(value)
    if len(result) != len(set(result)):
        raise ExecutionContractError(f"{label} must be unique")
    return tuple(result)


def _is_protected_writable_path(value: str) -> bool:
    path = Path(value)
    try:
        resolved = path.resolve(strict=False)
    except OSError:
        resolved = path
    temporary_roots = (Path("/tmp"), Path(tempfile.gettempdir()))
    if any(
        path == root or resolved == root.resolve(strict=False)
        for root in temporary_roots
    ):
        return True
    return any(
        resolved == root
        or resolved.is_relative_to(root)
        for root in (
            Path(item).resolve(strict=False) for item in _PROTECTED_WRITABLE_ROOTS
        )
    )


def _is_under_private_tmp(path: Path) -> bool:
    """Match OS temporary roots, including macOS's /private/tmp alias."""

    try:
        resolved = path.resolve(strict=False)
    except OSError:
        resolved = path
    for root in (Path("/tmp"), Path(tempfile.gettempdir())):
        if path.is_relative_to(root) or resolved.is_relative_to(
            root.resolve(strict=False)
        ):
            return True
    return False


def _is_regular_non_symlink(path: Path) -> bool:
    try:
        return stat.S_ISREG(path.lstat().st_mode)
    except OSError:
        return False


@dataclass(frozen=True)
class ExecutionLimits:
    """Provider-enforced timeout, output, and inherited resource limits."""

    timeout_seconds: float
    output_limit_bytes: int
    address_space_bytes: int = 8 * 1024 * 1024 * 1024
    file_size_bytes: int = 2 * 1024 * 1024 * 1024
    open_file_limit: int = 1_024
    process_count_limit: int = 256
    descendant_grace_seconds: float = 1.0

    def __post_init__(self) -> None:
        if (
            not isinstance(self.timeout_seconds, (int, float))
            or isinstance(self.timeout_seconds, bool)
            or not math.isfinite(float(self.timeout_seconds))
            or self.timeout_seconds <= 0
            or self.timeout_seconds > _MAX_TIMEOUT_SECONDS
        ):
            raise ExecutionContractError(
                "execution timeout is outside the bounded range"
            )
        if (
            not isinstance(self.output_limit_bytes, int)
            or isinstance(self.output_limit_bytes, bool)
            or self.output_limit_bytes <= 0
            or self.output_limit_bytes > _MAX_OUTPUT_BYTES
        ):
            raise ExecutionContractError(
                "execution output limit is outside the bounded range"
            )
        for name in (
            "address_space_bytes",
            "file_size_bytes",
            "open_file_limit",
            "process_count_limit",
        ):
            value = getattr(self, name)
            if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
                raise ExecutionContractError(f"execution {name} must be positive")
        if self.address_space_bytes > _MAX_ADDRESS_SPACE_BYTES:
            raise ExecutionContractError("execution address space limit is too large")
        if self.file_size_bytes > _MAX_FILE_SIZE_BYTES:
            raise ExecutionContractError("execution file size limit is too large")
        if self.open_file_limit > _MAX_OPEN_FILE_LIMIT:
            raise ExecutionContractError("execution open file limit is too large")
        if self.process_count_limit > _MAX_PROCESS_COUNT_LIMIT:
            raise ExecutionContractError("execution process count limit is too large")
        if (
            not isinstance(self.descendant_grace_seconds, (int, float))
            or isinstance(self.descendant_grace_seconds, bool)
            or not math.isfinite(float(self.descendant_grace_seconds))
            or self.descendant_grace_seconds < 0
            or self.descendant_grace_seconds > 60
        ):
            raise ExecutionContractError(
                "execution descendant grace is outside the bounded range"
            )

    def to_dict(self) -> dict[str, Any]:
        return {
            "timeout_seconds": float(self.timeout_seconds),
            "output_limit_bytes": self.output_limit_bytes,
            "address_space_bytes": self.address_space_bytes,
            "file_size_bytes": self.file_size_bytes,
            "open_file_limit": self.open_file_limit,
            "process_count_limit": self.process_count_limit,
            "descendant_grace_seconds": float(self.descendant_grace_seconds),
        }

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any]) -> "ExecutionLimits":
        if not isinstance(payload, Mapping):
            raise ExecutionContractError("execution limits must be an object")
        return cls(
            timeout_seconds=payload["timeout_seconds"],
            output_limit_bytes=payload["output_limit_bytes"],
            address_space_bytes=payload.get(
                "address_space_bytes", 8 * 1024 * 1024 * 1024
            ),
            file_size_bytes=payload.get("file_size_bytes", 2 * 1024 * 1024 * 1024),
            open_file_limit=payload.get("open_file_limit", 1_024),
            process_count_limit=payload.get("process_count_limit", 256),
            descendant_grace_seconds=payload.get("descendant_grace_seconds", 1.0),
        )


@dataclass(frozen=True)
class ExecutionSandbox:
    """The exact filesystem boundary selected by the authority layer."""

    mode: Literal["bubblewrap", "none"]
    readable_roots: tuple[str, ...] = ()
    writable_roots: tuple[str, ...] = ()
    readonly_bindings: tuple[tuple[str, str], ...] = ()

    def __post_init__(self) -> None:
        if self.mode not in {"bubblewrap", "none"}:
            raise ExecutionContractError("execution sandbox mode is invalid")
        _validate_paths(
            self.readable_roots, label="execution readable roots", absolute=True
        )
        _validate_paths(
            self.writable_roots, label="execution writable roots", absolute=True
        )
        if any(_is_protected_writable_path(path) for path in self.writable_roots):
            raise ExecutionContractError(
                "execution writable roots cannot override protected sandbox roots"
            )
        if len(self.readonly_bindings) > _MAX_PATH_ENTRIES:
            raise ExecutionContractError("execution readonly bindings are too numerous")
        for binding in self.readonly_bindings:
            if not isinstance(binding, (tuple, list)) or len(binding) != 2:
                raise ExecutionContractError("execution readonly binding is invalid")
            _canonical_path(binding[0], label="execution readonly source")
            _canonical_path(binding[1], label="execution readonly destination")

    def to_dict(self) -> dict[str, Any]:
        return {
            "mode": self.mode,
            "readable_roots": list(self.readable_roots),
            "writable_roots": list(self.writable_roots),
            "readonly_bindings": [list(item) for item in self.readonly_bindings],
        }

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any]) -> "ExecutionSandbox":
        if not isinstance(payload, Mapping):
            raise ExecutionContractError("execution sandbox must be an object")
        return cls(
            mode=payload["mode"],
            readable_roots=tuple(payload.get("readable_roots", [])),
            writable_roots=tuple(payload.get("writable_roots", [])),
            readonly_bindings=tuple(
                tuple(item) for item in payload.get("readonly_bindings", [])
            ),
        )


def _validate_prepared_bubblewrap_argv(
    argv: Sequence[str],
    sandbox: ExecutionSandbox,
    working_directory: str,
    limits: ExecutionLimits,
    environment: Sequence[tuple[str, str]] = (),
) -> None:
    if sandbox.mode == "none" and os.name != "posix":
        return
    if not argv or not Path(argv[0]).is_absolute() or Path(argv[0]).name != "bwrap":
        raise ExecutionContractError(
            "execution provider requires a prepared Bubblewrap argv"
        )
    if sandbox.mode != "bubblewrap":
        raise ExecutionContractError("execution provider requires Bubblewrap")
    trusted_roots = tuple(
        Path(value).resolve(strict=False)
        for value in ("/usr/bin", "/usr/sbin", "/bin", "/sbin")
    )
    bubblewrap_path = Path(argv[0]).resolve(strict=False)
    if not any(
        bubblewrap_path == root or bubblewrap_path.is_relative_to(root)
        for root in trusted_roots
    ):
        raise ExecutionContractError("execution Bubblewrap executable is not trusted")

    try:
        separator = argv.index("--")
    except ValueError as exc:
        raise ExecutionContractError(
            "execution Bubblewrap argv is missing its separator"
        ) from exc
    if separator <= 1 or separator == len(argv) - 1:
        raise ExecutionContractError("execution Bubblewrap argv is incomplete")

    prefix = list(argv[1:separator])
    command = list(argv[separator + 1 :])
    required_flags = {
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
    }
    if any(prefix.count(flag) != 1 for flag in required_flags):
        raise ExecutionContractError(
            "execution Bubblewrap argv must retain process supervision and isolation"
        )

    ro_bindings: list[tuple[str, str]] = []
    rw_bindings: list[tuple[str, str]] = []
    tmpfs_targets: list[str] = []
    dev_targets: list[str] = []
    proc_targets: list[str] = []
    chdir_targets: list[str] = []
    directory_targets: list[str] = []
    flag_tokens = required_flags
    pair_tokens = {
        "--ro-bind",
        "--bind",
        "--tmpfs",
        "--dev",
        "--proc",
        "--chdir",
        "--dir",
    }
    index = 0
    while index < len(prefix):
        token = prefix[index]
        if token in flag_tokens:
            index += 1
            continue
        if token not in pair_tokens or index + 1 >= len(prefix):
            raise ExecutionContractError(
                "execution Bubblewrap argv contains an unsupported option"
            )
        if token in {"--ro-bind", "--bind"}:
            if index + 2 >= len(prefix):
                raise ExecutionContractError("execution Bubblewrap mount is incomplete")
            source, destination = prefix[index + 1 : index + 3]
            if token == "--ro-bind":
                ro_bindings.append((source, destination))
            else:
                rw_bindings.append((source, destination))
            index += 3
            continue
        value = prefix[index + 1]
        if token == "--tmpfs":
            tmpfs_targets.append(value)
        elif token == "--dev":
            dev_targets.append(value)
        elif token == "--proc":
            proc_targets.append(value)
        elif token == "--chdir":
            chdir_targets.append(value)
        else:
            directory_targets.append(value)
        index += 2

    if tmpfs_targets.count("/") != 1 or tmpfs_targets.count("/tmp") != 1:
        raise ExecutionContractError(
            "execution Bubblewrap root and temporary mounts are invalid"
        )
    if dev_targets != ["/dev"] or proc_targets != ["/proc"]:
        raise ExecutionContractError("execution Bubblewrap device mounts are invalid")
    if chdir_targets != [working_directory]:
        raise ExecutionContractError(
            "execution Bubblewrap working directory is invalid"
        )
    if any(not Path(value).is_absolute() for value in directory_targets):
        raise ExecutionContractError("execution Bubblewrap directory mount is invalid")
    if any(source == "/" and destination == "/" for source, destination in rw_bindings):
        raise ExecutionContractError(
            "execution Bubblewrap host root cannot be writable"
        )
    if any(
        _is_protected_writable_path(destination)
        for _source, destination in rw_bindings
    ):
        raise ExecutionContractError(
            "execution Bubblewrap writable mount overrides a protected sandbox root"
        )
    if any(source == "/" or destination == "/" for source, destination in ro_bindings):
        raise ExecutionContractError("execution Bubblewrap host root cannot be mounted")

    writable = set(sandbox.writable_roots)
    readable = set(sandbox.readable_roots) - writable
    expected_rw = {(path, path) for path in writable}
    expected_ro = {(path, path) for path in readable}
    expected_ro.update(sandbox.readonly_bindings)
    if not expected_rw.issubset(set(rw_bindings)):
        raise ExecutionContractError(
            "execution Bubblewrap writable boundary is incomplete"
        )
    if expected_ro - set(ro_bindings):
        raise ExecutionContractError(
            "execution Bubblewrap readonly boundary is incomplete"
        )
    if any(binding not in expected_rw for binding in rw_bindings):
        raise ExecutionContractError(
            "execution Bubblewrap contains an undeclared writable mount"
        )
    if any(destination in writable for _source, destination in ro_bindings):
        raise ExecutionContractError(
            "execution Bubblewrap writable path is also readonly"
        )

    implicit_command = (
        list(command[6:]) if len(command) > 6 and command[5] == "--" else []
    )
    environment_path = dict(environment).get("PATH", os.defpath)
    implicit_executable_paths: set[Path] = set()
    if implicit_command:
        command_executable = Path(implicit_command[0])
        resolved_executable: str | None = (
            str(command_executable)
            if command_executable.is_absolute()
            else shutil.which(implicit_command[0], path=environment_path)
        )
        if resolved_executable:
            executable_entry = Path(resolved_executable)
            if _is_under_private_tmp(executable_entry):
                if executable_entry.is_symlink():
                    raise ExecutionContractError(
                        "execution /tmp executable must not be a symlink"
                    )
                try:
                    resolved_path = executable_entry.resolve(strict=True)
                except OSError as exc:
                    raise ExecutionContractError(
                        "execution /tmp executable cannot be resolved"
                    ) from exc
                if not _is_under_private_tmp(resolved_path):
                    raise ExecutionContractError(
                        "execution /tmp executable escapes its private root"
                    )
                implicit_executable_paths.add(resolved_path)
            else:
                implicit_executable_paths.add(executable_entry.resolve(strict=False))
    implicit_script_path: Path | None = None
    if implicit_command and (
        Path(implicit_command[0]).name.casefold().startswith("python")
        or Path(implicit_command[0]).name.casefold() in {"node", "bash", "sh", "ruby"}
    ):
        for argument in implicit_command[1:]:
            if argument in {"-c", "-m", "-e"}:
                break
            if argument.startswith("-"):
                continue
            candidate = Path(argument)
            if candidate.is_absolute() and _is_under_private_tmp(candidate):
                if candidate.is_symlink():
                    raise ExecutionContractError(
                        "execution /tmp interpreter script must not be a symlink"
                    )
                if candidate.exists():
                    if not _is_regular_non_symlink(candidate):
                        raise ExecutionContractError(
                            "execution /tmp interpreter script must be a regular file"
                        )
                    try:
                        resolved_script = candidate.resolve(strict=True)
                    except OSError as exc:
                        raise ExecutionContractError(
                            "execution /tmp interpreter script cannot be resolved"
                        ) from exc
                    if not _is_under_private_tmp(resolved_script):
                        raise ExecutionContractError(
                            "execution /tmp interpreter script escapes its private root"
                        )
                    implicit_script_path = resolved_script
            break

    def is_allowed_implicit_binding(binding: tuple[str, str]) -> bool:
        source, destination = binding
        if source == destination and source in _IMPLICIT_READONLY_ROOTS:
            return True
        source_path = Path(source)
        destination_path = Path(destination)
        if not _is_regular_non_symlink(source_path):
            return False
        if destination_path.is_symlink() and _is_under_private_tmp(destination_path):
            return False
        try:
            source_resolved = source_path.resolve(strict=True)
            destination_resolved = destination_path.resolve(strict=True)
        except OSError:
            return False
        if source_resolved != destination_resolved:
            return False
        if destination_resolved in implicit_executable_paths:
            return os.access(source_path, os.X_OK)
        return (
            implicit_script_path is not None
            and destination_resolved == implicit_script_path
        )

    undeclared_readonly = set(ro_bindings) - expected_ro
    if any(not is_allowed_implicit_binding(binding) for binding in undeclared_readonly):
        raise ExecutionContractError(
            "execution Bubblewrap contains an undeclared readonly mount"
        )

    if Path(command[0]).name != "prlimit" or len(command) < 7:
        raise ExecutionContractError(
            "execution Bubblewrap resource boundary is missing"
        )
    prlimit_path = Path(command[0]).resolve(strict=False)
    if not any(
        prlimit_path == root or prlimit_path.is_relative_to(root)
        for root in trusted_roots
    ):
        raise ExecutionContractError("execution prlimit executable is not trusted")
    expected_limits = [
        f"--as={limits.address_space_bytes}",
        f"--fsize={limits.file_size_bytes}",
        f"--nofile={limits.open_file_limit}",
        f"--nproc={limits.process_count_limit}",
    ]
    if command[1:5] != expected_limits or command[5] != "--":
        raise ExecutionContractError(
            "execution Bubblewrap resource boundary is invalid"
        )


@dataclass(frozen=True)
class LocalAgentExecutionAuthority:
    mission_id: str
    session_id: str
    session_revision: int
    runner_operation_id: str
    worktree_identity: str
    allowed_paths: tuple[str, ...] = ()
    kind: Literal["local-agent"] = field(default="local-agent", init=False)

    def __post_init__(self) -> None:
        _validate_identity(self.mission_id, label="Local Agent Mission identity")
        _validate_identity(self.session_id, label="Local Agent session identity")
        _validate_identity(
            self.runner_operation_id, label="Local Agent runner operation"
        )
        _validate_identity(
            self.worktree_identity, label="Local Agent Worktree Identity"
        )
        if (
            not isinstance(self.session_revision, int)
            or isinstance(self.session_revision, bool)
            or self.session_revision < 0
        ):
            raise ExecutionContractError("Local Agent session revision is invalid")
        _validate_paths(
            self.allowed_paths, label="Local Agent allowed paths", absolute=False
        )

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": self.kind,
            "mission_id": self.mission_id,
            "session_id": self.session_id,
            "session_revision": self.session_revision,
            "runner_operation_id": self.runner_operation_id,
            "worktree_identity": self.worktree_identity,
            "allowed_paths": list(self.allowed_paths),
        }

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any]) -> "LocalAgentExecutionAuthority":
        if payload.get("kind") != "local-agent":
            raise ExecutionContractError("Local Agent authority kind is invalid")
        return cls(
            mission_id=payload["mission_id"],
            session_id=payload["session_id"],
            session_revision=payload["session_revision"],
            runner_operation_id=payload["runner_operation_id"],
            worktree_identity=payload["worktree_identity"],
            allowed_paths=tuple(payload.get("allowed_paths", [])),
        )


@dataclass(frozen=True)
class ShellExecutionAuthority:
    mission_id: str
    command_id: str
    correlation_id: str
    command: str
    classification: Literal["auto-allowed", "frontier-approvable", "human-required"]
    requester: str
    working_directory: str
    requested_paths: tuple[str, ...]
    access_level: Literal["read", "write"]
    approval_actor: str = ""
    kind: Literal["shell"] = field(default="shell", init=False)

    def __post_init__(self) -> None:
        _validate_identity(self.mission_id, label="Shell Mission identity")
        _validate_identity(self.command_id, label="Shell command identity")
        _validate_identity(self.correlation_id, label="Shell correlation identity")
        if (
            not isinstance(self.command, str)
            or not self.command.strip()
            or "\0" in self.command
        ):
            raise ExecutionContractError("Shell command text is invalid")
        if self.classification not in {
            "auto-allowed",
            "frontier-approvable",
            "human-required",
        }:
            raise ExecutionContractError("Shell command classification is invalid")
        if not isinstance(self.requester, str) or not self.requester.strip():
            raise ExecutionContractError("Shell requester is invalid")
        _canonical_path(self.working_directory, label="Shell working directory")
        _validate_paths(
            self.requested_paths, label="Shell requested paths", absolute=True
        )
        if self.access_level not in {"read", "write"}:
            raise ExecutionContractError("Shell access level is invalid")
        if not isinstance(self.approval_actor, str):
            raise ExecutionContractError("Shell approval actor is invalid")

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": self.kind,
            "mission_id": self.mission_id,
            "command_id": self.command_id,
            "correlation_id": self.correlation_id,
            "command": self.command,
            "classification": self.classification,
            "requester": self.requester,
            "working_directory": self.working_directory,
            "requested_paths": list(self.requested_paths),
            "access_level": self.access_level,
            "approval_actor": self.approval_actor,
        }

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any]) -> "ShellExecutionAuthority":
        if payload.get("kind") != "shell":
            raise ExecutionContractError("Shell authority kind is invalid")
        return cls(
            mission_id=payload["mission_id"],
            command_id=payload["command_id"],
            correlation_id=payload["correlation_id"],
            command=payload["command"],
            classification=payload["classification"],
            requester=payload["requester"],
            working_directory=payload["working_directory"],
            requested_paths=tuple(payload.get("requested_paths", [])),
            access_level=payload["access_level"],
            approval_actor=payload.get("approval_actor", ""),
        )


ExecutionAuthority = LocalAgentExecutionAuthority | ShellExecutionAuthority


@dataclass(frozen=True)
class ExecutionRequest:
    """One exact already-authorized host effect."""

    request_id: str
    effect: ExecutionEffect
    argv: tuple[str, ...]
    working_directory: str
    authority: ExecutionAuthority
    limits: ExecutionLimits
    sandbox: ExecutionSandbox
    environment: tuple[tuple[str, str], ...] = ()
    input_text: str | None = None
    input_sha256: str | None = None
    shell: bool = False
    schema_version: int = EXECUTION_SCHEMA_VERSION

    def __post_init__(self) -> None:
        self.validate()

    def validate(self) -> None:
        if self.schema_version != EXECUTION_SCHEMA_VERSION:
            raise ExecutionContractError("unsupported execution request schema")
        _validate_identity(self.request_id, label="execution request identity")
        if self.effect not in {"local-agent", "shell"}:
            raise ExecutionContractError("execution effect kind is invalid")
        if self.shell:
            raise ExecutionContractError(
                "shell execution is not allowed; argv is required"
            )
        if not isinstance(self.argv, (tuple, list)) or not self.argv:
            raise ExecutionContractError("execution argv must not be empty")
        for argument in self.argv:
            if not isinstance(argument, str) or not argument or "\0" in argument:
                raise ExecutionContractError(
                    "execution argv contains an invalid argument"
                )
            if len(argument.encode("utf-8")) > _MAX_ARGUMENT_BYTES:
                raise ExecutionContractError("execution argv argument is too long")
        _canonical_path(self.working_directory, label="execution working directory")
        if not isinstance(
            self.authority, (LocalAgentExecutionAuthority, ShellExecutionAuthority)
        ):
            raise ExecutionContractError("execution authority is invalid")
        if self.effect != self.authority.kind:
            raise ExecutionContractError(
                "execution authority kind does not match effect"
            )
        if not isinstance(self.limits, ExecutionLimits):
            raise ExecutionContractError("execution limits are invalid")
        if not isinstance(self.sandbox, ExecutionSandbox):
            raise ExecutionContractError("execution sandbox is invalid")
        if os.name != "posix":
            raise ExecutionContractError(
                "host execution is unsupported on non-POSIX hosts"
            )
        if self.sandbox.mode != "bubblewrap":
            raise ExecutionContractError("host effects require the Bubblewrap sandbox")
        if len(self.environment) > _MAX_ENVIRONMENT_ENTRIES:
            raise ExecutionContractError("execution environment is too large")
        environment_keys: set[str] = set()
        environment_bytes = 0
        for key, value in self.environment:
            if (
                not isinstance(key, str)
                or not _ENVIRONMENT_KEY_PATTERN.fullmatch(key)
                or key in environment_keys
                or key not in _EXECUTION_ENVIRONMENT_ALLOWLIST
                or not isinstance(value, str)
                or "\0" in value
            ):
                raise ExecutionContractError("execution environment is invalid")
            value_bytes = len(value.encode("utf-8"))
            if value_bytes > _MAX_ENVIRONMENT_VALUE_BYTES:
                raise ExecutionContractError("execution environment value is too large")
            environment_bytes += len(key.encode("utf-8")) + value_bytes
            if environment_bytes > _MAX_ENVIRONMENT_TOTAL_BYTES:
                raise ExecutionContractError("execution environment is too large")
            environment_keys.add(key)
        if self.input_text is not None:
            if not isinstance(self.input_text, str) or "\0" in self.input_text:
                raise ExecutionContractError("execution input is invalid")
            if len(self.input_text.encode("utf-8")) > _MAX_INPUT_BYTES:
                raise ExecutionContractError("execution input exceeds the bounded size")
            computed_input_sha256 = _sha256_text(self.input_text)
            if (
                self.input_sha256 is not None
                and self.input_sha256 != computed_input_sha256
            ):
                raise ExecutionContractError("execution input digest is invalid")
            object.__setattr__(self, "input_sha256", computed_input_sha256)
        elif self.input_sha256 is None:
            object.__setattr__(self, "input_sha256", _sha256_text(""))
        elif not re.fullmatch(r"[0-9a-f]{64}", self.input_sha256):
            raise ExecutionContractError("execution input digest is invalid")

    @property
    def request_digest(self) -> str:
        return _sha256_bytes(_json_bytes(self.to_dict(include_input=False)))

    def to_dict(self, *, include_input: bool = True) -> dict[str, Any]:
        self.validate()
        payload: dict[str, Any] = {
            "schema_version": self.schema_version,
            "request_id": self.request_id,
            "effect": self.effect,
            "argv": list(self.argv),
            "working_directory": self.working_directory,
            "authority": self.authority.to_dict(),
            "limits": self.limits.to_dict(),
            "sandbox": self.sandbox.to_dict(),
            "environment": {key: value for key, value in self.environment},
            "input_sha256": self.input_sha256,
            "shell": False,
        }
        if include_input and self.input_text is not None:
            payload["input_text"] = self.input_text
        return payload

    def with_updates(self, **updates: Any) -> "ExecutionRequest":
        if "input_text" in updates and "input_sha256" not in updates:
            updates["input_sha256"] = None
        return replace(self, **updates)

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any]) -> "ExecutionRequest":
        if not isinstance(payload, Mapping):
            raise ExecutionContractError("execution request must be an object")
        if "schema_version" not in payload:
            raise ExecutionContractError("execution request schema is missing")
        raw_authority = payload.get("authority")
        if not isinstance(raw_authority, Mapping):
            raise ExecutionContractError("execution request authority is missing")
        authority: ExecutionAuthority
        if raw_authority.get("kind") == "local-agent":
            authority = LocalAgentExecutionAuthority.from_dict(raw_authority)
        elif raw_authority.get("kind") == "shell":
            authority = ShellExecutionAuthority.from_dict(raw_authority)
        else:
            raise ExecutionContractError("execution request authority kind is invalid")
        raw_environment = payload.get("environment", {})
        if not isinstance(raw_environment, Mapping):
            raise ExecutionContractError("execution request environment is invalid")
        input_text = payload.get("input_text")
        if input_text is not None and not isinstance(input_text, str):
            raise ExecutionContractError("execution request input is invalid")
        request = cls(
            schema_version=payload["schema_version"],
            request_id=payload["request_id"],
            effect=payload["effect"],
            argv=tuple(payload["argv"]),
            working_directory=payload["working_directory"],
            authority=authority,
            limits=ExecutionLimits.from_dict(payload["limits"]),
            sandbox=ExecutionSandbox.from_dict(payload["sandbox"]),
            environment=tuple(
                sorted((str(key), value) for key, value in raw_environment.items())
            ),
            input_text=input_text,
            input_sha256=payload.get("input_sha256"),
            shell=payload.get("shell", False),
        )
        input_digest = payload.get("input_sha256")
        if (
            input_text is not None
            and input_digest is not None
            and input_digest != _sha256_text(input_text)
        ):
            raise ExecutionContractError("execution request input digest is invalid")
        return request


@dataclass(frozen=True)
class ExecutionReceipt:
    """Typed outcome of one request, including explicit uncertainty."""

    request_id: str
    request_digest: str
    effect: ExecutionEffect
    status: ExecutionStatus
    started_at: str
    ended_at: str
    exit_code: int | None
    stdout: str
    stderr: str
    stdout_bytes: int
    stderr_bytes: int
    stdout_sha256: str
    stderr_sha256: str
    effect_started: bool
    reconciliation_required: bool
    error_code: str
    error_message: str
    receipt_id: str = ""
    owner_pid: int | None = None
    owner_identity: str = ""
    process_pid: int | None = None
    process_identity: str = ""
    provider: str = "python"
    schema_version: int = EXECUTION_SCHEMA_VERSION

    def __post_init__(self) -> None:
        if self.schema_version != EXECUTION_SCHEMA_VERSION:
            raise ExecutionContractError("unsupported execution receipt schema")
        _validate_identity(self.request_id, label="execution receipt request identity")
        if not re.fullmatch(r"[0-9a-f]{64}", self.request_digest):
            raise ExecutionContractError("execution receipt request digest is invalid")
        if self.effect not in {"local-agent", "shell"}:
            raise ExecutionContractError("execution receipt effect is invalid")
        if self.status not in {
            "executing",
            "completed",
            "failed",
            "cancelled",
            "timed-out",
            "output-limit",
            "start-failed",
            "outcome-unknown",
        }:
            raise ExecutionContractError("execution receipt status is invalid")
        _validate_timestamp(self.started_at, label="started_at")
        _validate_timestamp(self.ended_at, label="ended_at")
        if self.exit_code is not None and (
            not isinstance(self.exit_code, int) or isinstance(self.exit_code, bool)
        ):
            raise ExecutionContractError("execution receipt exit code is invalid")
        for pid, label in (
            (self.owner_pid, "owner_pid"),
            (self.process_pid, "process_pid"),
        ):
            if pid is not None and (
                not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0
            ):
                raise ExecutionContractError(f"execution receipt {label} is invalid")
        if not isinstance(self.effect_started, bool) or not isinstance(
            self.reconciliation_required, bool
        ):
            raise ExecutionContractError("execution receipt effect flags are invalid")
        if self.status == "executing":
            if (
                self.effect_started
                or self.reconciliation_required
                or self.exit_code is not None
            ):
                raise ExecutionContractError("executing receipt flags are invalid")
        elif self.status == "start-failed":
            if (
                self.effect_started
                or self.reconciliation_required
                or self.exit_code != 127
            ):
                raise ExecutionContractError("start-failed receipt flags are invalid")
        elif self.status == "outcome-unknown":
            if (
                not self.effect_started
                or not self.reconciliation_required
                or self.exit_code is not None
            ):
                raise ExecutionContractError(
                    "outcome-unknown receipt flags are invalid"
                )
        elif self.status == "cancelled":
            if self.reconciliation_required or self.exit_code is not None:
                raise ExecutionContractError("cancelled receipt flags are invalid")
        else:
            if (
                not self.effect_started
                or self.reconciliation_required
                or self.exit_code is None
            ):
                raise ExecutionContractError("terminal receipt flags are invalid")
        if self.status == "completed" and self.exit_code != 0:
            raise ExecutionContractError("completed receipt exit code is invalid")
        if self.status == "failed" and self.exit_code == 0:
            raise ExecutionContractError("failed receipt exit code is invalid")
        if self.status == "timed-out" and self.exit_code != 124:
            raise ExecutionContractError("timed-out receipt exit code is invalid")
        if self.status == "output-limit" and self.exit_code != 125:
            raise ExecutionContractError("output-limit receipt exit code is invalid")
        for value, label in ((self.stdout, "stdout"), (self.stderr, "stderr")):
            if not isinstance(value, str) or "\0" in value:
                raise ExecutionContractError(f"execution receipt {label} is invalid")
        for value, label in (
            (self.stdout_bytes, "stdout_bytes"),
            (self.stderr_bytes, "stderr_bytes"),
        ):
            if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                raise ExecutionContractError(f"execution receipt {label} is invalid")
        for value, label in (
            (self.error_code, "error_code"),
            (self.error_message, "error_message"),
            (self.provider, "provider"),
            (self.owner_identity, "owner_identity"),
            (self.process_identity, "process_identity"),
        ):
            if not isinstance(value, str) or "\0" in value:
                raise ExecutionContractError(f"execution receipt {label} is invalid")
        _validate_identity(self.receipt_id, label="execution receipt identity")
        expected_receipt_id = "execution-receipt:" + _sha256_text(
            "\n".join(
                (
                    self.request_id,
                    self.request_digest,
                    self.started_at,
                    self.ended_at,
                    self.status,
                )
            )
        )
        if self.receipt_id != expected_receipt_id:
            raise ExecutionContractError("execution receipt identity is invalid")
        if not re.fullmatch(r"[0-9a-f]{64}", self.stdout_sha256) or not re.fullmatch(
            r"[0-9a-f]{64}", self.stderr_sha256
        ):
            raise ExecutionContractError("execution receipt output digest is invalid")

    @classmethod
    def _make(
        cls,
        request: ExecutionRequest,
        *,
        status: ExecutionStatus,
        exit_code: int | None,
        stdout: str = "",
        stderr: str = "",
        effect_started: bool,
        reconciliation_required: bool,
        error_code: str = "",
        error_message: str = "",
        started_at: str | None = None,
        ended_at: str | None = None,
        owner_pid: int | None = None,
        owner_identity: str = "",
        process_pid: int | None = None,
        process_identity: str = "",
        provider: str = "python",
    ) -> "ExecutionReceipt":
        started = started_at or _utc_now()
        ended = ended_at or _utc_now()
        stdout_bytes = len(stdout.encode("utf-8"))
        stderr_bytes = len(stderr.encode("utf-8"))
        receipt_id = "execution-receipt:" + _sha256_text(
            "\n".join(
                (request.request_id, request.request_digest, started, ended, status)
            )
        )
        return cls(
            request_id=request.request_id,
            request_digest=request.request_digest,
            effect=request.effect,
            status=status,
            started_at=started,
            ended_at=ended,
            exit_code=exit_code,
            stdout=stdout,
            stderr=stderr,
            stdout_bytes=stdout_bytes,
            stderr_bytes=stderr_bytes,
            stdout_sha256=_sha256_text(stdout),
            stderr_sha256=_sha256_text(stderr),
            effect_started=effect_started,
            reconciliation_required=reconciliation_required,
            error_code=error_code,
            error_message=error_message,
            receipt_id=receipt_id,
            owner_pid=owner_pid,
            owner_identity=owner_identity,
            process_pid=process_pid,
            process_identity=process_identity,
            provider=provider,
        )

    @classmethod
    def executing(cls, request: ExecutionRequest) -> "ExecutionReceipt":
        return cls._make(
            request,
            status="executing",
            exit_code=None,
            effect_started=False,
            reconciliation_required=False,
            owner_pid=os.getpid(),
            owner_identity=_process_identity(os.getpid()),
        )

    @classmethod
    def completed(
        cls,
        request: ExecutionRequest,
        *,
        exit_code: int,
        stdout: str,
        stderr: str,
    ) -> "ExecutionReceipt":
        return cls._make(
            request,
            status="completed" if exit_code == 0 else "failed",
            exit_code=exit_code,
            stdout=stdout,
            stderr=stderr,
            effect_started=True,
            reconciliation_required=False,
        )

    @classmethod
    def start_failed(
        cls,
        request: ExecutionRequest,
        *,
        exit_code: int,
        error_message: str,
    ) -> "ExecutionReceipt":
        return cls._make(
            request,
            status="start-failed",
            exit_code=exit_code,
            effect_started=False,
            reconciliation_required=False,
            error_code="provider-start-failed",
            error_message=error_message,
        )

    @classmethod
    def unknown(
        cls,
        request: ExecutionRequest,
        *,
        error_message: str,
        owner_pid: int | None = None,
        owner_identity: str = "",
        process_pid: int | None = None,
        process_identity: str = "",
    ) -> "ExecutionReceipt":
        return cls._make(
            request,
            status="outcome-unknown",
            exit_code=None,
            effect_started=True,
            reconciliation_required=True,
            error_code="outcome-unknown",
            error_message=error_message,
            owner_pid=owner_pid,
            owner_identity=owner_identity,
            process_pid=process_pid,
            process_identity=process_identity,
        )

    @classmethod
    def cancelled(
        cls,
        request: ExecutionRequest,
        *,
        error_message: str,
        effect_started: bool = True,
        process_pid: int | None = None,
        process_identity: str = "",
    ) -> "ExecutionReceipt":
        return cls._make(
            request,
            status="cancelled",
            exit_code=None,
            effect_started=effect_started,
            reconciliation_required=False,
            error_code="cancelled",
            error_message=error_message,
            process_pid=process_pid,
            process_identity=process_identity,
        )

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any]) -> "ExecutionReceipt":
        if not isinstance(payload, Mapping):
            raise ExecutionContractError("execution receipt must be an object")
        for field_name in ("schema_version", "receipt_id"):
            if field_name not in payload:
                raise ExecutionContractError(
                    f"execution receipt {field_name} is missing"
                )
        receipt = cls(
            schema_version=payload["schema_version"],
            request_id=payload["request_id"],
            request_digest=payload["request_digest"],
            effect=payload["effect"],
            status=payload["status"],
            started_at=payload["started_at"],
            ended_at=payload["ended_at"],
            exit_code=payload.get("exit_code"),
            stdout=payload.get("stdout", ""),
            stderr=payload.get("stderr", ""),
            stdout_bytes=payload["stdout_bytes"],
            stderr_bytes=payload["stderr_bytes"],
            stdout_sha256=payload["stdout_sha256"],
            stderr_sha256=payload["stderr_sha256"],
            effect_started=payload["effect_started"],
            reconciliation_required=payload["reconciliation_required"],
            error_code=payload.get("error_code", ""),
            error_message=payload.get("error_message", ""),
            receipt_id=payload["receipt_id"],
            owner_pid=payload.get("owner_pid"),
            owner_identity=payload.get("owner_identity", ""),
            process_pid=payload.get("process_pid"),
            process_identity=payload.get("process_identity", ""),
            provider=payload.get("provider", "python"),
        )
        if receipt.stdout and _sha256_text(receipt.stdout) != receipt.stdout_sha256:
            raise ExecutionContractError("execution receipt stdout digest is invalid")
        if receipt.stderr and _sha256_text(receipt.stderr) != receipt.stderr_sha256:
            raise ExecutionContractError("execution receipt stderr digest is invalid")
        if receipt.stdout and receipt.stdout_bytes != len(
            receipt.stdout.encode("utf-8")
        ):
            raise ExecutionContractError("execution receipt stdout size is invalid")
        if receipt.stderr and receipt.stderr_bytes != len(
            receipt.stderr.encode("utf-8")
        ):
            raise ExecutionContractError("execution receipt stderr size is invalid")
        return receipt

    def to_dict(self, *, include_output: bool = True) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "schema_version": self.schema_version,
            "request_id": self.request_id,
            "request_digest": self.request_digest,
            "effect": self.effect,
            "status": self.status,
            "started_at": self.started_at,
            "ended_at": self.ended_at,
            "exit_code": self.exit_code,
            "stdout_bytes": self.stdout_bytes,
            "stderr_bytes": self.stderr_bytes,
            "stdout_sha256": self.stdout_sha256,
            "stderr_sha256": self.stderr_sha256,
            "effect_started": self.effect_started,
            "reconciliation_required": self.reconciliation_required,
            "error_code": self.error_code,
            "error_message": self.error_message,
            "receipt_id": self.receipt_id,
            "owner_pid": self.owner_pid,
            "owner_identity": self.owner_identity,
            "process_pid": self.process_pid,
            "process_identity": self.process_identity,
            "provider": self.provider,
        }
        if include_output:
            payload["stdout"] = self.stdout
            payload["stderr"] = self.stderr
        return payload


def _process_identity(pid: int) -> str:
    if pid <= 0:
        return ""
    if sys.platform.startswith("linux"):
        try:
            remainder = (
                Path(f"/proc/{pid}/stat")
                .read_text(encoding="utf-8")
                .rsplit(")", 1)[1]
                .split()
            )
        except (OSError, IndexError, UnicodeError):
            return ""
        if len(remainder) <= 19:
            return ""
        return f"linux:{pid}:{remainder[19]}"
    if os.name == "posix":
        for executable in ("/bin/ps", "/usr/bin/ps"):
            try:
                result = subprocess.run(
                    [executable, "-p", str(pid), "-o", "lstart="],
                    check=False,
                    capture_output=True,
                    text=True,
                    timeout=0.5,
                )
            except (OSError, subprocess.SubprocessError):
                continue
            started_at = result.stdout.strip()
            if result.returncode == 0 and started_at:
                return f"posix:{pid}:{started_at}"
    return ""


def _owner_is_live(pid: int | None, identity: str) -> bool:
    if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0:
        return False
    return bool(identity) and _process_identity(pid) == identity


ExecutionExecutor = Callable[..., subprocess.CompletedProcess[str]]


class PythonExecutionProvider:
    """Python-backed provider for an already prepared bounded argv effect."""

    def __init__(
        self,
        *,
        executor: ExecutionExecutor | None = None,
        provider_id: str = "python",
    ) -> None:
        self._executor = executor or self._default_executor
        self.provider_id = provider_id

    def validate_request(self, request: ExecutionRequest) -> None:
        request.validate()
        _validate_prepared_bubblewrap_argv(
            request.argv,
            request.sandbox,
            request.working_directory,
            request.limits,
            request.environment,
        )

    @staticmethod
    def _default_executor(
        argv: Sequence[str], **kwargs: Any
    ) -> subprocess.CompletedProcess[str]:
        # Lazy import preserves the public module boundary without making core
        # depend on itself while it initializes the compatibility helpers.
        from .core import _run_bounded_process

        return _run_bounded_process(list(argv), **kwargs)

    def execute(
        self,
        request: ExecutionRequest,
        *,
        process_binding_started: Callable[[Any, str], None] | None = None,
        poll_callback: Callable[[], None] | None = None,
        output_callback: Callable[[str, bytes], None] | None = None,
    ) -> ExecutionReceipt:
        self.validate_request(request)
        binding: dict[str, Any] = {}

        def bind(process: Any, process_token: str = "") -> None:
            binding["pid"] = getattr(process, "pid", None)
            binding["identity"] = (
                _process_identity(binding["pid"])
                if isinstance(binding["pid"], int)
                else ""
            )
            if process_binding_started is not None:
                process_binding_started(process, process_token)

        try:
            provider_argv: Sequence[str] | str = list(request.argv)
            if os.name == "nt" and len(request.argv) == 1:
                provider_argv = request.argv[0]
            completed = self._executor(
                provider_argv,
                input_text=request.input_text,
                cwd=Path(request.working_directory),
                env=dict(request.environment),
                timeout_seconds=request.limits.timeout_seconds,
                output_limit_bytes=request.limits.output_limit_bytes,
                address_space_bytes=request.limits.address_space_bytes,
                file_size_bytes=request.limits.file_size_bytes,
                open_file_limit=request.limits.open_file_limit,
                process_count_limit=request.limits.process_count_limit,
                descendant_grace_seconds=request.limits.descendant_grace_seconds,
                process_binding_started=bind,
                poll_callback=poll_callback,
                output_callback=output_callback,
            )
        except FileNotFoundError as exc:
            if binding:
                return ExecutionReceipt.unknown(
                    request,
                    error_message=str(exc),
                    process_pid=binding.get("pid"),
                    process_identity=binding.get("identity", ""),
                )
            return ExecutionReceipt.start_failed(
                request,
                exit_code=127,
                error_message=str(exc),
            )
        except OSError as exc:
            if binding:
                return ExecutionReceipt.unknown(
                    request,
                    error_message=str(exc),
                    process_pid=binding.get("pid"),
                    process_identity=binding.get("identity", ""),
                )
            return ExecutionReceipt.start_failed(
                request,
                exit_code=127,
                error_message=str(exc),
            )
        if not isinstance(completed, subprocess.CompletedProcess):
            raise ExecutionContractError(
                "Python execution provider returned an invalid result"
            )
        stdout = completed.stdout if isinstance(completed.stdout, str) else ""
        stderr = completed.stderr if isinstance(completed.stderr, str) else ""
        bounded_outcome = getattr(completed, "albert_outcome", "")
        if bounded_outcome not in {"", "timed-out", "output-limit"}:
            raise ExecutionContractError(
                "Python execution provider returned an invalid outcome marker"
            )
        return ExecutionReceipt._make(
            request,
            status=(
                bounded_outcome
                if bounded_outcome
                else "completed"
                if completed.returncode == 0
                else "failed"
            ),
            exit_code=completed.returncode,
            stdout=stdout,
            stderr=stderr,
            effect_started=True,
            reconciliation_required=False,
            error_code=(
                "timeout"
                if bounded_outcome == "timed-out"
                else "output-limit"
                if bounded_outcome == "output-limit"
                else ""
            ),
            error_message=(
                "Process timed out after the bounded timeout."
                if bounded_outcome == "timed-out"
                else "Process output exceeded the bounded output limit."
                if bounded_outcome == "output-limit"
                else ""
            ),
            process_pid=binding.get("pid"),
            process_identity=binding.get("identity", ""),
            provider=self.provider_id,
        )


class ExecutionJournal:
    """Atomic, lock-serialized intent and receipt storage for one runtime."""

    def __init__(self, path: Path):
        self.path = path
        self.lock_path = path.with_name(f".{path.name}.lock")

    @contextmanager
    def _lock(self):
        if fcntl is None:
            raise ExecutionContractError(
                "execution journal locking is unsupported on this host"
            )
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.lock_path.open("a+", encoding="utf-8") as lock_file:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)

    def _read(self) -> dict[str, Any]:
        if not self.path.exists():
            return {"schema_version": EXECUTION_SCHEMA_VERSION, "records": {}}
        try:
            payload = json.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise ExecutionContractError(
                f"execution journal read failed: {exc}"
            ) from exc
        if (
            not isinstance(payload, dict)
            or payload.get("schema_version") != EXECUTION_SCHEMA_VERSION
        ):
            raise ExecutionContractError("unsupported execution journal schema")
        records = payload.get("records")
        if not isinstance(records, dict):
            raise ExecutionContractError("execution journal records must be an object")
        for request_id, record in records.items():
            if not isinstance(request_id, str) or not isinstance(record, dict):
                raise ExecutionContractError("execution journal record is invalid")
            request_payload = record.get("request")
            receipt_payload = record.get("receipt")
            if not isinstance(request_payload, dict) or not isinstance(
                receipt_payload, dict
            ):
                raise ExecutionContractError("execution journal record is incomplete")
            if "input_text" in request_payload or any(
                field_name in receipt_payload for field_name in ("stdout", "stderr")
            ):
                raise ExecutionContractError(
                    "execution journal must not contain raw input or output"
                )
            try:
                request = ExecutionRequest.from_dict(request_payload)
                receipt = ExecutionReceipt.from_dict(receipt_payload)
            except (KeyError, TypeError, ValueError) as exc:
                raise ExecutionContractError(
                    "execution journal record contains an invalid request or receipt"
                ) from exc
            if request.request_id != request_id or receipt.request_id != request_id:
                raise ExecutionContractError(
                    "execution journal identity is inconsistent"
                )
            if (
                request.request_digest != record.get("request_digest")
                or receipt.request_digest != request.request_digest
                or receipt.effect != request.effect
            ):
                raise ExecutionContractError("execution journal digest is inconsistent")
        return payload

    def _write(self, payload: dict[str, Any]) -> None:
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
                temporary.write(json.dumps(payload, indent=2, sort_keys=True) + "\n")
                temporary.flush()
                os.fsync(temporary.fileno())
                temporary_path = Path(temporary.name)
            temporary_path.replace(self.path)
            if os.name == "posix":
                directory_fd = os.open(self.path.parent, os.O_RDONLY)
                try:
                    os.fsync(directory_fd)
                finally:
                    os.close(directory_fd)
        finally:
            if temporary_path is not None and temporary_path.exists():
                temporary_path.unlink()

    @staticmethod
    def _record(request: ExecutionRequest, receipt: ExecutionReceipt) -> dict[str, Any]:
        return {
            "request_digest": request.request_digest,
            "request": request.to_dict(include_input=False),
            "receipt": receipt.to_dict(include_output=False),
        }

    def claim(self, request: ExecutionRequest) -> ExecutionReceipt | None:
        request.validate()
        with self._lock():
            payload = self._read()
            records = payload["records"]
            existing = records.get(request.request_id)
            if existing is None:
                records[request.request_id] = self._record(
                    request,
                    ExecutionReceipt.executing(request),
                )
                self._write(payload)
                return None
            if existing.get("request_digest") != request.request_digest:
                raise ExecutionReplayConflict(
                    f"Execution request {request.request_id} was reused for a different effect."
                )
            receipt = ExecutionReceipt.from_dict(existing["receipt"])
            if receipt.status != "executing":
                return receipt
            if _owner_is_live(receipt.owner_pid, receipt.owner_identity):
                return receipt
            unknown = ExecutionReceipt.unknown(
                request,
                error_message=(
                    "The provider owner stopped before a typed execution receipt was "
                    "durably reconciled; inspect the external effect before deciding."
                ),
                owner_pid=receipt.owner_pid,
                owner_identity=receipt.owner_identity,
                process_pid=receipt.process_pid,
                process_identity=receipt.process_identity,
            )
            records[request.request_id] = self._record(request, unknown)
            self._write(payload)
            return unknown

    def bind_process(
        self,
        request: ExecutionRequest,
        *,
        process_pid: int,
        process_identity: str,
    ) -> ExecutionReceipt:
        """Durably bind child identity to the executing intent before capture setup."""

        request.validate()
        if (
            not isinstance(process_pid, int)
            or isinstance(process_pid, bool)
            or process_pid <= 0
            or not isinstance(process_identity, str)
            or "\0" in process_identity
        ):
            raise ExecutionContractError("execution process identity is invalid")
        with self._lock():
            payload = self._read()
            existing = payload["records"].get(request.request_id)
            if existing is None:
                raise ExecutionContractError(
                    "execution intent is missing before process binding"
                )
            if existing.get("request_digest") != request.request_digest:
                raise ExecutionReplayConflict(
                    f"Execution request {request.request_id} was reused for a different effect."
                )
            current = ExecutionReceipt.from_dict(existing["receipt"])
            if current.status != "executing":
                return current
            bound = replace(
                current,
                process_pid=process_pid,
                process_identity=process_identity,
            )
            payload["records"][request.request_id] = self._record(request, bound)
            self._write(payload)
            return bound

    def record_start_failed(
        self,
        request: ExecutionRequest,
        receipt: ExecutionReceipt,
    ) -> ExecutionReceipt:
        """Atomically persist a deterministic pre-effect failure or existing result."""

        request.validate()
        if (
            receipt.status != "start-failed"
            or receipt.request_id != request.request_id
            or receipt.request_digest != request.request_digest
        ):
            raise ExecutionContractError(
                "execution start-failed receipt does not match request"
            )
        with self._lock():
            payload = self._read()
            records = payload["records"]
            existing = records.get(request.request_id)
            if existing is not None:
                if existing.get("request_digest") != request.request_digest:
                    raise ExecutionReplayConflict(
                        f"Execution request {request.request_id} was reused for a different effect."
                    )
                current = ExecutionReceipt.from_dict(existing["receipt"])
                if current.status == "executing" and not _owner_is_live(
                    current.owner_pid,
                    current.owner_identity,
                ):
                    current = ExecutionReceipt.unknown(
                        request,
                        error_message=(
                            "The provider owner stopped before a typed execution receipt was "
                            "durably reconciled; inspect the external effect before deciding."
                        ),
                        owner_pid=current.owner_pid,
                        owner_identity=current.owner_identity,
                        process_pid=current.process_pid,
                        process_identity=current.process_identity,
                    )
                    records[request.request_id] = self._record(request, current)
                    self._write(payload)
                return current
            records[request.request_id] = self._record(request, receipt)
            self._write(payload)
            return receipt

    def complete(self, request: ExecutionRequest, receipt: ExecutionReceipt) -> None:
        request.validate()
        if (
            receipt.request_id != request.request_id
            or receipt.request_digest != request.request_digest
        ):
            raise ExecutionContractError("execution receipt does not match request")
        with self._lock():
            payload = self._read()
            existing = payload["records"].get(request.request_id)
            if existing is None:
                raise ExecutionContractError(
                    "execution intent is missing before receipt reconciliation"
                )
            if existing.get("request_digest") != request.request_digest:
                raise ExecutionReplayConflict(
                    f"Execution request {request.request_id} was reused for a different effect."
                )
            current = ExecutionReceipt.from_dict(existing["receipt"])
            if current.status != "executing" and current.to_dict(
                include_output=False
            ) != receipt.to_dict(include_output=False):
                raise ExecutionReplayConflict(
                    f"Execution request {request.request_id} already has a different receipt."
                )
            payload["records"][request.request_id] = self._record(request, receipt)
            self._write(payload)

    def reconcile(self) -> tuple[ExecutionReceipt, ...]:
        with self._lock():
            payload = self._read()
            changed = False
            for request_id, record in payload["records"].items():
                receipt = ExecutionReceipt.from_dict(record["receipt"])
                if receipt.status != "executing" or _owner_is_live(
                    receipt.owner_pid, receipt.owner_identity
                ):
                    continue
                request = ExecutionRequest.from_dict(record["request"])
                unknown = ExecutionReceipt.unknown(
                    request,
                    error_message=(
                        "The provider owner stopped before a typed execution receipt was "
                        "durably reconciled; inspect the external effect before deciding."
                    ),
                    owner_pid=receipt.owner_pid,
                    owner_identity=receipt.owner_identity,
                    process_pid=receipt.process_pid,
                    process_identity=receipt.process_identity,
                )
                payload["records"][request_id] = self._record(request, unknown)
                changed = True
            if changed:
                self._write(payload)
            return tuple(
                ExecutionReceipt.from_dict(record["receipt"])
                for record in payload["records"].values()
            )

    def inspect(self) -> tuple[ExecutionReceipt, ...]:
        return tuple(receipt for _request, receipt in self.inspect_records())

    def inspect_records(self) -> tuple[tuple[ExecutionRequest, ExecutionReceipt], ...]:
        with self._lock():
            payload = self._read()
            return tuple(
                sorted(
                    (
                        (
                            ExecutionRequest.from_dict(record["request"]),
                            ExecutionReceipt.from_dict(record["receipt"]),
                        )
                        for record in payload["records"].values()
                    ),
                    key=lambda item: (item[1].started_at, item[1].request_id),
                )
            )


class ExecutionCoordinator:
    """Authorize, persist, execute, and reconcile one exact host effect."""

    def __init__(self, journal: ExecutionJournal, provider: PythonExecutionProvider):
        self.journal = journal
        self.provider = provider

    def execute(
        self,
        request: ExecutionRequest,
        *,
        authorize: Callable[[ExecutionRequest], None] | None = None,
        process_binding_started: Callable[[Any, str], None] | None = None,
        poll_callback: Callable[[], None] | None = None,
        output_callback: Callable[[str, bytes], None] | None = None,
        exception_status: Callable[
            [BaseException], Literal["cancelled", "outcome-unknown"]
        ]
        | None = None,
    ) -> ExecutionReceipt:
        request.validate()
        try:
            if authorize is not None:
                authorize(request)
            self.provider.validate_request(request)
        except Exception as exc:
            receipt = ExecutionReceipt.start_failed(
                request,
                exit_code=127,
                error_message=str(exc),
            )
            return self.journal.record_start_failed(request, receipt)
        process_bound = False
        process_pid: int | None = None
        process_identity = ""

        def bind(process: Any, process_token: str = "") -> None:
            nonlocal process_bound, process_pid, process_identity
            process_bound = True
            process_pid = getattr(process, "pid", None)
            process_identity = (
                _process_identity(process_pid)
                if isinstance(process_pid, int)
                else ""
            )
            self.journal.bind_process(
                request,
                process_pid=process_pid,
                process_identity=process_identity,
            )
            if process_binding_started is not None:
                process_binding_started(process, process_token)

        existing = self.journal.claim(request)
        if existing is not None:
            return existing
        try:
            receipt = self.provider.execute(
                request,
                process_binding_started=bind,
                poll_callback=poll_callback,
                output_callback=output_callback,
            )
        except BaseException as exc:
            status = (
                exception_status(exc)
                if exception_status is not None
                else "outcome-unknown"
            )
            if status == "cancelled":
                receipt = ExecutionReceipt.cancelled(
                    request,
                    error_message=str(exc),
                    effect_started=process_bound,
                    process_pid=process_pid,
                    process_identity=process_identity,
                )
            elif process_bound:
                receipt = ExecutionReceipt.unknown(
                    request,
                    error_message=str(exc),
                    process_pid=process_pid,
                    process_identity=process_identity,
                )
            else:
                receipt = ExecutionReceipt.start_failed(
                    request,
                    exit_code=127,
                    error_message=str(exc),
                )
            try:
                self.journal.complete(request, receipt)
            finally:
                raise
        self.journal.complete(request, receipt)
        return receipt


__all__ = [
    "EXECUTION_SCHEMA_VERSION",
    "ExecutionAuthority",
    "ExecutionContractError",
    "ExecutionCoordinator",
    "ExecutionEffect",
    "ExecutionJournal",
    "ExecutionLimits",
    "ExecutionReceipt",
    "ExecutionReplayConflict",
    "ExecutionRequest",
    "ExecutionSandbox",
    "ExecutionStatus",
    "LocalAgentExecutionAuthority",
    "PythonExecutionProvider",
    "ShellExecutionAuthority",
]
