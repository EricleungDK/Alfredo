from __future__ import annotations

import codecs
from contextlib import contextmanager, nullcontext
from dataclasses import dataclass, field, replace
from datetime import datetime, timezone
import difflib
from hashlib import sha1, sha256
import fcntl
import json
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any, Callable
from urllib.parse import quote

from .agents import (
    AgentConfig,
    AgentRegistry,
    is_cloud_model,
    is_eligible_assignment_agent,
    is_eligible_controller_agent,
    load_agent_registry,
)
from .capabilities import CapabilityCatalogService, SKILL_NAME_PATTERN


_REPOSITORY_CONTEXT_LIMIT = 24_000
_REPOSITORY_TREE_LIMIT = 8_000
_REPOSITORY_SOURCE_LIMIT = 4_000
_REPOSITORY_SOURCE_COUNT = 10
_MODEL_COMMAND_LIMIT = 8
_MODEL_COMMAND_LENGTH_LIMIT = 1_000
_MODEL_FILE_COUNT_LIMIT = 128
_MODEL_FILE_BYTES_LIMIT = 512_000
_MODEL_FILE_TOTAL_BYTES_LIMIT = 2_000_000
_MODEL_COMMAND_TIMEOUT_SECONDS = 120
_MODEL_AGENT_ITERATION_LIMIT = 3
_MODEL_FEEDBACK_LIMIT = 8_000
_MODEL_PROCESS_OUTPUT_BYTES_LIMIT = 3_000_000
_RUNNER_COMMAND_TIMEOUT_SECONDS = 600
_PROCESS_OUTPUT_BYTES_LIMIT = 1_000_000
_PROCESS_OUTPUT_LIMIT_EXIT_STATUS = 125
_PROCESS_OUTPUT_MESSAGE_RESERVE = 256
_PROCESS_ADDRESS_SPACE_BYTES_LIMIT = 8 * 1024 * 1024 * 1024
_PROCESS_FILE_SIZE_BYTES_LIMIT = 2 * 1024 * 1024 * 1024
_PROCESS_OPEN_FILE_LIMIT = 1_024
_PROCESS_COUNT_LIMIT = 256
_PROCESS_DESCENDANT_GRACE_SECONDS = 1.0
_GIT_SNAPSHOT_TIMEOUT_SECONDS = 30
_GIT_SNAPSHOT_BYTES_LIMIT = 8_000_000
_GIT_COMMAND_OUTPUT_BYTES_LIMIT = 64_000
_GIT_PATH_OUTPUT_BYTES_LIMIT = 1_000_000
_GIT_PATH_COUNT_LIMIT = 10_000
_FILESYSTEM_SCAN_ENTRY_LIMIT = 20_000
_FINGERPRINT_SAMPLE_BYTES_LIMIT = 128 * 1024
_UNTRACKED_SOURCE_FILE_LIMIT = 64
_UNTRACKED_SOURCE_FILE_BYTES_LIMIT = 128_000
_UNTRACKED_SOURCE_TOTAL_BYTES_LIMIT = 1_000_000
_UNTRACKED_SOURCE_MANIFEST_BYTES_LIMIT = 2_000_000
_DIRECTORY_SOURCE_SCAN_LIMIT = 2_000
_DEPENDENCY_PARENT_SCAN_LIMIT = 256
_REPAIR_INHERITED_FILE_LIMIT = 128
_REPAIR_INHERITED_FILE_BYTES_LIMIT = 512_000
_REPAIR_INHERITED_TOTAL_BYTES_LIMIT = 2_000_000
_REPAIR_INHERITED_MANIFEST_BYTES_LIMIT = 2_000_000
_REVIEW_DIFF_BYTES_LIMIT = 1_000_000
_REVIEW_BASELINE_FILE_LIMIT = 256
_REVIEW_BASELINE_FILE_BYTES_LIMIT = _REVIEW_DIFF_BYTES_LIMIT
_REVIEW_BASELINE_TOTAL_BYTES_LIMIT = 8_000_000
_WORKTREE_PREPARATION_SCHEMA_VERSION = 1
_GIT_NOT_REPOSITORY_MESSAGE = (
    "fatal: not a git repository (or any of the parent directories): .git"
)
_GIT_NOT_REPOSITORY_BOUNDARY_PATTERN = re.compile(
    r"fatal: not a git repository \(or any parent up to mount point .+\)\n"
    r"Stopping at filesystem boundary "
    r"\(GIT_DISCOVERY_ACROSS_FILESYSTEM not set\)\."
)
_SKILL_INSTRUCTION_LIMIT = 12_000
_TEXT_SOURCE_SUFFIXES = {
    ".c",
    ".cc",
    ".cpp",
    ".cs",
    ".css",
    ".go",
    ".h",
    ".hpp",
    ".html",
    ".java",
    ".js",
    ".json",
    ".jsx",
    ".kt",
    ".md",
    ".php",
    ".py",
    ".rb",
    ".rs",
    ".scss",
    ".sh",
    ".sql",
    ".swift",
    ".toml",
    ".ts",
    ".tsx",
    ".txt",
    ".xml",
    ".yaml",
    ".yml",
}
_TEXT_SOURCE_NAMES = {
    "Dockerfile",
    "Makefile",
    "Procfile",
}
_PROCESS_ENV_ALLOWLIST = {
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
_PROCESS_ENV_OVERRIDE_ALLOWLIST = {
    "ALBERT_SESSION_ID",
    "ALBERT_TASK_PACKET",
}
_PROCESS_SYSTEM_READ_ROOTS = (
    Path("/usr"),
    Path("/bin"),
    Path("/sbin"),
    Path("/lib"),
    Path("/lib64"),
    Path("/etc"),
)
_TRUSTED_PROCESS_HELPER_DIRECTORIES = (
    Path("/usr/bin"),
    Path("/usr/sbin"),
    Path("/bin"),
    Path("/sbin"),
)


class AlbertError(Exception):
    """Base error for user-actionable MVP failures."""


class SharedUnderstandingGateError(AlbertError):
    """Raised before work can cross a pending Wayfinder authority boundary."""

    code = "shared-understanding-required"

    def __init__(self) -> None:
        super().__init__(
            "Wayfinder Shared Understanding Gate is still pending; canonical planning "
            "artifacts, delegation, and production implementation are not eligible."
        )


class WayfinderStatePersistenceError(AlbertError):
    """Raised when the shared Wayfinder state cannot prove launch eligibility."""

    code = "persistence-read-failure"


class LockedFieldError(AlbertError):
    """Raised when an approved Issue Slice contract is edited while locked."""


class LaunchBlockedError(AlbertError):
    """Raised when an Issue Slice cannot be launched."""


class SessionCancelledError(AlbertError):
    """Raised internally when durable cancellation stops a runner."""


class EvidenceValidationError(AlbertError):
    """Raised when an Evidence Package is incomplete."""


def wayfinder_state_path(*, runtime_root: Path, target_repo: Path) -> Path:
    """Return the mission-independent durable Wayfinder state path for one repository."""
    canonical_target = str(target_repo.resolve())
    project_key = sha1(canonical_target.encode("utf-8")).hexdigest()[:16]
    return runtime_root.resolve() / "wayfinder" / project_key / "wayfinder-state.json"


def load_wayfinder_state(path: Path) -> dict[str, Any]:
    """Load and validate the schema-versioned shared Wayfinder state."""
    if not path.exists():
        return {"schema_version": 1, "active_flow": None}
    try:
        state = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(state, dict) or state.get("schema_version") != 1:
            raise ValueError("unsupported Wayfinder state schema")
        if "active_flow" not in state:
            raise ValueError("Wayfinder state must contain active_flow")
        active = state.get("active_flow")
        if active is None:
            return state
        if not isinstance(active, dict):
            raise ValueError("Wayfinder active flow must be an object")
        for field_name in ("flow_id", "originating_message_id", "reference"):
            if not isinstance(active.get(field_name), str):
                raise ValueError(f"Wayfinder active flow {field_name} must be a string")
        if active.get("mode") not in {"chart", "work-through"}:
            raise ValueError("Wayfinder active flow has an invalid mode")
        scope = active.get("scope")
        if not isinstance(scope, dict):
            raise ValueError("Wayfinder active flow scope must be an object")
        if scope.get("kind") not in {"working-directory", "mission", "issue-slice"}:
            raise ValueError("Wayfinder active flow scope has an invalid kind")
        if not isinstance(scope.get("target_id"), str) or not isinstance(scope.get("label"), str):
            raise ValueError("Wayfinder active flow scope must name target and label")
        if scope.get("mission_id") is not None and not isinstance(scope.get("mission_id"), str):
            raise ValueError("Wayfinder active flow scope Mission must be a string or null")
        gate = active.get("gate")
        if not isinstance(gate, dict):
            raise ValueError("Wayfinder active flow must contain a gate")
        if gate.get("status") not in {"pending", "open"}:
            raise ValueError("Wayfinder gate has an invalid status")
        if not isinstance(gate.get("opened_by"), str) or not isinstance(
            gate.get("receipt_id"), str
        ):
            raise ValueError("Wayfinder gate receipt fields must be strings")
        if gate["status"] == "open" and (
            gate["opened_by"] not in {"mission-commander", "wayfinder-agent"}
            or not gate["receipt_id"].strip()
        ):
            raise ValueError("Open Wayfinder gate must have a valid opening receipt")
        return state
    except (OSError, json.JSONDecodeError, TypeError, ValueError) as exc:
        raise WayfinderStatePersistenceError(
            f"Wayfinder state persistence read failed: {exc}"
        ) from exc


def ensure_wayfinder_gate_open(*, runtime_root: Path, target_repo: Path) -> None:
    """Fail closed before a direct production launch crosses a pending Wayfinder gate."""
    path = wayfinder_state_path(runtime_root=runtime_root, target_repo=target_repo)
    state = load_wayfinder_state(path)
    active = state["active_flow"]
    if active is None:
        return
    if active["gate"]["status"] != "open":
        raise SharedUnderstandingGateError()


def _read_bounded_bytes(path: Path, limit_bytes: int) -> bytes:
    """Read at most limit + one probe byte without loading the whole file."""

    if limit_bytes < 0:
        raise ValueError("bounded byte-read limit must not be negative")
    with path.open("rb") as source:
        return source.read(limit_bytes + 1)


def _read_bounded_utf8(
    path: Path,
    limit_characters: int,
    *,
    probe_for_truncation: bool = False,
) -> str:
    """Read a code-point-safe UTF-8 prefix through a bounded text stream."""

    if limit_characters < 0:
        raise ValueError("bounded text-read limit must not be negative")
    read_size = limit_characters + (1 if probe_for_truncation else 0)
    with path.open("r", encoding="utf-8") as source:
        return source.read(read_size)


def _bounded_process_output(
    value: str | bytes | None,
    *,
    limit_bytes: int = _PROCESS_OUTPUT_BYTES_LIMIT,
) -> str:
    """Decode and cap one captured child-process stream with an explicit marker."""

    if value is None:
        return ""
    text = (
        value.decode("utf-8", errors="replace")
        if isinstance(value, bytes)
        else value
    )
    payload = text.encode("utf-8", errors="replace")
    if len(payload) <= limit_bytes:
        return text
    marker = f"\n... process output truncated at {limit_bytes} bytes ...\n"
    marker_size = len(marker.encode("utf-8"))
    prefix = payload[: max(0, limit_bytes - marker_size)].decode(
        "utf-8", errors="replace"
    )
    while len(prefix.encode("utf-8")) + marker_size > limit_bytes:
        prefix = prefix[:-1]
    return prefix + marker


class _BoundedProcessCapture:
    """Drain child pipes while retaining at most one aggregate byte budget."""

    def __init__(
        self,
        limit_bytes: int,
        output_callback: Callable[[str, bytes], None] | None = None,
    ):
        self.limit_bytes = limit_bytes
        self.output_callback = output_callback
        self._remaining = max(0, limit_bytes - _PROCESS_OUTPUT_MESSAGE_RESERVE)
        self._buffers = {"stdout": bytearray(), "stderr": bytearray()}
        self._lock = threading.Lock()
        self.exceeded = threading.Event()
        self.invalid_utf8 = threading.Event()
        self.drained = threading.Event()
        self._threads: list[threading.Thread] = []
        self._active_streams = 0

    def start(self, process: subprocess.Popen[bytes]) -> None:
        if process.stdout is None or process.stderr is None:
            raise AlbertError("Bounded process capture requires stdout and stderr pipes.")
        self._active_streams = 2
        for name, stream in (("stdout", process.stdout), ("stderr", process.stderr)):
            thread = threading.Thread(
                target=self._drain,
                args=(name, stream),
                daemon=True,
            )
            thread.start()
            self._threads.append(thread)

    def _drain(self, name: str, stream: Any) -> None:
        try:
            while True:
                # BufferedReader.read(n) is allowed to wait for n bytes or EOF.
                # A live inspector must see a flushed short record while its child
                # is still running, so prefer the pipe's immediately-available
                # read primitive and retain the same aggregate byte budget below.
                read_available = getattr(stream, "read1", stream.read)
                chunk = read_available(64 * 1024)
                if not chunk:
                    break
                retained_chunk = b""
                with self._lock:
                    retained = min(self._remaining, len(chunk))
                    if retained:
                        self._buffers[name].extend(chunk[:retained])
                        self._remaining -= retained
                        retained_chunk = bytes(chunk[:retained])
                    if retained < len(chunk):
                        self.exceeded.set()
                if retained_chunk and self.output_callback is not None:
                    try:
                        self.output_callback(name, retained_chunk)
                    except Exception:
                        pass
        except OSError:
            pass
        finally:
            try:
                stream.close()
            except OSError:
                pass
            with self._lock:
                self._active_streams -= 1
                if self._active_streams == 0:
                    self.drained.set()

    def wait_for_eof(self, timeout: float) -> bool:
        return self.drained.wait(timeout=max(0.0, timeout))

    def finish(
        self,
        *,
        extra_stderr: str = "",
        join_timeout: float = 2.0,
    ) -> tuple[str, str]:
        deadline = time.monotonic() + max(0.0, join_timeout)
        for thread in self._threads:
            thread.join(timeout=max(0.0, deadline - time.monotonic()))
        try:
            stdout = bytes(self._buffers["stdout"]).decode("utf-8", errors="strict")
            stderr = bytes(self._buffers["stderr"]).decode("utf-8", errors="strict")
        except UnicodeDecodeError:
            # Do not drop or replace undecodable bytes: either strategy can
            # alias distinct POSIX paths, and replacement can expand beyond the
            # retained byte budget. The caller turns this into a bounded failure.
            self.invalid_utf8.set()
            stdout = ""
            stderr = ""
        if extra_stderr:
            stderr = f"{stderr.rstrip()}\n{extra_stderr}" if stderr else extra_stderr
        return stdout, stderr


class _SessionOutputRecorder:
    """Append bounded JSONL output without exposing a host path to clients."""

    _JOURNAL_LIMIT_BYTES = 128_000
    _EVENT_CONTENT_LIMIT_BYTES = 16_000

    def __init__(self, path: Path, mission_id: str, session_id: str):
        self._path = path
        self._mission_id = mission_id
        self._session_id = session_id
        self._lock = threading.Lock()
        self._sequence = 0
        self._bytes_written = 0
        self._decoders: dict[str, codecs.IncrementalDecoder] = {}
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.is_symlink():
            raise AlbertError("Session output journal must not be a symlink.")
        with path.open("wb"):
            pass
        self._stream = path.open("ab", buffering=0)

    def record(self, _stream_name: str, payload: bytes) -> None:
        if not payload:
            return
        with self._lock:
            decoder = self._decoders.setdefault(
                _stream_name,
                codecs.getincrementaldecoder("utf-8")(errors="replace"),
            )
            self._record_content(decoder.decode(payload, final=False))

    @classmethod
    def _utf8_chunks(cls, content: str) -> list[str]:
        """Split text into valid UTF-8 event payloads without dropping a suffix."""

        chunks: list[str] = []
        current: list[str] = []
        current_size = 0
        for character in content:
            character_size = len(character.encode("utf-8"))
            if current and current_size + character_size > cls._EVENT_CONTENT_LIMIT_BYTES:
                chunks.append("".join(current))
                current = []
                current_size = 0
            current.append(character)
            current_size += character_size
        if current:
            chunks.append("".join(current))
        return chunks

    def _encoded_event(self, sequence: int, content: str) -> bytes:
        return (
            json.dumps(
                {
                    "schema_version": 1,
                    "mission_id": self._mission_id,
                    "session_id": self._session_id,
                    "sequence": sequence,
                    "content": content,
                },
                ensure_ascii=False,
                separators=(",", ":"),
            )
            + "\n"
        ).encode("utf-8")

    def _record_content(self, content: str) -> None:
        for chunk in self._utf8_chunks(content):
            if self._bytes_written >= self._JOURNAL_LIMIT_BYTES:
                return
            sequence = self._sequence + 1
            encoded = self._encoded_event(sequence, chunk)
            remaining = self._JOURNAL_LIMIT_BYTES - self._bytes_written
            if len(encoded) > remaining:
                # Keep the greatest code-point-safe prefix that still leaves a
                # valid bounded JSONL event. The aggregate journal budget, not
                # a per-callback truncation, is the authoritative boundary.
                lower = 0
                upper = len(chunk)
                prefix = ""
                while lower <= upper:
                    midpoint = (lower + upper) // 2
                    candidate = chunk[:midpoint]
                    if len(self._encoded_event(sequence, candidate)) <= remaining:
                        prefix = candidate
                        lower = midpoint + 1
                    else:
                        upper = midpoint - 1
                if not prefix:
                    return
                encoded = self._encoded_event(sequence, prefix)
            try:
                self._stream.write(encoded)
                self._stream.flush()
            except OSError:
                return
            self._sequence = sequence
            self._bytes_written += len(encoded)

    def close(self) -> None:
        with self._lock:
            try:
                for decoder in self._decoders.values():
                    self._record_content(decoder.decode(b"", final=True))
                self._stream.close()
            except OSError:
                pass


def _probe_process_token_pids(process_token: str) -> tuple[set[int], bool]:
    """Return token-bound processes and whether their absence was observable."""

    if os.name != "posix" or not process_token or not Path("/proc").is_dir():
        return set(), False
    marker = f"ALFREDO_PROCESS_TOKEN={process_token}".encode("utf-8")
    matches: set[int] = set()
    try:
        entries = os.scandir("/proc")
    except OSError:
        return matches, False
    with entries:
        for entry in entries:
            if not entry.name.isdigit():
                continue
            pid = int(entry.name)
            try:
                payload = _read_bounded_bytes(
                    Path(entry.path) / "environ",
                    1_000_000,
                )
            except FileNotFoundError:
                continue
            except OSError:
                # hidepid, sandbox, and transient I/O restrictions make an
                # absence claim unavailable; supervision must not collapse
                # that uncertainty into proof of quiescence.
                return matches, False
            if marker in payload.split(b"\0"):
                matches.add(pid)
    return matches, True


def _process_token_pids(process_token: str) -> set[int]:
    """Return observable live processes that inherited one bounded-run token."""

    matches, _absence_observable = _probe_process_token_pids(process_token)
    return matches


def _process_group_is_live(
    process: subprocess.Popen[Any],
    process_token: str = "",
) -> bool:
    if os.name != "posix":
        return process.poll() is None
    try:
        os.killpg(process.pid, 0)
    except ProcessLookupError:
        return bool(_process_token_pids(process_token))
    except PermissionError:
        return True
    return True


def _signal_process_group(
    process: subprocess.Popen[Any],
    signal_number: int,
    process_token: str = "",
) -> None:
    try:
        if os.name == "posix":
            os.killpg(process.pid, signal_number)
        elif process.poll() is None:
            if signal_number == signal.SIGTERM:
                process.terminate()
            else:
                process.kill()
    except ProcessLookupError:
        pass
    if os.name == "posix":
        for pid in _process_token_pids(process_token):
            try:
                os.kill(pid, signal_number)
            except (PermissionError, ProcessLookupError):
                continue


def _terminate_process_group(
    process: subprocess.Popen[Any],
    process_token: str = "",
) -> None:
    _signal_process_group(process, signal.SIGTERM, process_token)
    deadline = time.monotonic() + 1.0
    while (
        _process_group_is_live(process, process_token)
        and time.monotonic() < deadline
    ):
        process.poll()
        time.sleep(0.02)
    if _process_group_is_live(process, process_token):
        _signal_process_group(process, signal.SIGKILL, process_token)
    if process.poll() is None:
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            _signal_process_group(process, signal.SIGKILL, process_token)
            process.wait(timeout=1)


def _trusted_system_executable(name: str) -> str | None:
    """Resolve an enforcement helper without consulting caller-controlled PATH."""

    trusted_roots = {
        directory.resolve(strict=False)
        for directory in _TRUSTED_PROCESS_HELPER_DIRECTORIES
    }
    for directory in _TRUSTED_PROCESS_HELPER_DIRECTORIES:
        candidate = directory / name
        try:
            resolved = candidate.resolve(strict=True)
        except OSError:
            continue
        if (
            not resolved.is_file()
            or not os.access(resolved, os.X_OK)
            or not any(
                resolved == root or resolved.is_relative_to(root)
                for root in trusted_roots
            )
        ):
            continue
        return str(resolved)
    return None


def _resource_bounded_process_argv(argv: str | list[str]) -> str | list[str]:
    """Apply inherited host resource limits without unsafe pre-exec callbacks."""

    if os.name != "posix" or not isinstance(argv, list):
        return argv
    prlimit = _trusted_system_executable("prlimit")
    if prlimit is None:
        raise AlbertError(
            "Unable to start bounded process: trusted system prlimit is required."
        )
    return [
        prlimit,
        f"--as={_PROCESS_ADDRESS_SPACE_BYTES_LIMIT}",
        f"--fsize={_PROCESS_FILE_SIZE_BYTES_LIMIT}",
        f"--nofile={_PROCESS_OPEN_FILE_LIMIT}",
        f"--nproc={_PROCESS_COUNT_LIMIT}",
        "--",
        *argv,
    ]


def _process_isolated_argv(argv: str | list[str]) -> str | list[str]:
    """Place raw POSIX children in a kill-on-exit PID namespace."""

    if os.name != "posix" or not isinstance(argv, list):
        return _resource_bounded_process_argv(argv)
    bubblewrap = _trusted_system_executable("bwrap")
    if bubblewrap is None:
        raise AlbertError(
            "Unable to start bounded process: trusted system bubblewrap is required."
        )
    try:
        already_sandboxed = bool(argv) and Path(argv[0]).resolve(strict=True) == Path(
            bubblewrap
        )
    except OSError:
        already_sandboxed = False
    if already_sandboxed:
        # sandboxed_process_argv places prlimit inside the new namespaces so its
        # per-UID process limit cannot prevent Bubblewrap from creating them.
        return argv
    bounded = _resource_bounded_process_argv(argv)
    assert isinstance(bounded, list)
    supervisor = Path(__file__).with_name("process_supervisor.py")
    return [
        bubblewrap,
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--",
        sys.executable,
        str(supervisor),
        str(_PROCESS_DESCENDANT_GRACE_SECONDS),
        "--",
        *bounded,
    ]


def _run_bounded_process(
    argv: str | list[str],
    *,
    input_text: str | None = None,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    timeout_seconds: float,
    output_limit_bytes: int = _PROCESS_OUTPUT_BYTES_LIMIT,
    process_started: Callable[[subprocess.Popen[bytes]], None] | None = None,
    process_binding_started: (
        Callable[[subprocess.Popen[bytes], str], None] | None
    ) = None,
    poll_callback: Callable[[], None] | None = None,
    output_callback: Callable[[str, bytes], None] | None = None,
) -> subprocess.CompletedProcess[str]:
    """Run one child with bounded aggregate output and process-group termination."""

    process_token = secrets.token_hex(16)
    process_env = sanitized_process_environment(env)
    process_env["ALFREDO_PROCESS_TOKEN"] = process_token
    process = subprocess.Popen(
        _process_isolated_argv(argv),
        cwd=cwd,
        stdin=subprocess.PIPE if input_text is not None else None,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=process_env,
        shell=False,
        start_new_session=True,
    )
    capture = _BoundedProcessCapture(output_limit_bytes, output_callback=output_callback)
    capture.start(process)
    input_thread: threading.Thread | None = None

    def deliver_input() -> None:
        if process.stdin is None or input_text is None:
            return
        try:
            process.stdin.write(input_text.encode("utf-8"))
            process.stdin.flush()
        except (BrokenPipeError, OSError):
            pass
        finally:
            try:
                process.stdin.close()
            except OSError:
                pass

    try:
        if process_binding_started is not None:
            process_binding_started(process, process_token)
        if process_started is not None:
            process_started(process)
        started = time.monotonic()
        if input_text is not None:
            input_thread = threading.Thread(target=deliver_input, daemon=True)
            input_thread.start()
        exit_status: int | None = None
        extra_stderr = ""
        leader_exited_at: float | None = None
        while True:
            now = time.monotonic()
            leader_status = process.poll()
            if leader_status is not None and leader_exited_at is None:
                leader_exited_at = now
            if capture.exceeded.is_set():
                exit_status = _PROCESS_OUTPUT_LIMIT_EXIT_STATUS
                extra_stderr = (
                    "Process output exceeded the "
                    f"{output_limit_bytes}-byte aggregate limit and was terminated."
                )
                _terminate_process_group(process, process_token)
                break
            if now - started >= timeout_seconds:
                exit_status = 124
                extra_stderr = f"Process timed out after {timeout_seconds} seconds."
                _terminate_process_group(process, process_token)
                break
            if poll_callback is not None:
                poll_callback()
            group_live = _process_group_is_live(process, process_token)
            if (
                leader_status is not None
                and capture.drained.is_set()
                and not group_live
            ):
                break
            if (
                leader_exited_at is not None
                and now - leader_exited_at >= _PROCESS_DESCENDANT_GRACE_SECONDS
            ):
                exit_status = 124
                extra_stderr = (
                    "Process descendants timed out "
                    f"{_PROCESS_DESCENDANT_GRACE_SECONDS} seconds after the leader "
                    "exited and were terminated."
                )
                _terminate_process_group(process, process_token)
                break
            time.sleep(0.02)
        if exit_status is not None:
            capture.wait_for_eof(1.0)
        if capture.exceeded.is_set():
            exit_status = _PROCESS_OUTPUT_LIMIT_EXIT_STATUS
            extra_stderr = (
                "Process output exceeded the "
                f"{output_limit_bytes}-byte aggregate limit and was terminated."
            )
            _terminate_process_group(process, process_token)
            capture.wait_for_eof(1.0)
        stdout, stderr = capture.finish(
            extra_stderr=extra_stderr,
            join_timeout=1.0,
        )
        if capture.invalid_utf8.is_set():
            invalid_message = "Process output was not valid UTF-8 and was rejected."
            if extra_stderr:
                invalid_message = f"{extra_stderr.rstrip()}\n{invalid_message}"
            stdout = ""
            stderr = _bounded_process_output(
                invalid_message,
                limit_bytes=output_limit_bytes,
            )
            if exit_status is None:
                exit_status = _PROCESS_OUTPUT_LIMIT_EXIT_STATUS
        if input_thread is not None:
            input_thread.join(timeout=1)
        return subprocess.CompletedProcess(
            argv,
            process.returncode if exit_status is None else exit_status,
            stdout,
            stderr,
        )
    except BaseException:
        _terminate_process_group(process, process_token)
        capture.finish()
        if input_thread is not None:
            input_thread.join(timeout=1)
        raise


def _command_invocation(command: str) -> str | list[str]:
    """Preserve native Windows command-line parsing and POSIX argv parsing."""
    return command if os.name == "nt" else shlex.split(command)


def _runtime_identity_path(path: Path) -> str:
    """Collapse WSL aliases for case-insensitive Windows-mounted paths only."""
    value = path.as_posix()
    if re.match(r"^/mnt/[A-Za-z](?:/|$)", value):
        return value.casefold()
    return value


def _git_metadata_exists(root: Path) -> bool:
    marker = root / ".git"
    return marker.exists() or marker.is_symlink()


def _is_explicit_git_not_repository(
    completed: subprocess.CompletedProcess[str],
) -> bool:
    if completed.returncode != 128 or completed.stdout.strip():
        return False
    stderr = completed.stderr.replace("\r\n", "\n").strip()
    return bool(
        stderr == _GIT_NOT_REPOSITORY_MESSAGE
        or _GIT_NOT_REPOSITORY_BOUNDARY_PATTERN.fullmatch(stderr)
    )


def _positive_pid(value: Any) -> int | None:
    if isinstance(value, int) and not isinstance(value, bool) and value > 0:
        return value
    return None


def _optional_positive_pid(data: dict[str, Any], field_name: str) -> int | None:
    if field_name not in data or data[field_name] is None:
        return None
    value = data[field_name]
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise AlbertError(f"Local Agent session {field_name} is invalid")
    return value


def _optional_session_string(data: dict[str, Any], field_name: str) -> str:
    if field_name not in data:
        return ""
    value = data[field_name]
    if not isinstance(value, str):
        raise AlbertError(f"Local Agent session {field_name} is invalid")
    return value


def _optional_nonnegative_int(data: dict[str, Any], field_name: str) -> int:
    if field_name not in data:
        return 0
    value = data[field_name]
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise AlbertError(f"Local Agent session {field_name} is invalid")
    return value


def _validated_runner_result(data: dict[str, Any]) -> dict[str, Any]:
    raw = data.get("runner_result", {})
    if not isinstance(raw, dict):
        raise AlbertError("Local Agent session runner_result is invalid")
    if not raw:
        return {}
    required_strings = (
        "mission_id",
        "session_id",
        "runner_operation_id",
        "worktree_identity",
        "status",
        "runner_ended_at",
        "evidence_correlation_id",
        "digest",
    )
    if any(
        not isinstance(raw.get(field_name), str)
        for field_name in required_strings
    ):
        raise AlbertError("Local Agent session runner_result boundary is invalid")
    if not all(
        raw[field_name].strip()
        for field_name in (
            "mission_id",
            "session_id",
            "runner_operation_id",
            "worktree_identity",
            "status",
            "runner_ended_at",
            "digest",
        )
    ):
        raise AlbertError("Local Agent session runner_result identity is invalid")
    exit_status = raw.get("runner_exit_status")
    evidence = raw.get("evidence")
    artifacts = raw.get("artifacts")
    if (
        (exit_status is not None and (
            not isinstance(exit_status, int) or isinstance(exit_status, bool)
        ))
        or not isinstance(raw.get("evidence_valid"), bool)
        or (evidence is not None and not isinstance(evidence, dict))
        or not isinstance(artifacts, dict)
        or any(
            not isinstance(key, str) or not isinstance(value, str)
            for key, value in artifacts.items()
        )
        or not re.fullmatch(r"sha256:[0-9a-f]{64}", raw["digest"])
    ):
        raise AlbertError("Local Agent session runner_result payload is invalid")
    if raw["status"] not in {"completed", "evidence-ready", "failed"}:
        raise AlbertError("Local Agent session runner_result status is invalid")
    if (
        raw["session_id"] != data.get("session_id")
        or raw["runner_operation_id"] != data.get("runner_operation_id")
        or raw["worktree_identity"] != data.get("worktree_identity")
    ):
        raise AlbertError("Local Agent session runner_result boundary is invalid")
    digest_payload = {key: value for key, value in raw.items() if key != "digest"}
    expected_digest = "sha256:" + sha256(
        json.dumps(
            digest_payload,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
    ).hexdigest()
    if raw["digest"] != expected_digest:
        raise AlbertError("Local Agent session runner_result digest is invalid")
    return dict(raw)


def _process_identity(pid: int) -> str:
    """Return a Linux PID start identity so PID reuse cannot impersonate a runner."""
    if pid <= 0:
        return ""
    stat_path = Path(f"/proc/{pid}/stat")
    try:
        remainder = stat_path.read_text(encoding="utf-8").rsplit(")", 1)[1].split()
    except (OSError, IndexError, UnicodeError):
        return ""
    if len(remainder) <= 19:
        return ""
    return f"linux:{pid}:{remainder[19]}"


def _process_identity_is_live(pid: int | None, identity: str) -> bool:
    return bool(pid and identity and _process_identity(pid) == identity)


def sanitized_process_environment(
    overrides: dict[str, str] | None = None,
) -> dict[str, str]:
    """Return a small environment without inherited credentials or agent secrets."""
    source = os.environ.copy()
    if overrides:
        source.update(overrides)
    result = {key: value for key, value in source.items() if key in _PROCESS_ENV_ALLOWLIST}
    if overrides:
        result.update(
            {
                key: value
                for key, value in overrides.items()
                if key in _PROCESS_ENV_OVERRIDE_ALLOWLIST
            }
        )
    result.setdefault("PATH", os.defpath)
    result["HOME"] = "/tmp"
    result["TMPDIR"] = "/tmp"
    result["PYTHONDONTWRITEBYTECODE"] = "1"
    result["PIP_CACHE_DIR"] = "/tmp/pip-cache"
    result["npm_config_cache"] = "/tmp/npm-cache"
    return result


def sandboxed_process_argv(
    argv: str | list[str],
    *,
    working_directory: Path,
    readable_roots: tuple[Path, ...] = (),
    writable_roots: tuple[Path, ...] = (),
    readonly_bindings: tuple[tuple[Path, Path], ...] = (),
    allow_implicit_executable_bindings: bool = True,
) -> tuple[str | list[str], bool]:
    """Wrap a child command in a minimal bubblewrap filesystem view."""
    if os.name != "posix" or not isinstance(argv, list):
        return argv, False
    bubblewrap = _trusted_system_executable("bwrap")
    if not bubblewrap:
        return argv, False
    bounded_argv = _resource_bounded_process_argv(argv)
    assert isinstance(bounded_argv, list)
    command = [
        bubblewrap,
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--tmpfs",
        "/",
    ]
    for system_root in _PROCESS_SYSTEM_READ_ROOTS:
        if system_root.exists():
            command.extend(["--ro-bind", str(system_root), str(system_root)])
    command.extend(
        [
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
        ]
    )
    executable = shutil.which(argv[0]) if argv else None
    executable_bindings: set[tuple[Path, Path]] = set()
    if executable:
        executable_entry = Path(executable).absolute()
        if executable_entry.is_relative_to(Path("/tmp")):
            if executable_entry.is_symlink():
                raise AlbertError(
                    f"Executable {executable_entry} under /tmp must not be a symlink."
                )
            resolved_tmp = Path("/tmp").resolve()
            executable_path = executable_entry.resolve()
            if not (
                executable_path == resolved_tmp
                or executable_path.is_relative_to(resolved_tmp)
            ):
                raise AlbertError(
                    f"Executable {executable_entry} must resolve inside /tmp."
                )
        else:
            executable_path = executable_entry.resolve()
        if not any(
            executable_entry == root or executable_entry.is_relative_to(root)
            for root in _PROCESS_SYSTEM_READ_ROOTS
            if root.exists()
        ):
            executable_bindings.add((executable_path, executable_entry))
    interpreter = Path(argv[0]).name.casefold() if argv else ""
    if interpreter.startswith("python") or interpreter in {
        "node",
        "bash",
        "sh",
        "ruby",
    }:
        for token in argv[1:]:
            if token in {"-c", "-m", "-e"}:
                break
            if token.startswith("-"):
                continue
            candidate = Path(token)
            if candidate.is_absolute() and candidate.is_relative_to(Path("/tmp")):
                candidate_entry = candidate.absolute()
                if candidate_entry.is_symlink():
                    raise AlbertError(
                        f"Interpreter script {candidate_entry} under /tmp must not be a symlink."
                    )
                candidate_source = candidate_entry.resolve()
                resolved_tmp = Path("/tmp").resolve()
                if not (
                    candidate_source == resolved_tmp
                    or candidate_source.is_relative_to(resolved_tmp)
                ):
                    raise AlbertError(
                        f"Interpreter script {candidate_entry} must resolve inside /tmp."
                    )
                if candidate_source.exists():
                    if not candidate_source.is_file():
                        raise AlbertError(
                            f"Interpreter script {candidate_entry} must be a regular file."
                        )
                    executable_bindings.add(
                        (candidate_source, candidate_entry)
                    )
            break
    readonly_mounts = {
        path.resolve()
        for path in readable_roots
        if path.exists()
    }
    writable_mounts = {
        path.resolve()
        for path in writable_roots
        if path.exists()
    }
    readonly_mounts.difference_update(writable_mounts)
    for source in sorted(
        readonly_mounts,
        key=lambda path: (len(path.parts), path.as_posix()),
    ):
        command.extend(["--ro-bind", str(source), str(source)])
    for source in sorted(
        writable_mounts,
        key=lambda path: (len(path.parts), path.as_posix()),
    ):
        command.extend(["--bind", str(source), str(source)])
    covered_roots = readonly_mounts | writable_mounts
    if not allow_implicit_executable_bindings:
        executable_bindings.clear()
    executable_bindings = {
        (source, destination)
        for source, destination in executable_bindings
        if not any(
            destination == root or destination.is_relative_to(root)
            for root in covered_roots
        )
    }
    binding_parent_directories: set[Path] = set()
    for _source, destination in executable_bindings:
        parent = destination.parent
        while parent != parent.parent and parent not in {Path("/tmp")}:
            if any(
                parent == root or parent.is_relative_to(root)
                for root in covered_roots
            ):
                break
            binding_parent_directories.add(parent)
            parent = parent.parent
    for directory in sorted(
        binding_parent_directories,
        key=lambda path: (len(path.parts), path.as_posix()),
    ):
        command.extend(["--dir", str(directory)])
    explicit_readonly_bindings = {
        (source.resolve(), destination.absolute())
        for source, destination in readonly_bindings
        if source.exists()
    }
    for source, destination in sorted(
        executable_bindings | explicit_readonly_bindings,
        key=lambda item: (len(item[1].parts), item[1].as_posix()),
    ):
        command.extend(["--ro-bind", str(source), str(destination)])
    cwd = working_directory.resolve()
    command.extend(
        [
            "--chdir",
            str(cwd),
            "--",
            *bounded_argv,
        ]
    )
    return command, True


@dataclass
class IssueSlice:
    id: str
    slug: str
    title: str
    status: str
    tracker_status: str
    type: str
    risk: str
    suggested_agent: str
    assigned_agent: str
    what_to_build: str
    acceptance_criteria: list[str]
    blocked_by: list[str]
    source_path: str
    evidence_requirements: list[str] = field(default_factory=list)
    review_state: str = "needs-review"
    locked: bool = False
    notes: str = ""
    launch_order: int | None = None
    contract_overridden: bool = False

    def to_runtime(self) -> dict[str, Any]:
        data = {
            "assigned_agent": self.assigned_agent,
            "locked": self.locked,
            "notes": self.notes,
            "review_state": self.review_state,
            "status": self.status,
            "launch_order": self.launch_order,
            "contract_overridden": self.contract_overridden,
        }
        if self.contract_overridden:
            data.update(
                {
                    "acceptance_criteria": self.acceptance_criteria,
                    "blocked_by": self.blocked_by,
                    "evidence_requirements": self.evidence_requirements,
                    "type": self.type,
                    "risk": self.risk,
                    "what_to_build": self.what_to_build,
                }
            )
        return data

    def apply_runtime(self, data: dict[str, Any]) -> None:
        tracker_complete = self.tracker_status.lower() in {"complete", "completed"}
        for field_name in [
            "assigned_agent",
            "notes",
            "launch_order",
            "contract_overridden",
        ]:
            if field_name in data:
                setattr(self, field_name, data[field_name])
        if not tracker_complete:
            for field_name in ["locked", "review_state", "status"]:
                if field_name in data:
                    setattr(self, field_name, data[field_name])
        else:
            self.status = "complete"
            self.review_state = "complete"
            self.locked = True
        if self.contract_overridden:
            for field_name in [
                "acceptance_criteria",
                "blocked_by",
                "evidence_requirements",
                "type",
                "risk",
                "what_to_build",
            ]:
                if field_name in data:
                    setattr(self, field_name, data[field_name])


@dataclass
class EvidencePackage:
    changed_files: list[str] = field(default_factory=list)
    diff_summary: str = ""
    commands_run: list[str] = field(default_factory=list)
    test_results: str = ""
    known_risks: str = ""
    proposed_context_updates: str = ""
    artifact_links: list[str] = field(default_factory=list)

    def missing_fields(self) -> list[str]:
        missing: list[str] = []
        if not self.changed_files:
            missing.append("changed_files")
        if not self.diff_summary.strip():
            missing.append("diff_summary")
        if not self.commands_run:
            missing.append("commands_run")
        if not self.test_results.strip():
            missing.append("test_results")
        if not self.known_risks.strip():
            missing.append("known_risks")
        if not self.proposed_context_updates.strip():
            missing.append("proposed_context_updates")
        return missing

    def to_dict(self) -> dict[str, Any]:
        return {
            "changed_files": self.changed_files,
            "diff_summary": self.diff_summary,
            "commands_run": self.commands_run,
            "test_results": self.test_results,
            "known_risks": self.known_risks,
            "proposed_context_updates": self.proposed_context_updates,
            "artifact_links": self.artifact_links,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any] | None) -> "EvidencePackage | None":
        if not data:
            return None
        return cls(
            changed_files=list(data.get("changed_files", [])),
            diff_summary=str(data.get("diff_summary", "")),
            commands_run=list(data.get("commands_run", [])),
            test_results=str(data.get("test_results", "")),
            known_risks=str(data.get("known_risks", "")),
            proposed_context_updates=str(data.get("proposed_context_updates", "")),
            artifact_links=list(data.get("artifact_links", [])),
        )


@dataclass
class LocalAgentSession:
    session_id: str
    issue_id: str
    assigned_agent: str
    worktree_path: Path
    task_packet: dict[str, Any]
    status: str = "launched"
    cleanup_eligible: bool = False
    evidence: EvidencePackage | None = None
    evidence_valid: bool = False
    evidence_correlation_id: str = ""
    artifacts: dict[str, str] = field(default_factory=dict)
    runner_exit_status: int | None = None
    runner_started_at: str = ""
    runner_ended_at: str = ""
    repository_snapshot: dict[str, Any] = field(default_factory=dict)
    baseline_fingerprints: dict[str, str] = field(default_factory=dict)
    runner_pid: int | None = None
    runner_identity: str = ""
    runner_process_pid: int | None = None
    runner_process_identity: str = ""
    runner_process_token: str = ""
    runner_operation_id: str = ""
    worktree_identity: str = ""
    revision: int = 0
    automatic_recovery_count: int = 0
    supervision_receipt_id: str = ""
    runner_result: dict[str, Any] = field(default_factory=dict)
    cancel_requested_at: str = ""
    cancel_reason: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "session_id": self.session_id,
            "issue_id": self.issue_id,
            "assigned_agent": self.assigned_agent,
            "worktree_path": str(self.worktree_path),
            "task_packet": self.task_packet,
            "status": self.status,
            "cleanup_eligible": self.cleanup_eligible,
            "evidence": self.evidence.to_dict() if self.evidence else None,
            "evidence_valid": self.evidence_valid,
            "evidence_correlation_id": self.evidence_correlation_id,
            "artifacts": self.artifacts,
            "runner_exit_status": self.runner_exit_status,
            "runner_started_at": self.runner_started_at,
            "runner_ended_at": self.runner_ended_at,
            "repository_snapshot": self.repository_snapshot,
            "baseline_fingerprints": self.baseline_fingerprints,
            "runner_pid": self.runner_pid,
            "runner_identity": self.runner_identity,
            "runner_process_pid": self.runner_process_pid,
            "runner_process_identity": self.runner_process_identity,
            "runner_process_token": self.runner_process_token,
            "runner_operation_id": self.runner_operation_id,
            "worktree_identity": self.worktree_identity,
            "revision": self.revision,
            "automatic_recovery_count": self.automatic_recovery_count,
            "supervision_receipt_id": self.supervision_receipt_id,
            "runner_result": self.runner_result,
            "cancel_requested_at": self.cancel_requested_at,
            "cancel_reason": self.cancel_reason,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "LocalAgentSession":
        status = data.get("status", "launched")
        if (
            status == "launched"
            and not data.get("runner_started_at")
            and not data.get("runner_ended_at")
            and not data.get("evidence")
        ):
            status = "queued"
        runner_result = _validated_runner_result(data)
        return cls(
            session_id=data["session_id"],
            issue_id=data["issue_id"],
            assigned_agent=data["assigned_agent"],
            worktree_path=Path(data["worktree_path"]),
            task_packet=dict(data.get("task_packet", {})),
            status=status,
            cleanup_eligible=bool(data.get("cleanup_eligible", False)),
            evidence=EvidencePackage.from_dict(data.get("evidence")),
            evidence_valid=bool(data.get("evidence_valid", False)),
            evidence_correlation_id=str(data.get("evidence_correlation_id", "")),
            artifacts=dict(data.get("artifacts", {})),
            runner_exit_status=data.get("runner_exit_status"),
            runner_started_at=data.get("runner_started_at", ""),
            runner_ended_at=data.get("runner_ended_at", ""),
            repository_snapshot=dict(data.get("repository_snapshot", {})),
            baseline_fingerprints=dict(data.get("baseline_fingerprints", {})),
            runner_pid=_optional_positive_pid(data, "runner_pid"),
            runner_identity=_optional_session_string(data, "runner_identity"),
            runner_process_pid=_optional_positive_pid(data, "runner_process_pid"),
            runner_process_identity=_optional_session_string(
                data,
                "runner_process_identity",
            ),
            runner_process_token=_optional_session_string(
                data,
                "runner_process_token",
            ),
            runner_operation_id=_optional_session_string(
                data,
                "runner_operation_id",
            ),
            worktree_identity=_optional_session_string(data, "worktree_identity"),
            revision=_optional_nonnegative_int(data, "revision"),
            automatic_recovery_count=_optional_nonnegative_int(
                data,
                "automatic_recovery_count",
            ),
            supervision_receipt_id=_optional_session_string(
                data,
                "supervision_receipt_id",
            ),
            runner_result=runner_result,
            cancel_requested_at=str(data.get("cancel_requested_at", "")),
            cancel_reason=str(data.get("cancel_reason", "")),
        )


@dataclass(frozen=True)
class RunnerObservation:
    source_id: str
    source_incarnation: str
    sequence: int
    mission_id: str
    session_id: str
    session_revision: int
    runner_operation_id: str
    owner_signal: str
    process_group_signal: str
    worktree_identity: str
    result_signal: str
    result_digest: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "source_id": self.source_id,
            "source_incarnation": self.source_incarnation,
            "sequence": self.sequence,
            "mission_id": self.mission_id,
            "session_id": self.session_id,
            "session_revision": self.session_revision,
            "runner_operation_id": self.runner_operation_id,
            "owner_signal": self.owner_signal,
            "process_group_signal": self.process_group_signal,
            "worktree_identity": self.worktree_identity,
            "result_signal": self.result_signal,
            "result_digest": self.result_digest,
        }


@dataclass(frozen=True)
class SupervisionReceipt:
    receipt_id: str
    correlation_id: str
    outcome: str
    effect: str
    mission_id: str
    session_id: str
    attention_id: str = ""

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "SupervisionReceipt":
        return cls(
            receipt_id=str(data["receipt_id"]),
            correlation_id=str(data["correlation_id"]),
            outcome=str(data["outcome"]),
            effect=str(data["effect"]),
            mission_id=str(data["mission_id"]),
            session_id=str(data["session_id"]),
            attention_id=str(data.get("attention_id", "")),
        )


@dataclass
class ReviewDecision:
    session_id: str
    issue_id: str
    outcome: str
    reason: str
    next_action: str
    limitations: list[str] = field(default_factory=list)
    workspace_action: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "session_id": self.session_id,
            "issue_id": self.issue_id,
            "outcome": self.outcome,
            "reason": self.reason,
            "next_action": self.next_action,
            "limitations": self.limitations,
            "workspace_action": self.workspace_action,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "ReviewDecision":
        return cls(
            session_id=data["session_id"],
            issue_id=data["issue_id"],
            outcome=data["outcome"],
            reason=data.get("reason", ""),
            next_action=data.get("next_action", ""),
            limitations=list(data.get("limitations", [])),
            workspace_action=dict(data.get("workspace_action", {})),
        )


@dataclass
class DelegationDecision:
    issue_id: str
    router_agent: str
    recommended_agent: str
    complexity: str
    reason: str
    requires_approval: bool
    approved: bool = False

    def to_dict(self) -> dict[str, Any]:
        return {
            "issue_id": self.issue_id,
            "router_agent": self.router_agent,
            "recommended_agent": self.recommended_agent,
            "complexity": self.complexity,
            "reason": self.reason,
            "requires_approval": self.requires_approval,
            "approved": self.approved,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "DelegationDecision":
        return cls(
            issue_id=data["issue_id"],
            router_agent=data["router_agent"],
            recommended_agent=data["recommended_agent"],
            complexity=data.get("complexity", ""),
            reason=data.get("reason", ""),
            requires_approval=bool(data.get("requires_approval", False)),
            approved=bool(data.get("approved", False)),
        )


@dataclass
class PrPreparation:
    issue_id: str
    branch_name: str
    title: str
    body: str
    create_command: str
    merge_approved: bool = False


class AlbertMission:
    def __init__(
        self,
        target_repo: Path,
        tracker_dir: Path,
        runtime_root: Path,
        mission_id: str = "mission-001",
        agent_config_path: Path | None = None,
        allow_empty_tracker: bool = False,
        issues_dir: Path | None = None,
        agent_availability_snapshot: dict[str, tuple[str, str]] | None = None,
    ):
        self.target_repo = target_repo.resolve()
        self.tracker_dir = tracker_dir.resolve()
        self.issues_dir = (issues_dir or (self.tracker_dir / "issues")).resolve()
        self.runtime_root = runtime_root.resolve()
        self.mission_id = mission_id
        self.agent_config_path = (agent_config_path or (self.target_repo / ".albert" / "agents.json")).resolve()
        self.allow_empty_tracker = allow_empty_tracker
        self.agent_availability_snapshot = agent_availability_snapshot
        identity_paths = [
            _runtime_identity_path(path)
            for path in (self.target_repo, self.tracker_dir, self.issues_dir)
        ]
        runtime_identity = "\n".join([*identity_paths, self.mission_id])
        digest = sha1(runtime_identity.encode("utf-8")).hexdigest()[:8]
        identity_name = Path(identity_paths[0]).name
        self.project_key = f"{identity_name}-{digest}"
        self.prd_title = ""
        self.issues: dict[str, IssueSlice] = {}
        self.sessions: dict[str, LocalAgentSession] = {}
        self.reviews: list[ReviewDecision] = []
        self.delegations: dict[str, DelegationDecision] = {}
        self.command_policy: dict[str, str] = {}
        self.workstation_actions: dict[str, dict[str, Any]] = {}
        self.archived_issue_ids: set[str] = set()
        self.supervision: dict[str, Any] = self._empty_supervision_state()
        self.timeline: list[str] = []
        self.agent_registry = AgentRegistry(agents=[], source_path=self.agent_config_path)
        self._evidence_activity_recorder: (
            Callable[[str, LocalAgentSession, EvidencePackage], None] | None
        ) = None
        self._session_output_recorders: dict[str, _SessionOutputRecorder] = {}
        self._session_output_recorders_lock = threading.Lock()

    @property
    def runtime_dir(self) -> Path:
        return self.runtime_root / self.project_key

    @property
    def runtime_path(self) -> Path:
        return self.runtime_dir / "runtime.json"

    @staticmethod
    def _empty_supervision_state() -> dict[str, Any]:
        return {
            "schema_version": 1,
            "observers": {},
            "attentions": {},
            "intents": {},
            "receipts": {},
        }

    @classmethod
    def _validated_supervision_state(cls, raw: Any) -> dict[str, Any]:
        """Copy and validate the durable supervision ledger container."""

        if not isinstance(raw, dict) or raw.get("schema_version") != 1:
            raise AlbertError("Mission supervision state is invalid")
        for field_name in ("observers", "attentions", "intents", "receipts"):
            collection = raw.get(field_name)
            if not isinstance(collection, dict) or not all(
                isinstance(key, str)
                and key.strip()
                and isinstance(value, dict)
                for key, value in collection.items()
            ):
                raise AlbertError("Mission supervision state is invalid")
        for source_id, observer in raw["observers"].items():
            cursor = observer.get("cursor")
            observer_receipts = observer.get("receipts")
            if (
                not isinstance(observer.get("incarnation"), str)
                or not observer["incarnation"].strip()
                or not isinstance(cursor, int)
                or isinstance(cursor, bool)
                or cursor < 0
                or not isinstance(observer_receipts, dict)
                or any(
                    not isinstance(sequence, str)
                    or not sequence.isdigit()
                    or not isinstance(receipt_id, str)
                    or not receipt_id.strip()
                    for sequence, receipt_id in observer_receipts.items()
                )
            ):
                raise AlbertError(
                    f"Mission supervision observer state is invalid: {source_id}"
                )
        cls._validate_supervision_records(
            raw["receipts"],
            required_strings=(
                "receipt_id",
                "correlation_id",
                "outcome",
                "effect",
                "mission_id",
                "session_id",
            ),
            identity_field="receipt_id",
        )
        cls._validate_supervision_records(
            raw["attentions"],
            required_strings=(
                "attention_id",
                "incident_id",
                "mission_id",
                "session_id",
                "kind",
                "detail",
                "next_effect",
                "disposition",
                "receipt_id",
            ),
            identity_field="attention_id",
        )
        cls._validate_supervision_records(
            raw["intents"],
            required_strings=(
                "intent_id",
                "attention_id",
                "receipt_id",
                "effect",
                "status",
                "mission_id",
                "session_id",
            ),
            identity_field="intent_id",
        )
        if any(
            receipt["outcome"]
            not in {
                "no-change",
                "attention-recorded",
                "recovered",
                "result-reconciled",
                "decision-needed",
            }
            or receipt["effect"]
            not in {
                "none",
                "recover-same-session",
                "reconcile-result",
                "mission-commander-decision",
            }
            for receipt in raw["receipts"].values()
        ):
            raise AlbertError("Mission supervision receipt outcome is invalid")
        if any(
            attention["next_effect"]
            not in {
                "recover-same-session",
                "reconcile-result",
                "mission-commander-decision",
            }
            or attention["disposition"] not in {"open", "resolved"}
            for attention in raw["attentions"].values()
        ):
            raise AlbertError("Mission supervision attention state is invalid")
        if any(
            intent["effect"]
            not in {"recover-same-session", "reconcile-result"}
            or intent["status"] not in {"pending", "applied", "blocked"}
            for intent in raw["intents"].values()
        ):
            raise AlbertError("Mission supervision intent state is invalid")
        receipts = raw["receipts"]
        attentions = raw["attentions"]
        for observer in raw["observers"].values():
            cursor = observer["cursor"]
            expected_sequences = {str(sequence) for sequence in range(1, cursor + 1)}
            if set(observer["receipts"]) != expected_sequences or any(
                receipt_id not in receipts
                for receipt_id in observer["receipts"].values()
            ):
                raise AlbertError(
                    "Mission supervision observer receipt chain is invalid"
                )
        for receipt_id, receipt in receipts.items():
            attention_id = receipt.get("attention_id", "")
            if not isinstance(attention_id, str) or (
                receipt["effect"] == "none" and attention_id
            ) or (
                receipt["effect"] != "none"
                and (
                    not attention_id
                    or attention_id not in attentions
                    or attentions[attention_id]["receipt_id"] != receipt_id
                )
            ):
                raise AlbertError(
                    "Mission supervision receipt attention chain is invalid"
                )
        for intent in raw["intents"].values():
            revision = intent.get("session_revision")
            result_signal = intent.get("result_signal")
            result_digest = intent.get("result_digest")
            if (
                intent["receipt_id"] not in receipts
                or intent["attention_id"] not in attentions
                or attentions[intent["attention_id"]]["receipt_id"]
                != intent["receipt_id"]
                or not isinstance(revision, int)
                or isinstance(revision, bool)
                or revision < 0
                or any(
                    not isinstance(intent.get(field_name), str)
                    for field_name in (
                        "runner_operation_id",
                        "runner_identity",
                        "runner_process_identity",
                        "worktree_identity",
                        "result_signal",
                        "result_digest",
                    )
                )
                or any(
                    intent.get(field_name) is not None
                    and (
                        not isinstance(intent[field_name], int)
                        or isinstance(intent[field_name], bool)
                        or intent[field_name] <= 0
                    )
                    for field_name in ("runner_pid", "runner_process_pid")
                )
                or result_signal not in {"absent", "exact-valid"}
                or (result_signal == "exact-valid" and not result_digest)
                or (result_signal == "absent" and result_digest)
            ):
                raise AlbertError("Mission supervision intent boundary is invalid")
        return json.loads(json.dumps(raw))

    @staticmethod
    def _validate_supervision_records(
        records: dict[str, dict[str, Any]],
        *,
        required_strings: tuple[str, ...],
        identity_field: str,
    ) -> None:
        for record_id, record in records.items():
            if record.get(identity_field) != record_id or any(
                not isinstance(record.get(field_name), str)
                or not record[field_name].strip()
                for field_name in required_strings
            ):
                raise AlbertError(
                    f"Mission supervision {identity_field} record is invalid: {record_id}"
                )

    def _supervision_from_payload(self, data: dict[str, Any]) -> dict[str, Any]:
        raw = data.get("supervision", self._empty_supervision_state())
        supervision = self._validated_supervision_state(raw)
        self._validate_supervision_semantics(
            supervision,
            data.get("sessions", {}),
        )
        return supervision

    def _validate_supervision_semantics(
        self,
        supervision: dict[str, Any],
        raw_sessions: Any,
    ) -> None:
        """Reject well-shaped records that contradict their causal boundary."""

        if not isinstance(raw_sessions, dict) or any(
            not isinstance(session_id, str)
            or not session_id.strip()
            or not isinstance(raw_session, dict)
            for session_id, raw_session in raw_sessions.items()
        ):
            raise AlbertError("Mission supervision session boundary is invalid")
        for session_id, raw_session in raw_sessions.items():
            result = raw_session.get("runner_result", {})
            if isinstance(result, dict) and result and (
                result.get("mission_id") != self.mission_id
                or result.get("session_id") != session_id
            ):
                raise AlbertError("Mission runner result identity is invalid")

        receipts = supervision["receipts"]
        attentions = supervision["attentions"]
        intents = supervision["intents"]
        allowed_receipt_outcomes = {
            "none": {"no-change"},
            "recover-same-session": {"attention-recorded", "recovered"},
            "reconcile-result": {"attention-recorded", "result-reconciled"},
            "mission-commander-decision": {"decision-needed"},
        }
        for receipt_id, receipt in receipts.items():
            effect = receipt["effect"]
            session_id = receipt["session_id"]
            attention_id = receipt.get("attention_id", "")
            if (
                receipt["mission_id"] != self.mission_id
                or session_id not in raw_sessions
                or receipt["outcome"] not in allowed_receipt_outcomes[effect]
            ):
                raise AlbertError("Mission supervision receipt boundary is invalid")
            if effect == "none":
                continue
            attention = attentions.get(attention_id)
            if not isinstance(attention, dict) or (
                attention["mission_id"] != self.mission_id
                or attention["session_id"] != session_id
                or attention["next_effect"] != effect
            ):
                raise AlbertError("Mission supervision attention boundary is invalid")

        referenced_attention_ids = {
            str(receipt.get("attention_id", ""))
            for receipt in receipts.values()
            if receipt.get("attention_id")
        }
        if set(attentions) != referenced_attention_ids:
            raise AlbertError("Mission supervision attention ownership is invalid")

        intents_by_receipt: dict[str, list[dict[str, Any]]] = {}
        for intent in intents.values():
            receipt = receipts[intent["receipt_id"]]
            attention = attentions[intent["attention_id"]]
            status = intent["status"]
            effect = intent["effect"]
            expected_projection_effect = (
                "mission-commander-decision" if status == "blocked" else effect
            )
            if (
                intent["mission_id"] != self.mission_id
                or intent["session_id"] not in raw_sessions
                or receipt["mission_id"] != intent["mission_id"]
                or attention["mission_id"] != intent["mission_id"]
                or receipt["session_id"] != intent["session_id"]
                or attention["session_id"] != intent["session_id"]
                or receipt["effect"] != expected_projection_effect
                or attention["next_effect"] != expected_projection_effect
                or (
                    effect == "recover-same-session"
                    and (
                        intent["result_signal"] != "absent"
                        or bool(intent["result_digest"])
                    )
                )
                or (
                    effect == "reconcile-result"
                    and (
                        intent["result_signal"] != "exact-valid"
                        or not intent["result_digest"]
                    )
                )
            ):
                raise AlbertError("Mission supervision intent semantics are invalid")
            if status == "pending":
                raw_session = raw_sessions[intent["session_id"]]
                boundary_fields = (
                    ("revision", "session_revision"),
                    ("runner_operation_id", "runner_operation_id"),
                    ("runner_pid", "runner_pid"),
                    ("runner_identity", "runner_identity"),
                    ("runner_process_pid", "runner_process_pid"),
                    ("runner_process_identity", "runner_process_identity"),
                    ("worktree_identity", "worktree_identity"),
                )
                if any(
                    raw_session.get(session_field) != intent.get(intent_field)
                    for session_field, intent_field in boundary_fields
                ):
                    raise AlbertError(
                        "Mission supervision pending intent boundary is invalid"
                    )
            intents_by_receipt.setdefault(intent["receipt_id"], []).append(intent)

        for receipt_id, receipt in receipts.items():
            intent_count = len(intents_by_receipt.get(receipt_id, []))
            if receipt["effect"] in {"recover-same-session", "reconcile-result"}:
                if intent_count != 1:
                    raise AlbertError("Mission supervision receipt intent is invalid")
            elif intent_count:
                linked_intent = intents_by_receipt[receipt_id][0]
                if not (
                    receipt["effect"] == "mission-commander-decision"
                    and intent_count == 1
                    and linked_intent["status"] == "blocked"
                ):
                    raise AlbertError("Mission supervision receipt intent is invalid")

    def supervision_state(self) -> dict[str, Any]:
        return json.loads(json.dumps(self.supervision))

    def _start_session_output(self, session: LocalAgentSession) -> None:
        recorder = _SessionOutputRecorder(
            self.runtime_dir / "sessions" / session.session_id / "output-events.jsonl",
            self.mission_id,
            session.session_id,
        )
        with self._session_output_recorders_lock:
            previous = self._session_output_recorders.pop(session.session_id, None)
            if previous is not None:
                previous.close()
            self._session_output_recorders[session.session_id] = recorder

    def _finish_session_output(self, session_id: str) -> None:
        with self._session_output_recorders_lock:
            recorder = self._session_output_recorders.pop(session_id, None)
        if recorder is not None:
            recorder.close()

    def _record_session_output(self, session_id: str, stream_name: str, payload: bytes) -> None:
        with self._session_output_recorders_lock:
            recorder = self._session_output_recorders.get(session_id)
        if recorder is not None:
            recorder.record(stream_name, payload)

    def load(self) -> "AlbertMission":
        self.agent_registry = load_agent_registry(self.agent_config_path)
        if self.agent_availability_snapshot is not None:
            refreshed_agents: list[AgentConfig] = []
            for agent in self.agent_registry.agents:
                availability, availability_reason = self.agent_availability_snapshot.get(
                    agent.id,
                    (agent.availability, agent.availability_reason),
                )
                if availability not in {"available", "unavailable", "disconnected"}:
                    raise AlbertError(
                        f"Agent availability snapshot for {agent.id} has unknown state "
                        f"{availability!r}."
                    )
                if agent.availability != "available":
                    availability = agent.availability
                    availability_reason = agent.availability_reason
                refreshed_agents.append(
                    replace(
                        agent,
                        availability=availability,
                        availability_reason=availability_reason,
                    )
                )
            self.agent_registry = AgentRegistry(
                agents=refreshed_agents,
                source_path=self.agent_registry.source_path,
            )
        prd_path = self.tracker_dir / "PRD.md"
        if self.allow_empty_tracker and not prd_path.exists():
            self.prd_title = "Untracked Workspace"
        else:
            self.prd_title = self._load_prd_title()
        self.issues = self._load_issues()
        runtime_exists = self.runtime_path.exists()
        self._load_runtime()
        if runtime_exists:
            self._reconcile_abandoned_sessions()
        else:
            self._persist()
        return self

    def board_summary(self) -> dict[str, Any]:
        ordered = self.ordered_issue_ids()
        ready = [issue_id for issue_id in ordered if self._issue_launch_eligible(self.issues[issue_id])]
        return {
            "prd_title": self.prd_title,
            "issue_count": len(self.issues),
            "ordered_issue_ids": ordered,
            "ready_issue_ids": ready,
            "approved_issue_ids": [issue_id for issue_id in ordered if self.issues[issue_id].review_state == "approved"],
            "issue_slices": [self._issue_summary(issue_id) for issue_id in ordered],
        }

    def _issue_summary(self, issue_id: str) -> dict[str, Any]:
        issue = self.issues[issue_id]
        blockers = [
            {
                "issue_id": blocker_id,
                "title": self.issues[blocker_id].title,
                "lifecycle": self._issue_lifecycle(self.issues[blocker_id]),
                "satisfied": self._lifecycle_satisfies_blocker(self.issues[blocker_id]),
            }
            for blocker_id in issue.blocked_by
        ]
        sessions = [
            self._session_summary(session)
            for session in sorted(self.sessions.values(), key=lambda item: item.session_id)
            if session.issue_id == issue.id
        ]
        latest_evidence = next(
            (session["evidence"] for session in reversed(sessions) if session.get("evidence")),
            None,
        )
        return {
            "issue_id": issue.id,
            "title": issue.title,
            "work_type": issue.type,
            "tracker_status": issue.tracker_status,
            "lifecycle": self._issue_lifecycle(issue),
            "progress": self._issue_progress(issue),
            "launch_eligible": self._issue_launch_eligible(issue),
            "blockers": blockers,
            "accepted_boundary": {
                "what_to_build": issue.what_to_build,
                "acceptance_criteria": issue.acceptance_criteria,
                "evidence_requirements": issue.evidence_requirements or self.default_evidence_requirements(),
                "source_path": issue.source_path,
            },
            "sessions": sessions,
            "provenance": self._issue_provenance(issue, sessions),
            "model_assignment": self._model_assignment(issue, sessions),
            "evidence": latest_evidence
            or {
                "state": "missing",
                "changed_files": [],
                "commands_run": [],
                "test_results": "No evidence package recorded.",
                "risks": "None recorded.",
                "artifact_links": [],
            },
            "working_context_sources": [
                {
                    "source_id": f"shared-context:{self.mission_id}:issue-slice:{issue.id}",
                    "kind": "shared-context",
                    "label": f"Shared Context — {issue.title}",
                },
                {
                    "source_id": f"issue:{self.mission_id}:{issue.id}",
                    "kind": "unresolved-item",
                    "label": f"{issue.id} — {issue.title}",
                },
            ],
        }

    def _issue_launch_eligible(self, issue: IssueSlice) -> bool:
        return (
            issue.review_state == "approved"
            and self._assignment_available(issue)
            and "launch" in self._next_actions_for_issue(issue)
        )

    def _issue_lifecycle(self, issue: IssueSlice) -> str:
        if issue.tracker_status.lower() in {"merged"}:
            return "Merged"
        if issue.review_state == "complete":
            return "Merged"
        if issue.review_state == "pr-ready":
            return "Complete"
        if self._issue_launch_eligible(issue):
            return "Ready"
        labels = {
            "approved": "Approved",
            "needs-review": "Needs review",
            "needs-human-review": "Needs human review",
            "needs-repair": "Needs repair",
            "rejected": "Rejected",
        }
        return labels.get(issue.review_state, issue.review_state.replace("-", " ").title())

    def _issue_progress(self, issue: IssueSlice) -> str:
        assignment = self._model_assignment(issue, [])
        if issue.review_state == "approved" and assignment["availability"] != "available":
            reason = assignment["availability_reason"] or assignment["availability"]
            return f"Assigned model unavailable: {reason}"
        if issue.review_state == "approved" and not self._assignment_authorized(issue):
            return (
                "Assigned agent is not launchable: choose an assignable worker or "
                "route through an approved delegate."
            )
        if self._issue_launch_eligible(issue):
            return "Launch eligible"
        if issue.blocked_by:
            unsatisfied = [
                blocker
                for blocker in issue.blocked_by
                if not self._lifecycle_satisfies_blocker(self.issues[blocker])
            ]
            if unsatisfied:
                return f"Waiting on {', '.join(unsatisfied)}"
        if issue.review_state == "pr-ready":
            return "Evidence accepted and PR-ready"
        if issue.review_state == "complete":
            return "Merged"
        return self._issue_lifecycle(issue)

    def _session_summary(self, session: LocalAgentSession) -> dict[str, Any]:
        agent_config = session.task_packet.get("agent_config") or {}
        evidence = session.evidence.to_dict() if session.evidence else None
        if evidence is not None:
            evidence["artifact_links"] = self.review_artifact_links(session)
        review = self._latest_review_for_session(session.session_id)
        evidence_accepted = bool(
            review
            and review.outcome in {"Approved", "Approved with limitations"}
        )
        disconnected = bool(
            session.runner_started_at
            and not session.runner_ended_at
            and session.status not in {"evidence-ready", "reviewed", "complete"}
            and (
                session.status != "running"
                or not _process_identity_is_live(
                    session.runner_pid,
                    session.runner_identity,
                )
            )
        )
        return {
            "session_id": session.session_id,
            "assigned_agent": session.assigned_agent,
            "role": str(agent_config.get("role", "local-agent")),
            "provider": str(agent_config.get("provider", "unconfigured")),
            "model": str(agent_config.get("model", session.assigned_agent)),
            "status": session.status,
            "stale": session.status in {"failed", "needs-repair", "rejected"},
            "disconnected": disconnected,
            "operation_status": self._session_operation_status(session, disconnected),
            "failure": self._session_failure(session),
            "evidence": self._evidence_summary(
                evidence,
                accepted=evidence_accepted,
            ),
        }

    @staticmethod
    def _artifact_is_safe_for_review(artifact_key: str) -> bool:
        """Return whether an artifact contains governed, review-safe metadata."""

        return artifact_key in {
            "result",
            "review_diff",
            "completion",
            "fake_log",
        } or artifact_key.endswith("_result")

    def review_artifact_links(self, session: LocalAgentSession) -> list[str]:
        """Project evidence links without exposing raw prompts, output, or logs."""

        if session.evidence is None:
            return []
        links: list[str] = []
        for artifact_link in session.evidence.artifact_links:
            if not self._artifact_link_is_safe_for_review(session, artifact_link):
                continue
            registered_keys = sorted(
                artifact_key
                for artifact_key, artifact_path in session.artifacts.items()
                if artifact_path == artifact_link
            )
            projected = (
                self.review_artifact_reference(session, registered_keys[0])
                if registered_keys
                else artifact_link
            )
            if projected not in links:
                links.append(projected)
        return links

    def review_artifact_reference(
        self,
        session: LocalAgentSession,
        artifact_key: str,
    ) -> str:
        """Return an app-local identifier rather than a registered host path."""

        filenames = {
            "review_diff": "review.diff",
            "result": "runner-result.json",
            "completion": "completion.json",
            "fake_log": "fake-agent.log",
        }
        filename = filenames.get(
            artifact_key,
            f"{artifact_key.replace('_', '-')}.json"
            if artifact_key.endswith("_result")
            else artifact_key.replace("_", "-"),
        )
        return (
            f"app-local://missions/{quote(self.mission_id, safe='')}/sessions/"
            f"{quote(session.session_id, safe='')}/artifacts/"
            f"{quote(artifact_key, safe='')}/{quote(filename, safe='')}"
        )

    def _artifact_link_is_safe_for_review(
        self,
        session: LocalAgentSession,
        artifact_link: str,
    ) -> bool:
        if not isinstance(artifact_link, str) or not artifact_link.strip():
            return False
        if any(ord(character) < 32 for character in artifact_link):
            return False
        registered_keys = [
            artifact_key
            for artifact_key, artifact_path in session.artifacts.items()
            if artifact_path == artifact_link
        ]
        if registered_keys:
            return all(
                self._artifact_is_safe_for_review(artifact_key)
                for artifact_key in registered_keys
            )
        if artifact_link.startswith("app-local://"):
            return True
        if artifact_link.startswith("artifact://evidence/"):
            return True
        return False

    def _automated_review_artifact_links(
        self, session: LocalAgentSession
    ) -> list[str]:
        links: list[str] = []
        for artifact_key, artifact_path in session.artifacts.items():
            if (
                self._artifact_is_safe_for_review(artifact_key)
                and artifact_path not in links
            ):
                links.append(artifact_path)
        return links

    @staticmethod
    def _session_operation_status(session: LocalAgentSession, disconnected: bool) -> str:
        if disconnected:
            return "streaming"
        if session.status == "failed":
            return "failed"
        if session.status in {"queued", "running", "evidence-ready", "reviewed", "complete"}:
            return session.status
        if session.runner_ended_at:
            return "completed"
        return "idle" if session.status == "launched" else session.status

    @staticmethod
    def _session_failure(session: LocalAgentSession) -> str:
        if session.status != "failed":
            return ""
        if session.evidence and session.evidence.known_risks:
            return session.evidence.known_risks
        return "Provider operation failed; inspect session evidence."

    @staticmethod
    def _evidence_summary(
        evidence: dict[str, Any] | None,
        *,
        accepted: bool = False,
    ) -> dict[str, Any]:
        if evidence is None:
            return {
                "state": "missing",
                "changed_files": [],
                "commands_run": [],
                "test_results": "No evidence package recorded.",
                "risks": "None recorded.",
                "artifact_links": [],
            }
        return {
            "state": "accepted" if accepted else "ready-for-review",
            "changed_files": evidence["changed_files"],
            "commands_run": evidence["commands_run"],
            "test_results": evidence["test_results"],
            "risks": evidence["known_risks"],
            "artifact_links": evidence["artifact_links"],
        }

    def _issue_provenance(
        self, issue: IssueSlice, sessions: list[dict[str, Any]]
    ) -> dict[str, str]:
        if sessions:
            latest = sessions[-1]
            return {
                "role": latest["role"],
                "provider": latest["provider"],
                "model": latest["model"],
            }
        agent = self.agent_registry.find(issue.assigned_agent)
        if agent:
            return {"role": agent.role, "provider": agent.provider, "model": agent.model}
        return {
            "role": "local-agent",
            "provider": "unconfigured",
            "model": issue.assigned_agent or issue.suggested_agent,
        }

    def _model_assignment(self, issue: IssueSlice, sessions: list[dict[str, Any]]) -> dict[str, str]:
        agent = self.agent_registry.find(issue.assigned_agent)
        if agent:
            role = agent.role
            provider = agent.provider
            model = agent.model
            availability = agent.availability
            availability_reason = agent.availability_reason
        else:
            role = "local-agent"
            provider = "unconfigured"
            model = issue.assigned_agent or issue.suggested_agent
            availability = (
                "unavailable" if self.agent_registry.configured else "available"
            )
            availability_reason = (
                f"Agent {issue.assigned_agent!r} is not configured."
                if self.agent_registry.configured
                else ""
            )
        latest = sessions[-1] if sessions else None
        operation_status = "idle"
        failure = ""
        if latest:
            operation_status = str(latest.get("operation_status", "idle"))
            failure = str(latest.get("failure", ""))
        return {
            "agent_id": issue.assigned_agent,
            "role": role,
            "provider": provider,
            "model": model,
            "availability": availability,
            "availability_reason": availability_reason,
            "operation_status": operation_status,
            "failure": failure,
        }

    def _assignment_available(self, issue: IssueSlice) -> bool:
        agent = self.agent_registry.find(issue.assigned_agent)
        if agent is None:
            return not self.agent_registry.configured
        return self._assignment_authorized(issue) and agent.availability == "available"

    def _assignment_authorized(self, issue: IssueSlice) -> bool:
        agent = self.agent_registry.find(issue.assigned_agent)
        if agent is None:
            return not self.agent_registry.configured
        if agent in self.assignment_agents():
            return True
        delegation = self.delegations.get(issue.id)
        return bool(
            agent.delegate_only
            and delegation is not None
            and delegation.recommended_agent == agent.id
        )

    def issue_detail(self, issue_id: str) -> dict[str, Any]:
        issue = self._issue(issue_id)
        blockers = [
            {
                "issue_id": blocker,
                "review_state": self._issue(blocker).review_state,
                "satisfied": self._lifecycle_satisfies_blocker(self._issue(blocker)),
            }
            for blocker in issue.blocked_by
        ]
        return {
            "issue_id": issue.id,
            "title": issue.title,
            "tracker_status": issue.tracker_status,
            "runtime_status": issue.status,
            "review_state": issue.review_state,
            "locked": issue.locked,
            "assigned_agent": issue.assigned_agent,
            "notes": issue.notes,
            "blockers": blockers,
            "acceptance_criteria": issue.acceptance_criteria,
            "next_actions": self._next_actions_for_issue(issue),
            "available_agents": [agent.id for agent in self.assignment_agents()],
            "delegation": self.delegations.get(issue.id).to_dict() if issue.id in self.delegations else None,
            "source_path": issue.source_path,
        }

    def ordered_issue_ids(self) -> list[str]:
        result: list[str] = []
        visiting: set[str] = set()
        visited: set[str] = set()

        def visit(issue_id: str) -> None:
            if issue_id in visited:
                return
            if issue_id in visiting:
                raise AlbertError(f"Cycle detected in Issue Graph at {issue_id}.")
            if issue_id not in self.issues:
                raise AlbertError(f"Unknown blocker {issue_id}.")
            visiting.add(issue_id)
            for blocker in self.issues[issue_id].blocked_by:
                visit(blocker)
            visiting.remove(issue_id)
            visited.add(issue_id)
            result.append(issue_id)

        for issue_id in sorted(self.issues):
            visit(issue_id)
        return result

    def approve_issue(
        self,
        issue_id: str,
        *,
        workstation_action: dict[str, Any] | None = None,
    ) -> None:
        issue = self._issue(issue_id)
        if issue.review_state == "complete":
            raise AlbertError(f"{issue_id} is already complete and cannot be approved for launch.")
        if workstation_action:
            self._remember_workstation_action(workstation_action)
        issue.review_state = "approved"
        issue.status = "approved"
        issue.locked = True
        self._record(f"{issue_id} approved and locked.")
        self._persist()

    def unlock_issue(self, issue_id: str, reason: str) -> None:
        issue = self._issue(issue_id)
        issue.locked = False
        issue.review_state = "needs-review"
        issue.status = "needs-review"
        self._record(f"{issue_id} unlocked for re-review: {reason}")
        self._persist()

    def reopen_issue(self, issue_id: str, reason: str) -> None:
        if not reason.strip():
            raise AlbertError("Reopen requires a reason.")
        issue = self._issue(issue_id)
        issue.locked = False
        issue.review_state = "needs-review"
        issue.status = "needs-review"
        issue.tracker_status = "needs-review"
        self._write_tracker_status(issue, "needs-review")
        self._record(f"{issue_id} reopened for re-review: {reason}")
        self._persist()

    def _apply_governed_issue_change(
        self,
        issue_id: str,
        *,
        proposed_changes: dict[str, Any],
        reason: str,
        action_marker: dict[str, Any],
    ) -> bool:
        """Atomically apply one Queue-authorized Issue change and its recovery marker."""

        correlation_id = action_marker.get("correlation_id")
        if not isinstance(correlation_id, str) or not correlation_id.strip():
            raise AlbertError("Governed Issue change correlation id must not be empty.")
        with self._runtime_lock(exclusive=True):
            if self.runtime_path.exists():
                self._load_runtime()
            existing = self.workstation_actions.get(correlation_id)
            if existing is not None:
                if existing != action_marker:
                    raise AlbertError(
                        "Governed Issue change correlation id was already used for a "
                        "different request boundary."
                    )
                return False
            issue = self._issue(issue_id)
            issue.locked = False
            for field_name, value in proposed_changes.items():
                if field_name in {
                    "acceptance_criteria",
                    "blocked_by",
                    "evidence_requirements",
                }:
                    setattr(issue, field_name, list(value))
                elif field_name in {"what_to_build", "type", "risk"}:
                    setattr(issue, field_name, value)
                else:
                    raise AlbertError(
                        f"Unknown governed field in Issue Change Proposal: {field_name}"
                    )
            issue.contract_overridden = True
            issue.review_state = "needs-review"
            issue.status = "needs-review"
            self._remember_workstation_action(action_marker)
            self._record(f"{issue_id} changed through Workspace Queue: {reason}")
            self._write_runtime_payload(self._runtime_payload())
            return True

    def assign_issue(
        self,
        issue_id: str,
        assigned_agent: str,
        notes: str = "",
        launch_order: int | None = None,
        *,
        workstation_action: dict[str, Any] | None = None,
    ) -> None:
        issue = self._issue(issue_id)
        if self.agent_registry.configured:
            agent = self.agent_registry.find(assigned_agent)
            if not agent:
                raise AlbertError(f"Unknown configured agent: {assigned_agent}")
            if agent not in self.assignment_agents():
                raise AlbertError(
                    f"{assigned_agent} is not manually assignable; use an available worker "
                    "or let the Frontier router select delegate-only agents."
                )
        if workstation_action:
            self._remember_workstation_action(workstation_action)
        issue.assigned_agent = assigned_agent
        if notes:
            issue.notes = notes
        if launch_order is not None:
            issue.launch_order = launch_order
        self._record(f"{issue_id} assigned to {assigned_agent}.")
        self._persist()

    def update_issue_contract(
        self,
        issue_id: str,
        *,
        what_to_build: str | None = None,
        acceptance_criteria: list[str] | None = None,
        blocked_by: list[str] | None = None,
        type: str | None = None,
        risk: str | None = None,
        evidence_requirements: list[str] | None = None,
    ) -> None:
        issue = self._issue(issue_id)
        if issue.locked:
            raise LockedFieldError(f"{issue_id} is approved and locked. Unlock before editing contract fields.")
        if what_to_build is not None:
            issue.what_to_build = what_to_build
        if acceptance_criteria is not None:
            issue.acceptance_criteria = acceptance_criteria
        if blocked_by is not None:
            issue.blocked_by = blocked_by
        if type is not None:
            issue.type = type
        if risk is not None:
            issue.risk = risk
        if evidence_requirements is not None:
            issue.evidence_requirements = evidence_requirements
        issue.contract_overridden = True
        issue.review_state = "needs-review"
        issue.status = "needs-review"
        self._record(f"{issue_id} contract changed; re-review required.")
        self._persist()

    def launch_issue(
        self,
        issue_id: str,
        *,
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
        workstation_action: dict[str, str] | None = None,
    ) -> LocalAgentSession:
        with self._session_launch_lock():
            self._load_runtime()
            self._ensure_wayfinder_gate_open()
            return self._launch_issue(
                issue_id,
                allowed_paths=allowed_paths,
                command_policy=command_policy,
                workstation_action=workstation_action,
            )

    def _ensure_wayfinder_gate_open(self) -> None:
        ensure_wayfinder_gate_open(
            runtime_root=self.runtime_root,
            target_repo=self.target_repo,
        )

    def _launch_issue(
        self,
        issue_id: str,
        *,
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
        workstation_action: dict[str, str] | None = None,
    ) -> LocalAgentSession:
        issue = self._issue(issue_id)
        if issue.review_state != "approved":
            raise LaunchBlockedError(f"{issue_id} must be approved before launch.")
        unsatisfied = [blocker for blocker in issue.blocked_by if not self._lifecycle_satisfies_blocker(self._issue(blocker))]
        if unsatisfied:
            raise LaunchBlockedError(f"{issue_id} is blocked by {', '.join(unsatisfied)}.")
        self._validate_target_repository_boundary()
        configured_agent = self.agent_registry.find(issue.assigned_agent)
        if configured_agent is not None and not self._assignment_authorized(issue):
            raise LaunchBlockedError(
                f"{issue_id} assigned agent {issue.assigned_agent!r} is not launchable; "
                "choose an assignable worker or route through an approved delegate."
            )
        if not self._assignment_available(issue):
            assignment = self._model_assignment(issue, [])
            reason = assignment["availability_reason"] or assignment["availability"]
            raise LaunchBlockedError(f"{issue_id} assigned model is unavailable: {reason}.")
        if command_policy:
            self.command_policy.update(command_policy)
        agent_config = self.agent_registry.find(issue.assigned_agent)
        self._ensure_delegation_approved(issue, agent_config)
        if agent_config and agent_config.runner in {"command", "ollama"}:
            runner_command = self._runner_command(agent_config)
            policy = self.classify_command(runner_command)
            if policy != "auto-allowed":
                raise LaunchBlockedError(f"{issue_id} command runner policy is {policy}; auto-allowed is required.")
        session_id = f"session-{issue_id}-{len(self.sessions) + 1}"
        worktree_path = self._session_worktree_path(session_id)
        task_packet = {
            "issue_id": issue.id,
            "goal": issue.what_to_build,
            "acceptance_criteria": issue.acceptance_criteria,
            "allowed_paths": allowed_paths or [],
            "command_policy": command_policy or {},
            "evidence_requirements": issue.evidence_requirements or self.default_evidence_requirements(),
            "assigned_agent": issue.assigned_agent,
            "agent_config": self._agent_config_for(issue.assigned_agent),
            "notes": issue.notes,
        }
        if workstation_action:
            task_packet["workstation_action"] = dict(workstation_action)
        if issue.id in self.delegations:
            task_packet["delegation"] = self.delegations[issue.id].to_dict()
        session = LocalAgentSession(
            session_id=session_id,
            issue_id=issue.id,
            assigned_agent=issue.assigned_agent,
            worktree_path=worktree_path,
            task_packet=task_packet,
            status="queued",
        )
        self.sessions[session_id] = session
        self._record(f"{issue_id} queued as {session_id}.")
        self._persist()
        return session

    def route_issue(self, issue_id: str) -> DelegationDecision:
        self._ensure_wayfinder_gate_open()
        issue = self._issue(issue_id)
        if issue.review_state != "approved":
            raise LaunchBlockedError(f"{issue_id} must be approved before routing.")
        router = self._router_agent()
        command = self._runner_command(router)
        if command and self.classify_command(command) != "auto-allowed":
            raise LaunchBlockedError(f"{issue_id} router command policy is {self.classify_command(command)}; auto-allowed is required.")
        prompt = self._delegation_prompt(issue, router)
        command_argv = _command_invocation(command)
        governed_argv, sandboxed = sandboxed_process_argv(
            command_argv,
            working_directory=self.target_repo,
            readable_roots=(self.target_repo,),
        )
        if os.name == "posix" and isinstance(command_argv, list) and not sandboxed:
            raise AlbertError(
                "Router command sandbox unavailable: bubblewrap (bwrap) is required."
            )
        try:
            completed = _run_bounded_process(
                governed_argv,
                input_text=prompt,
                cwd=self.target_repo,
                env=sanitized_process_environment(),
                timeout_seconds=_MODEL_COMMAND_TIMEOUT_SECONDS,
            )
        except OSError as exc:
            raise AlbertError(f"Router command failed: {exc}") from exc
        exit_status = completed.returncode
        output = completed.stdout
        stderr = completed.stderr
        if exit_status == 124:
            raise AlbertError(stderr)
        if exit_status == _PROCESS_OUTPUT_LIMIT_EXIT_STATUS:
            raise AlbertError(stderr)
        if exit_status != 0:
            raise AlbertError(f"Router command exited {exit_status}: {stderr.strip()}")
        data = _parse_delegation_decision(output)
        recommended_agent = data["recommended_agent"]
        agent = self.agent_registry.find(recommended_agent)
        if not agent:
            raise AlbertError(f"Router recommended unknown configured agent: {recommended_agent}")
        if agent not in self._delegation_candidates():
            raise AlbertError(
                f"Router recommended agent {recommended_agent!r}, which is not an "
                "eligible delegation candidate."
            )
        requires_approval = bool(
            data["requires_approval"]
            or agent.requires_approval
            or is_cloud_model(agent.model)
        )
        decision = DelegationDecision(
            issue_id=issue.id,
            router_agent=router.id,
            recommended_agent=recommended_agent,
            complexity=data["complexity"],
            reason=data["reason"],
            requires_approval=requires_approval,
        )
        self.delegations[issue.id] = decision
        issue.assigned_agent = recommended_agent
        self._record(f"{issue.id} routed by {router.id} to {recommended_agent}: {decision.reason}")
        self._persist()
        return decision

    def approve_delegation(self, issue_id: str) -> DelegationDecision:
        self._ensure_wayfinder_gate_open()
        issue = self._issue(issue_id)
        decision = self.delegations.get(issue.id)
        if not decision:
            raise AlbertError(f"{issue_id} has no delegation decision to approve.")
        agent = self.agent_registry.require(decision.recommended_agent)
        if not decision.requires_approval:
            decision.approved = True
            self._persist()
            return decision
        decision.approved = True
        command = self._runner_command(agent)
        if command:
            self.command_policy[command] = "auto-allowed"
        self._record(f"{issue.id} delegation approved for {decision.recommended_agent}.")
        self._persist()
        return decision

    def launch_repair(
        self,
        session_id: str,
        *,
        agent_id: str = "",
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
        workstation_action: dict[str, str] | None = None,
        manual_retry_reason: str = "",
    ) -> LocalAgentSession:
        with self._session_launch_lock():
            self._load_runtime()
            self._ensure_wayfinder_gate_open()
            return self._launch_repair(
                session_id,
                agent_id=agent_id,
                allowed_paths=allowed_paths,
                command_policy=command_policy,
                workstation_action=workstation_action,
                manual_retry_reason=manual_retry_reason,
            )

    def _launch_repair(
        self,
        session_id: str,
        *,
        agent_id: str = "",
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
        workstation_action: dict[str, str] | None = None,
        manual_retry_reason: str = "",
    ) -> LocalAgentSession:
        prior_session = self._session(session_id)
        issue = self.issues.get(prior_session.issue_id)
        is_ad_hoc = (
            issue is None
            and prior_session.task_packet.get("work_kind") == "ad-hoc-delegation"
        )
        if issue is None and not is_ad_hoc:
            raise AlbertError(f"Unknown Issue Slice: {prior_session.issue_id}")
        review = self._latest_review_for_session(session_id)
        repairable_review = bool(
            review
            and review.next_action
            in {"same-local-agent-repair", "fresh-local-agent-repair"}
        )
        manual_retry = (
            prior_session.status == "failed" and bool(manual_retry_reason.strip())
        )
        if not repairable_review and not manual_retry:
            raise LaunchBlockedError(f"{session_id} does not have a repairable Frontier review.")
        existing_repair = next(
            (
                candidate
                for candidate in self.sessions.values()
                if isinstance(candidate.task_packet.get("repair_context"), dict)
                and candidate.task_packet["repair_context"].get("prior_session_id")
                == session_id
            ),
            None,
        )
        if existing_repair is not None:
            raise LaunchBlockedError(
                f"Repair was already launched for {session_id} as "
                f"{existing_repair.session_id}."
            )
        unsatisfied = (
            [
                blocker
                for blocker in issue.blocked_by
                if not self._lifecycle_satisfies_blocker(self._issue(blocker))
            ]
            if issue is not None
            else []
        )
        if unsatisfied:
            raise LaunchBlockedError(
                f"{prior_session.issue_id} is blocked by {', '.join(unsatisfied)}."
            )
        self._validate_target_repository_boundary()
        if command_policy:
            self.command_policy.update(command_policy)
        repair_allowed_paths = (
            list(prior_session.task_packet.get("allowed_paths", []))
            if allowed_paths is None
            else list(allowed_paths)
        )
        repair_command_policy = (
            dict(prior_session.task_packet.get("command_policy", {}))
            if command_policy is None
            else dict(command_policy)
        )
        assigned_agent = agent_id or prior_session.assigned_agent
        if self.agent_registry.configured and not self.agent_registry.find(assigned_agent):
            raise AlbertError(f"Unknown configured agent: {assigned_agent}")
        agent_config = self.agent_registry.find(assigned_agent)
        if agent_config is not None:
            manual_worker = (
                agent_config in self.assignment_agents()
                and not agent_config.requires_approval
                and not is_cloud_model(agent_config.model)
            )
            delegation = self.delegations.get(prior_session.issue_id)
            same_delegated_worker = bool(
                issue is not None
                and assigned_agent == prior_session.assigned_agent
                and agent_config.delegate_only
                and agent_config in self._delegation_candidates()
                and delegation is not None
                and delegation.recommended_agent == assigned_agent
            )
            if not manual_worker and not same_delegated_worker:
                raise LaunchBlockedError(
                    f"{prior_session.issue_id} repair agent {assigned_agent!r} is not "
                    "launchable; choose an assignable worker or preserve the session's "
                    "authorized delegated worker."
                )
            if same_delegated_worker and issue is not None:
                self._ensure_delegation_approved(issue, agent_config)
        if agent_config and agent_config.availability != "available":
            reason = agent_config.availability_reason or agent_config.availability
            raise LaunchBlockedError(
                f"{prior_session.issue_id} assigned model is unavailable: {reason}."
            )
        if agent_config and agent_config.runner in {"command", "ollama"}:
            runner_command = self._runner_command(agent_config)
            policy = self.classify_command(runner_command)
            if policy != "auto-allowed":
                raise LaunchBlockedError(
                    f"{prior_session.issue_id} command runner policy is {policy}; "
                    "auto-allowed is required."
                )
        if issue is not None:
            issue.assigned_agent = assigned_agent
        repair_context = {
            "prior_session_id": prior_session.session_id,
            "review_outcome": review.outcome if review is not None else "",
            "review_reason": (
                review.reason if review is not None else manual_retry_reason.strip()
            ),
            "next_action": (
                review.next_action
                if review is not None
                else "mission-commander-manual-retry"
            ),
            "prior_evidence": prior_session.evidence.to_dict() if prior_session.evidence else None,
            "prior_artifacts": prior_session.artifacts,
        }
        repair_session_id = (
            f"session-{prior_session.issue_id}-{len(self.sessions) + 1}"
        )
        worktree_path = self._session_worktree_path(repair_session_id)
        goal = (
            issue.what_to_build
            if issue is not None
            else str(prior_session.task_packet.get("goal", prior_session.issue_id))
        )
        acceptance_criteria = (
            issue.acceptance_criteria
            if issue is not None
            else list(prior_session.task_packet.get("acceptance_criteria", []))
        )
        evidence_requirements = (
            issue.evidence_requirements or self.default_evidence_requirements()
            if issue is not None
            else list(
                prior_session.task_packet.get(
                    "evidence_requirements",
                    self.default_evidence_requirements(),
                )
            )
        )
        task_packet = {
            "issue_id": prior_session.issue_id,
            "goal": goal,
            "acceptance_criteria": acceptance_criteria,
            "allowed_paths": repair_allowed_paths,
            "command_policy": repair_command_policy,
            "evidence_requirements": evidence_requirements,
            "assigned_agent": assigned_agent,
            "agent_config": self._agent_config_for(assigned_agent),
            "notes": issue.notes if issue is not None else "",
            "repair_context": repair_context,
        }
        if workstation_action:
            task_packet["workstation_action"] = dict(workstation_action)
        for field_name in (
            "work_kind",
            "conversation_scope",
            "originating_message_id",
            "requires_edit",
        ):
            if field_name in prior_session.task_packet:
                task_packet[field_name] = prior_session.task_packet[field_name]
        session = LocalAgentSession(
            session_id=repair_session_id,
            issue_id=prior_session.issue_id,
            assigned_agent=assigned_agent,
            worktree_path=worktree_path,
            task_packet=task_packet,
            status="queued",
        )
        self.sessions[repair_session_id] = session
        self._record(
            f"{prior_session.issue_id} repair queued as {repair_session_id} "
            f"from {session_id}."
        )
        self._persist()
        return session

    def run_session(self, session_id: str) -> LocalAgentSession:
        """Claim and execute one persisted queued Local Agent session."""
        session = self._session(session_id)
        if session.status != "queued":
            raise LaunchBlockedError(
                f"{session_id} cannot run from {session.status}; queued is required."
            )
        try:
            self._validate_target_repository_boundary()
            agent_config = self.agent_registry.find(session.assigned_agent)
            if agent_config is None:
                raise LaunchBlockedError(
                    f"{session_id} assigned agent {session.assigned_agent} has no "
                    "configured runner."
                )
            self._ensure_session_agent_authorized(session, agent_config)
            if agent_config.availability != "available":
                reason = agent_config.availability_reason or agent_config.availability
                raise LaunchBlockedError(
                    f"{session_id} assigned model is unavailable: {reason}."
                )
            if agent_config.runner not in {"fake", "command", "ollama"}:
                raise LaunchBlockedError(
                    f"{session_id} runner is not executable: {agent_config.runner}."
                )
            if agent_config.runner in {"command", "ollama"}:
                runner_command = self._runner_command(agent_config)
                policy = self.classify_command(runner_command)
                if policy != "auto-allowed":
                    raise LaunchBlockedError(
                        f"{session_id} command runner policy is {policy}; "
                        "auto-allowed is required."
                    )
            self._attach_selected_skill(session)
        except Exception as exc:
            session.status = "failed"
            session.runner_exit_status = 1
            session.runner_ended_at = _utc_now()
            session.runner_pid = None
            session.runner_identity = ""
            session.runner_process_pid = None
            session.runner_process_identity = ""
            session.runner_process_token = ""
            session.task_packet["runner_failure"] = str(exc)
            persisted_failure = self._persist_session_update(
                session,
                expected_statuses={"queued"},
                timeline_message=(
                    f"{session.issue_id} runner preflight failed for {session_id}: {exc}"
                ),
            )
            self._record_typed_recovery_failure(persisted_failure)
            raise

        session.status = "running"
        session.runner_started_at = _utc_now()
        session.runner_ended_at = ""
        session.runner_pid = os.getpid()
        session.runner_identity = _process_identity(session.runner_pid)
        raw_attempt = session.task_packet.get("runner_attempt_count", 0)
        attempt = (
            raw_attempt
            if isinstance(raw_attempt, int) and not isinstance(raw_attempt, bool)
            else 0
        ) + 1
        session.task_packet["runner_attempt_count"] = attempt
        operation_payload = (
            f"{self.mission_id}\n{session.session_id}\n{attempt}\n"
            f"{session.runner_pid}\n{session.runner_identity}"
        )
        session.runner_operation_id = (
            "runner-operation:" + sha256(operation_payload.encode("utf-8")).hexdigest()
        )
        session.runner_process_pid = session.runner_pid
        session.runner_process_identity = session.runner_identity
        session.runner_process_token = ""
        session.runner_result = {}
        session.cancel_requested_at = ""
        session.cancel_reason = ""
        session.task_packet.pop("runner_failure", None)
        started_message = f"{session.issue_id} runner started for {session_id}."
        self._persist_session_update(
            session,
            expected_statuses={"queued"},
            timeline_message=started_message,
        )
        try:
            try:
                # Journal creation is part of runner start, not an optional UI
                # enhancement. A failure after persisting `running` must enter
                # this durable terminal-failure path with every other runner
                # error.
                self._start_session_output(session)
                self._raise_if_cancelled(session)
                self._ensure_session_worktree(session)
                session.worktree_identity = self._worktree_identity_for_session(session)
                if not session.worktree_identity:
                    raise AlbertError(
                        f"{session_id} managed Worktree Identity could not be proven."
                    )
                persisted = self._persist_session_update(session)
                if persisted.status == "cancelled":
                    raise SessionCancelledError(
                        f"{session_id} cancelled while preparing its worktree"
                    )
                self._raise_if_cancelled(session)
                if agent_config.runner == "fake":
                    self._run_fake_agent(session)
                elif agent_config.runner == "command":
                    self._run_command_agent(session, agent_config)
                else:
                    self._run_ollama_agent(session, agent_config)
                self._raise_if_cancelled(session)
                self._persist_runner_result_candidate(session)
            except SessionCancelledError:
                return self._refresh_persisted_session(session_id)
            except Exception as exc:
                try:
                    self._raise_if_cancelled(session)
                except SessionCancelledError:
                    return self._refresh_persisted_session(session_id)
                session.status = "failed"
                session.runner_exit_status = (
                    session.runner_exit_status
                    if session.runner_exit_status is not None
                    else 1
                )
                session.runner_ended_at = session.runner_ended_at or _utc_now()
                session.runner_pid = None
                session.runner_identity = ""
                session.runner_process_pid = None
                session.runner_process_identity = ""
                session.runner_process_token = ""
                session.task_packet["runner_failure"] = str(exc)
                failure_message = f"{session.issue_id} runner failed for {session_id}: {exc}"
                persisted = self._persist_session_update(
                    session,
                    timeline_message=failure_message,
                )
                if persisted.status == "cancelled":
                    return persisted
                self._record_typed_recovery_failure(persisted)
                raise AlbertError(f"{session_id} runner failed: {exc}") from exc

            session.runner_ended_at = session.runner_ended_at or _utc_now()
            if session.status == "running":
                session.status = "completed"
            session.runner_pid = None
            session.runner_identity = ""
            session.runner_process_pid = None
            session.runner_process_identity = ""
            session.runner_process_token = ""
            finished_message = (
                f"{session.issue_id} runner finished for {session_id} "
                f"with status {session.status}."
            )
            persisted = self._persist_session_update(
                session,
                timeline_message=finished_message,
            )
            persisted = self._record_typed_recovery_failure(persisted)
            if (
                persisted.evidence is not None
                and persisted.evidence_valid
                and self._evidence_activity_recorder is not None
            ):
                self._evidence_activity_recorder(
                    self.mission_id,
                    persisted,
                    persisted.evidence,
                )
            return persisted
        finally:
            self._finish_session_output(session_id)

    @staticmethod
    def _runner_result_digest(candidate: dict[str, Any]) -> str:
        payload = json.dumps(
            candidate,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        return "sha256:" + sha256(payload).hexdigest()

    def _record_typed_recovery_failure(
        self,
        failed_session: LocalAgentSession,
    ) -> LocalAgentSession:
        if (
            failed_session.status != "failed"
            or failed_session.automatic_recovery_count < 1
        ):
            return failed_session
        incident_payload = (
            f"{self.mission_id}\n{failed_session.session_id}\n"
            f"{failed_session.runner_operation_id}\n{failed_session.revision}\n"
            "automatic-recovery-failed"
        )
        incident_id = sha256(incident_payload.encode("utf-8")).hexdigest()
        receipt_id = f"supervision-receipt:{incident_id[:24]}"
        attention_id = f"runner-attention:{incident_id[:24]}"
        correlation_id = f"supervise:{incident_id}"
        with self._runtime_lock(exclusive=True):
            data = self._read_runtime_payload()
            supervision = self._supervision_from_payload(data)
            existing = supervision.setdefault("receipts", {}).get(receipt_id)
            raw_latest = data.get("sessions", {}).get(failed_session.session_id)
            if not isinstance(raw_latest, dict):
                raise AlbertError(
                    f"Unknown Local Agent session: {failed_session.session_id}"
                )
            latest = LocalAgentSession.from_dict(raw_latest)
            if isinstance(existing, dict):
                self.sessions[latest.session_id] = latest
                self.supervision = supervision
                return latest
            if (
                latest.status != "failed"
                or latest.automatic_recovery_count < 1
                or latest.revision != failed_session.revision
                or latest.runner_operation_id != failed_session.runner_operation_id
            ):
                return latest
            detail = (
                "The one automatic recovery returned a failed runner outcome; "
                "further automation is disabled until the Mission Commander decides."
            )
            supervision.setdefault("attentions", {})[attention_id] = {
                "attention_id": attention_id,
                "incident_id": incident_id,
                "mission_id": self.mission_id,
                "session_id": latest.session_id,
                "kind": "automatic-recovery-failed",
                "detail": detail,
                "next_effect": "mission-commander-decision",
                "disposition": "open",
                "receipt_id": receipt_id,
            }
            supervision["receipts"][receipt_id] = {
                "receipt_id": receipt_id,
                "correlation_id": correlation_id,
                "outcome": "decision-needed",
                "effect": "mission-commander-decision",
                "mission_id": self.mission_id,
                "session_id": latest.session_id,
                "attention_id": attention_id,
            }
            latest.revision += 1
            latest.supervision_receipt_id = receipt_id
            data["sessions"][latest.session_id] = latest.to_dict()
            data["supervision"] = supervision
            data.setdefault("timeline", []).append(
                f"{latest.issue_id} automatic recovery returned failure for "
                f"{latest.session_id}; Mission Commander decision required under "
                f"receipt {receipt_id}."
            )
            self._write_runtime_payload(data)
            self.sessions[latest.session_id] = latest
            self.timeline = list(data.get("timeline", []))
            self.supervision = supervision
            return latest

    def _persist_runner_result_candidate(
        self,
        completed_session: LocalAgentSession,
    ) -> LocalAgentSession:
        """Persist one typed runner result before canonical terminal reconciliation."""

        if completed_session.status not in {"completed", "evidence-ready", "failed"}:
            raise AlbertError("Runner result candidate must be terminal.")
        candidate = {
            "mission_id": self.mission_id,
            "session_id": completed_session.session_id,
            "runner_operation_id": completed_session.runner_operation_id,
            "worktree_identity": completed_session.worktree_identity,
            "status": completed_session.status,
            "runner_exit_status": completed_session.runner_exit_status,
            "runner_ended_at": completed_session.runner_ended_at or _utc_now(),
            "evidence": (
                completed_session.evidence.to_dict()
                if completed_session.evidence is not None
                else None
            ),
            "evidence_valid": completed_session.evidence_valid,
            "evidence_correlation_id": completed_session.evidence_correlation_id,
            "artifacts": dict(completed_session.artifacts),
        }
        candidate["digest"] = self._runner_result_digest(candidate)
        with self._runtime_lock(exclusive=True):
            data = self._read_runtime_payload()
            raw_latest = data.get("sessions", {}).get(completed_session.session_id)
            if not isinstance(raw_latest, dict):
                raise AlbertError(
                    f"Unknown Local Agent session: {completed_session.session_id}"
                )
            latest = LocalAgentSession.from_dict(raw_latest)
            if (
                latest.status != "running"
                or latest.revision != completed_session.revision
                or latest.runner_operation_id != completed_session.runner_operation_id
                or latest.worktree_identity != completed_session.worktree_identity
            ):
                raise LaunchBlockedError(
                    "Runner result no longer matches the canonical session boundary."
                )
            latest.runner_result = candidate
            latest.revision += 1
            data["sessions"][latest.session_id] = latest.to_dict()
            self._write_runtime_payload(data)
            self.sessions[latest.session_id] = latest
            completed_session.runner_result = candidate
            completed_session.revision = latest.revision
            return latest

    def observe_runner(self, observation: RunnerObservation) -> SupervisionReceipt:
        """Reconcile one advisory runner event without granting it Mission authority."""

        string_fields = {
            "source identity": observation.source_id,
            "source incarnation": observation.source_incarnation,
            "Mission identity": observation.mission_id,
            "session identity": observation.session_id,
        }
        for field_name, value in string_fields.items():
            if (
                not isinstance(value, str)
                or not value.strip()
                or len(value.encode("utf-8")) > 4_096
            ):
                raise AlbertError(
                    f"Runner observation {field_name} must be a bounded non-empty string."
                )
        for value, boundary_name in (
            (observation.runner_operation_id, "runner operation identity"),
            (observation.worktree_identity, "Worktree Identity"),
        ):
            if not isinstance(value, str) or len(value.encode("utf-8")) > 4_096:
                raise AlbertError(
                    f"Runner observation {boundary_name} is invalid."
                )
        if (
            not isinstance(observation.sequence, int)
            or isinstance(observation.sequence, bool)
            or observation.sequence <= 0
        ):
            raise AlbertError("Runner observation sequence must be positive.")
        if (
            not isinstance(observation.session_revision, int)
            or isinstance(observation.session_revision, bool)
            or observation.session_revision < 0
        ):
            raise AlbertError("Runner observation session revision is invalid.")
        if observation.owner_signal not in {"live-exact", "absent", "reused", "unavailable"}:
            raise AlbertError("Runner observation owner signal is invalid.")
        if observation.process_group_signal not in {
            "live-exact",
            "absent",
            "reused",
            "unavailable",
        }:
            raise AlbertError("Runner observation process-group signal is invalid.")
        if observation.result_signal not in {
            "absent",
            "exact-valid",
            "invalid",
            "unavailable",
        }:
            raise AlbertError("Runner observation result signal is invalid.")
        if (
            not isinstance(observation.result_digest, str)
            or len(observation.result_digest.encode("utf-8")) > 4_096
            or (observation.result_signal == "exact-valid" and not observation.result_digest)
            or (observation.result_signal != "exact-valid" and observation.result_digest)
        ):
            raise AlbertError("Runner observation result boundary is invalid.")
        if observation.mission_id != self.mission_id:
            raise AlbertError("Runner observation Mission identity does not match.")

        with self._runtime_lock(exclusive=True):
            data = self._read_runtime_payload()
            raw_session = data.get("sessions", {}).get(observation.session_id)
            if not isinstance(raw_session, dict):
                raise AlbertError(
                    f"Unknown Local Agent session: {observation.session_id}"
                )
            session = LocalAgentSession.from_dict(raw_session)
            if (
                observation.owner_signal == "live-exact"
                and observation.process_group_signal == "live-exact"
                and observation.result_signal == "absent"
            ):
                finding_kind = "healthy"
            elif (
                observation.owner_signal == "absent"
                and observation.process_group_signal == "absent"
                and observation.result_signal == "absent"
            ):
                finding_kind = "dead-owner"
            elif (
                observation.owner_signal == "absent"
                and observation.process_group_signal == "absent"
                and observation.result_signal == "exact-valid"
                and bool(observation.result_digest)
            ):
                finding_kind = "result-unreconciled"
            else:
                finding_kind = "ambiguous-runner-state"
            incident_payload = json.dumps(
                {
                    "mission_id": observation.mission_id,
                    "session_id": observation.session_id,
                    "session_revision": observation.session_revision,
                    "runner_operation_id": observation.runner_operation_id,
                    "worktree_identity": observation.worktree_identity,
                    "finding_kind": finding_kind,
                    "owner_signal": observation.owner_signal,
                    "process_group_signal": observation.process_group_signal,
                    "result_signal": observation.result_signal,
                    "result_digest": observation.result_digest,
                },
                sort_keys=True,
                separators=(",", ":"),
            )
            incident_id = sha256(incident_payload.encode("utf-8")).hexdigest()
            correlation_id = f"supervise:{incident_id}"
            receipt_id = f"supervision-receipt:{incident_id[:24]}"
            supervision = self._supervision_from_payload(data)
            observers = supervision.setdefault("observers", {})
            receipts = supervision.setdefault("receipts", {})
            observer = observers.get(observation.source_id)
            if observer is None:
                observer = {
                    "incarnation": observation.source_incarnation,
                    "cursor": 0,
                    "receipts": {},
                }
            if not isinstance(observer, dict):
                raise AlbertError("Runner observer state is invalid.")
            if observer.get("incarnation") != observation.source_incarnation:
                if observation.sequence != 1:
                    raise AlbertError(
                        "A new runner observer incarnation must begin at sequence 1."
                    )
                observer = {
                    "incarnation": observation.source_incarnation,
                    "cursor": 0,
                    "receipts": {},
                }
            cursor = observer.get("cursor", 0)
            observer_receipts = observer.setdefault("receipts", {})
            if not isinstance(cursor, int) or isinstance(cursor, bool) or cursor < 0:
                raise AlbertError("Runner observer cursor is invalid.")
            if observation.sequence <= cursor:
                replay_id = observer_receipts.get(str(observation.sequence))
                replay = receipts.get(replay_id) if isinstance(replay_id, str) else None
                if not isinstance(replay, dict) or replay.get("correlation_id") != correlation_id:
                    raise AlbertError(
                        "Runner observation sequence was already used for a different boundary."
                    )
                self.supervision = supervision
                return SupervisionReceipt.from_dict(replay)
            if observation.sequence != cursor + 1:
                raise AlbertError(
                    f"Runner observer cursor gap: expected {cursor + 1}, "
                    f"received {observation.sequence}."
                )

            exact_canonical_boundary = (
                session.status == "running"
                and session.revision == observation.session_revision
                and session.runner_operation_id == observation.runner_operation_id
                and session.worktree_identity == observation.worktree_identity
            )
            healthy = (
                exact_canonical_boundary
                and finding_kind == "healthy"
            )
            if healthy:
                effect = "none"
            elif (
                exact_canonical_boundary
                and finding_kind == "dead-owner"
            ):
                effect = "recover-same-session"
            elif (
                exact_canonical_boundary
                and finding_kind == "result-unreconciled"
            ):
                effect = "reconcile-result"
            else:
                effect = "mission-commander-decision"
            existing_receipt = receipts.get(receipt_id)
            if isinstance(existing_receipt, dict):
                observer["cursor"] = observation.sequence
                observer_receipts[str(observation.sequence)] = receipt_id
                observers[observation.source_id] = observer
                data["supervision"] = supervision
                self._write_runtime_payload(data)
                self.supervision = supervision
                return SupervisionReceipt.from_dict(existing_receipt)

            receipt = SupervisionReceipt(
                receipt_id=receipt_id,
                correlation_id=correlation_id,
                outcome=(
                    "no-change"
                    if healthy
                    else "decision-needed"
                    if effect == "mission-commander-decision"
                    else "attention-recorded"
                ),
                effect=effect,
                mission_id=self.mission_id,
                session_id=session.session_id,
                attention_id=(
                    "" if healthy else f"runner-attention:{incident_id[:24]}"
                ),
            )
            receipt_payload = {
                "receipt_id": receipt.receipt_id,
                "correlation_id": receipt.correlation_id,
                "outcome": receipt.outcome,
                "effect": receipt.effect,
                "mission_id": receipt.mission_id,
                "session_id": receipt.session_id,
                "attention_id": receipt.attention_id,
            }
            receipts[receipt_id] = receipt_payload
            if not healthy:
                attention = {
                    "attention_id": receipt.attention_id,
                    "incident_id": incident_id,
                    "mission_id": self.mission_id,
                    "session_id": session.session_id,
                    "kind": finding_kind,
                    "detail": (
                        "Exact runner and process group are absent with no result; "
                        "one same-session recovery is pending exact proof."
                        if effect == "recover-same-session"
                        else "An exact durable runner result is pending canonical reconciliation."
                        if effect == "reconcile-result"
                        else "Runner evidence is ambiguous; automatic recovery is blocked."
                    ),
                    "next_effect": effect,
                    "disposition": "open",
                    "receipt_id": receipt_id,
                }
                supervision.setdefault("attentions", {})[receipt.attention_id] = attention
                if effect in {"recover-same-session", "reconcile-result"}:
                    intent_id = f"runner-intent:{incident_id[:24]}"
                    supervision.setdefault("intents", {})[intent_id] = {
                        "intent_id": intent_id,
                        "attention_id": receipt.attention_id,
                        "receipt_id": receipt_id,
                        "effect": effect,
                        "status": "pending",
                        "mission_id": self.mission_id,
                        "session_id": session.session_id,
                        "session_revision": session.revision,
                        "runner_operation_id": session.runner_operation_id,
                        "runner_pid": session.runner_pid,
                        "runner_identity": session.runner_identity,
                        "runner_process_pid": session.runner_process_pid,
                        "runner_process_identity": session.runner_process_identity,
                        "worktree_identity": session.worktree_identity,
                        "result_signal": observation.result_signal,
                        "result_digest": observation.result_digest,
                    }
                else:
                    session.revision += 1
                    session.supervision_receipt_id = receipt_id
                    data["sessions"][session.session_id] = session.to_dict()
            observer["cursor"] = observation.sequence
            observer_receipts[str(observation.sequence)] = receipt_id
            observers[observation.source_id] = observer
            data["supervision"] = supervision
            self._write_runtime_payload(data)
            if effect == "mission-commander-decision":
                self.sessions[session.session_id] = session
            self.supervision = supervision
        if receipt.effect in {"recover-same-session", "reconcile-result"}:
            return self._apply_supervision_intent(receipt.receipt_id)
        return receipt

    @staticmethod
    def _probe_process_identity(pid: int | None, expected_identity: str) -> str:
        if pid is None or not expected_identity:
            return "unavailable"
        if not Path("/proc").is_dir():
            return "unavailable"
        stat_path = Path(f"/proc/{pid}/stat")
        if not stat_path.exists():
            return "absent"
        actual_identity = _process_identity(pid)
        if not actual_identity:
            return "unavailable"
        return "live-exact" if actual_identity == expected_identity else "reused"

    def _probe_runner_boundary(self, session: LocalAgentSession) -> tuple[str, str]:
        owner = self._probe_process_identity(
            session.runner_pid,
            session.runner_identity,
        )
        process_group = self._probe_process_identity(
            session.runner_process_pid,
            session.runner_process_identity,
        )
        if process_group == "absent" and session.runner_process_token:
            token_pids, absence_observable = _probe_process_token_pids(
                session.runner_process_token
            )
            if token_pids:
                process_group = "live-exact"
            elif not absence_observable:
                process_group = "unavailable"
        if (
            process_group == "absent"
            and os.name == "posix"
            and session.runner_process_pid is not None
        ):
            try:
                os.killpg(session.runner_process_pid, 0)
            except ProcessLookupError:
                pass
            except PermissionError:
                process_group = "unavailable"
            else:
                # A live group after its recorded leader identity disappeared is
                # contradictory or reused, never exact quiescence proof.
                process_group = "reused"
        return owner, process_group

    def _apply_supervision_intent(self, receipt_id: str) -> SupervisionReceipt:
        with self._runtime_lock(exclusive=True):
            data = self._read_runtime_payload()
            supervision = self._supervision_from_payload(data)
            receipts = supervision.get("receipts", {})
            receipt_payload = receipts.get(receipt_id)
            if not isinstance(receipt_payload, dict):
                raise AlbertError(f"Unknown supervision receipt: {receipt_id}")
            intents = supervision.get("intents", {})
            intent = next(
                (
                    item
                    for item in intents.values()
                    if isinstance(item, dict) and item.get("receipt_id") == receipt_id
                ),
                None,
            )
            if intent is None or intent.get("status") == "applied":
                self.supervision = supervision
                return SupervisionReceipt.from_dict(receipt_payload)
            raw_session = data.get("sessions", {}).get(intent.get("session_id"))
            if not isinstance(raw_session, dict):
                raise AlbertError("Supervision intent session is unavailable")
            session = LocalAgentSession.from_dict(raw_session)
            owner_signal, group_signal = self._probe_runner_boundary(session)
            current_worktree_identity = self._worktree_identity_for_session(session)
            exact_boundary = (
                intent.get("mission_id") == self.mission_id
                and session.session_id == intent.get("session_id")
                and session.status == "running"
                and session.revision == intent.get("session_revision")
                and session.runner_operation_id == intent.get("runner_operation_id")
                and session.runner_pid == intent.get("runner_pid")
                and session.runner_identity == intent.get("runner_identity")
                and session.runner_process_pid == intent.get("runner_process_pid")
                and session.runner_process_identity
                == intent.get("runner_process_identity")
                and bool(session.worktree_identity)
                and session.worktree_identity == intent.get("worktree_identity")
                and current_worktree_identity == session.worktree_identity
                and owner_signal == "absent"
                and group_signal == "absent"
            )
            identity_boundary = exact_boundary
            effect = intent.get("effect")
            if effect == "recover-same-session":
                repeated_failed_recovery = (
                    identity_boundary
                    and session.automatic_recovery_count >= 1
                    and session.evidence is None
                    and not session.evidence_correlation_id
                    and not session.runner_result
                    and intent.get("result_signal") == "absent"
                )
                if repeated_failed_recovery:
                    session.status = "failed"
                    session.revision += 1
                    session.runner_ended_at = _utc_now()
                    session.runner_exit_status = 1
                    session.supervision_receipt_id = receipt_id
                    session.runner_pid = None
                    session.runner_identity = ""
                    session.runner_process_pid = None
                    session.runner_process_identity = ""
                    session.runner_process_token = ""
                    session.evidence_valid = False
                    session.task_packet["runner_failure"] = (
                        "The one automatic recovery failed; further automation is "
                        "disabled until the Mission Commander decides the next action."
                    )
                    data["sessions"][session.session_id] = session.to_dict()
                    intent["status"] = "blocked"
                    attention = supervision.get("attentions", {}).get(
                        intent.get("attention_id")
                    )
                    if isinstance(attention, dict):
                        attention["kind"] = "automatic-recovery-failed"
                        attention["detail"] = session.task_packet["runner_failure"]
                        attention["next_effect"] = "mission-commander-decision"
                    receipt_payload["outcome"] = "decision-needed"
                    receipt_payload["effect"] = "mission-commander-decision"
                    data.setdefault("timeline", []).append(
                        f"{session.issue_id} automatic recovery failed for "
                        f"{session.session_id}; Mission Commander decision required "
                        f"under receipt {receipt_id}."
                    )
                    data["supervision"] = supervision
                    self._write_runtime_payload(data)
                    self.sessions[session.session_id] = session
                    self.timeline = list(data.get("timeline", []))
                    self.supervision = supervision
                    return SupervisionReceipt.from_dict(receipt_payload)
                exact_boundary = (
                    exact_boundary
                    and session.automatic_recovery_count == 0
                    and session.evidence is None
                    and not session.evidence_correlation_id
                    and not session.runner_result
                    and intent.get("result_signal") == "absent"
                )
            elif effect == "reconcile-result":
                candidate = session.runner_result
                candidate_without_digest = (
                    {key: value for key, value in candidate.items() if key != "digest"}
                    if isinstance(candidate, dict)
                    else {}
                )
                candidate_digest = (
                    candidate.get("digest") if isinstance(candidate, dict) else ""
                )
                exact_boundary = (
                    exact_boundary
                    and bool(candidate_without_digest)
                    and candidate_digest == intent.get("result_digest")
                    and candidate_digest
                    == self._runner_result_digest(candidate_without_digest)
                    and candidate.get("mission_id") == self.mission_id
                    and candidate.get("session_id") == session.session_id
                    and candidate.get("runner_operation_id")
                    == session.runner_operation_id
                    and candidate.get("worktree_identity")
                    == session.worktree_identity
                )
            else:
                exact_boundary = False
            if not exact_boundary:
                return self._fail_closed_supervision_intent(
                    data,
                    supervision,
                    intent,
                    receipt_payload,
                    "Exact runner, process-group, Mission, session, revision, "
                    "Worktree Identity, and result-absence proof did not all hold.",
                )

            if effect == "recover-same-session":
                session.status = "queued"
                session.automatic_recovery_count = 1
                session.runner_started_at = ""
                session.runner_ended_at = ""
                session.runner_exit_status = None
                session.task_packet["runner_failure"] = (
                    "The prior runner and process group stopped without a valid result; "
                    "one automatic same-session recovery was queued."
                )
                outcome = "recovered"
                timeline_message = (
                    f"{session.issue_id} automatic runner recovery queued for "
                    f"{session.session_id}; receipt {receipt_id}."
                )
            else:
                candidate = session.runner_result
                session.status = str(candidate["status"])
                session.runner_exit_status = candidate.get("runner_exit_status")
                session.runner_ended_at = str(candidate.get("runner_ended_at", ""))
                session.evidence = EvidencePackage.from_dict(candidate.get("evidence"))
                session.evidence_valid = bool(candidate.get("evidence_valid", False))
                session.evidence_correlation_id = str(
                    candidate.get("evidence_correlation_id", "")
                )
                session.artifacts = dict(candidate.get("artifacts", {}))
                outcome = "result-reconciled"
                timeline_message = (
                    f"{session.issue_id} late runner result reconciled for "
                    f"{session.session_id}; receipt {receipt_id}."
                )
            session.revision += 1
            session.supervision_receipt_id = receipt_id
            session.runner_pid = None
            session.runner_identity = ""
            session.runner_process_pid = None
            session.runner_process_identity = ""
            session.runner_process_token = ""
            data["sessions"][session.session_id] = session.to_dict()
            intent["status"] = "applied"
            attention = supervision.get("attentions", {}).get(intent.get("attention_id"))
            if isinstance(attention, dict):
                attention["disposition"] = "resolved"
            receipt_payload["outcome"] = outcome
            data.setdefault("timeline", []).append(timeline_message)
            data["supervision"] = supervision
            self._write_runtime_payload(data)
            self.sessions[session.session_id] = session
            self.timeline = list(data.get("timeline", []))
            self.supervision = supervision
            return SupervisionReceipt.from_dict(receipt_payload)

    def _fail_closed_supervision_intent(
        self,
        data: dict[str, Any],
        supervision: dict[str, Any],
        intent: dict[str, Any],
        receipt_payload: dict[str, Any],
        detail: str,
    ) -> SupervisionReceipt:
        intent["status"] = "blocked"
        attention = supervision.get("attentions", {}).get(intent.get("attention_id"))
        if isinstance(attention, dict):
            attention["kind"] = "recovery-blocked"
            attention["detail"] = detail
            attention["next_effect"] = "mission-commander-decision"
        receipt_payload["outcome"] = "decision-needed"
        receipt_payload["effect"] = "mission-commander-decision"
        raw_session = data.get("sessions", {}).get(intent.get("session_id"))
        if not isinstance(raw_session, dict):
            raise AlbertError("Supervision intent session is unavailable")
        session = LocalAgentSession.from_dict(raw_session)
        session.revision += 1
        session.supervision_receipt_id = str(receipt_payload["receipt_id"])
        data["sessions"][session.session_id] = session.to_dict()
        data["supervision"] = supervision
        self._write_runtime_payload(data)
        self.sessions[session.session_id] = session
        self.supervision = supervision
        return SupervisionReceipt.from_dict(receipt_payload)

    def launch_headless_work(
        self,
        *,
        work_kind: str,
        agent_id: str,
        prompt: str = "",
        review_session_id: str = "",
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
    ) -> LocalAgentSession:
        if work_kind not in {"run", "review"}:
            raise AlbertError(f"Unknown headless work kind: {work_kind}")
        self._ensure_wayfinder_gate_open()
        self._validate_target_repository_boundary()
        agent_config = self.agent_registry.require(agent_id)
        self._ensure_headless_agent_authorized(agent_config)
        if agent_config.availability != "available":
            reason = agent_config.availability_reason or agent_config.availability
            raise LaunchBlockedError(f"{agent_id} assigned model is unavailable: {reason}.")
        if command_policy:
            self.command_policy.update(command_policy)
        if agent_config.runner in {"command", "ollama"}:
            runner_command = self._runner_command(agent_config)
            policy = self.classify_command(runner_command)
            if policy != "auto-allowed":
                raise LaunchBlockedError(
                    f"{agent_id} command runner policy is {policy}; auto-allowed is required."
                )
        work_id = f"headless-{work_kind}-{len(self.sessions) + 1:06d}"
        session_id = f"session-{work_id}"
        worktree_path = self._session_worktree_path(session_id)
        goal = prompt.strip()
        review_context: dict[str, Any] | None = None
        if work_kind == "review":
            if review_session_id:
                prior_session = self._session(review_session_id)
                review_context = {
                    "session_id": prior_session.session_id,
                    "issue_id": prior_session.issue_id,
                    "assigned_agent": prior_session.assigned_agent,
                    "status": prior_session.status,
                    "evidence_valid": prior_session.evidence_valid,
                    "evidence": prior_session.evidence.to_dict() if prior_session.evidence else None,
                    "artifacts": prior_session.artifacts,
                    "runner_exit_status": prior_session.runner_exit_status,
                }
            goal = (
                f"Review session {review_session_id}."
                if review_session_id
                else "Review the current Alfredo workspace state."
            )
        task_packet = {
            "issue_id": work_id,
            "work_kind": f"headless-{work_kind}",
            "goal": goal,
            "prompt": prompt,
            "review_session_id": review_session_id,
            "acceptance_criteria": [
                "Return terminal-suitable lifecycle output.",
                "Respect Orchestrator governance and Evidence Package boundaries.",
            ],
            "allowed_paths": allowed_paths or [],
            "command_policy": command_policy or {},
            "evidence_requirements": self.default_evidence_requirements(),
            "assigned_agent": agent_id,
            "agent_config": self._agent_config_for(agent_id),
        }
        if review_context is not None:
            task_packet["review_context"] = review_context
        session = LocalAgentSession(
            session_id=session_id,
            issue_id=work_id,
            assigned_agent=agent_id,
            worktree_path=worktree_path,
            task_packet=task_packet,
        )
        self.sessions[session_id] = session
        self._attach_selected_skill(session)
        self._ensure_session_worktree(session)
        if agent_config.runner == "fake":
            self._run_fake_agent(session)
        elif agent_config.runner == "command":
            self._run_command_agent(session, agent_config)
        elif agent_config.runner == "ollama":
            self._run_ollama_agent(session, agent_config)
        self._record(f"{work_id} launched as {session_id}.")
        self._persist()
        return session

    def _ensure_headless_agent_authorized(self, agent_config: AgentConfig) -> None:
        if agent_config.delegate_only:
            raise LaunchBlockedError(
                f"{agent_config.id} is delegate-only and must be routed and approved "
                "through a governed Issue Slice before execution."
            )
        if (
            agent_config not in self.assignment_agents()
            or agent_config.requires_approval
            or is_cloud_model(agent_config.model)
        ):
            raise LaunchBlockedError(
                f"{agent_config.id} is not an assignable worker for headless execution."
            )

    def _ensure_session_agent_authorized(
        self,
        session: LocalAgentSession,
        agent_config: AgentConfig,
    ) -> None:
        approval_required = (
            agent_config.requires_approval or is_cloud_model(agent_config.model)
        )
        delegation = self.delegations.get(session.issue_id)
        delegated = bool(
            delegation is not None
            and delegation.recommended_agent == agent_config.id
            and (not approval_required or delegation.approved)
        )
        if agent_config in self.assignment_agents() and (
            not approval_required or delegated
        ):
            return
        if (
            agent_config.delegate_only
            and agent_config in self._delegation_candidates()
            and delegated
        ):
            return
        raise LaunchBlockedError(
            f"{session.session_id} assigned agent {agent_config.id!r} is not authorized "
            "for deferred Local Agent execution; use an assignable worker or an "
            "approved governed delegation."
        )

    def classify_command(self, command: str) -> str:
        if command in self.command_policy:
            return self.command_policy[command]
        stripped = command.strip()
        if stripped.startswith("rm ") or " rm -rf" in f" {stripped}" or stripped.startswith("sudo "):
            return "human-required"
        if stripped.startswith("git push") or stripped.startswith("gh pr create"):
            return "frontier-approvable"
        if stripped.startswith("python -m unittest") or stripped.startswith("python3 -m unittest"):
            return "auto-allowed"
        if stripped.startswith("pytest") or stripped.startswith("npm test"):
            return "auto-allowed"
        if stripped.startswith("ollama run "):
            return "auto-allowed"
        return "human-required"

    def record_command_approval(self, command: str, level: str) -> None:
        if level not in {"auto-allowed", "frontier-approvable", "human-required"}:
            raise AlbertError(f"Unknown command policy level: {level}")
        self.command_policy[command] = level
        self._record(f"Command policy learned: {command} -> {level}.")
        self._persist()

    def classify_file_for_frontier(self, path: str) -> str:
        normalized = path.replace("\\", "/")
        if normalized.startswith("./"):
            normalized = normalized[2:]
        name = Path(normalized).name
        if name in {".env", ".env.local"} or normalized.endswith(".pem") or normalized.endswith(".key"):
            return "Blocked"
        if normalized.startswith(".local/") or "/.local/" in normalized:
            return "Local-only"
        return "Normal"

    def list_agents(self) -> list[AgentConfig]:
        return self.agent_registry.agents

    def assignment_agents(self) -> list[AgentConfig]:
        return [
            agent
            for agent in self.agent_registry.agents
            if is_eligible_assignment_agent(agent)
        ]

    def _runner_command(self, agent_config: AgentConfig) -> str:
        if agent_config.command:
            return agent_config.command
        if agent_config.runner == "ollama":
            return f"ollama run {agent_config.model} --think=false --nowordwrap --format json"
        return ""

    def record_evidence(self, session_id: str, evidence: EvidencePackage) -> None:
        session = self._session(session_id)
        missing = evidence.missing_fields()
        if missing:
            raise EvidenceValidationError(f"Evidence Package is missing: {', '.join(missing)}")
        unsafe_links = [
            link
            for link in evidence.artifact_links
            if not self._artifact_link_is_safe_for_review(session, link)
        ]
        if unsafe_links:
            raise EvidenceValidationError(
                "Evidence Package contains an unsafe artifact link: "
                + ", ".join(unsafe_links)
            )
        correlation_id = f"evidence:{self.mission_id}:{session_id}"
        if session.evidence_correlation_id:
            if (
                session.evidence_correlation_id == correlation_id
                and session.evidence == evidence
                and session.evidence_valid
            ):
                if self._evidence_activity_recorder is not None:
                    self._evidence_activity_recorder(
                        self.mission_id,
                        session,
                        evidence,
                    )
                return
            raise AlbertError(
                f"Evidence correlation id was already used for a different package: "
                f"{correlation_id}"
            )
        session.evidence = evidence
        session.evidence_valid = True
        session.evidence_correlation_id = correlation_id
        session.status = "evidence-ready"
        self._record(f"{session.issue_id} evidence package validated for {session_id}.")
        self._persist()
        if self._evidence_activity_recorder is not None:
            self._evidence_activity_recorder(self.mission_id, session, evidence)

    def cancel_session(
        self,
        session_id: str,
        *,
        reason: str,
        workstation_action: dict[str, Any] | None = None,
    ) -> LocalAgentSession:
        if not reason.strip():
            raise AlbertError("Session cancellation requires a reason.")
        with self._runtime_lock(exclusive=True):
            data = self._read_runtime_payload()
            session_data = data.get("sessions", {}).get(session_id)
            if not isinstance(session_data, dict):
                raise AlbertError(f"Unknown Local Agent session: {session_id}")
            session = LocalAgentSession.from_dict(session_data)
            if session.status not in {"queued", "launched", "running"}:
                raise AlbertError(
                    f"{session_id} cannot be cancelled from {session.status}."
                )
            workstation_actions = self._merge_workstation_action_ledgers(
                data.get("workstation_actions", {}),
                {},
            )
            if workstation_action:
                correlation_id = workstation_action.get("correlation_id")
                if not isinstance(correlation_id, str) or not correlation_id.strip():
                    raise AlbertError("Workstation action correlation id must not be empty")
                workstation_actions = self._merge_workstation_action_ledgers(
                    workstation_actions,
                    {correlation_id: dict(workstation_action)},
                )
            session.status = "cancelled"
            session.cancel_requested_at = _utc_now()
            session.cancel_reason = reason.strip()
            session.runner_ended_at = session.runner_ended_at or session.cancel_requested_at
            session.runner_pid = None
            session.runner_identity = ""
            session.runner_process_pid = None
            session.runner_process_identity = ""
            session.runner_process_token = ""
            data["sessions"][session_id] = session.to_dict()
            data["workstation_actions"] = workstation_actions
            timeline = list(data.get("timeline", []))
            timeline.append(
                f"{session.issue_id} session {session_id} cancelled: {reason.strip()}"
            )
            data["timeline"] = timeline
            self._write_runtime_payload(data)
        self.sessions[session_id] = session
        self.workstation_actions = workstation_actions
        self.timeline = timeline
        return session

    def archive_issue(
        self,
        issue_id: str,
        *,
        workstation_action: dict[str, Any] | None = None,
    ) -> IssueSlice:
        """Archive one evidence-accepted Issue Slice without removing its history."""
        return self._set_issue_archived(
            issue_id,
            archived=True,
            workstation_action=workstation_action,
        )

    def restore_archived_issue(
        self,
        issue_id: str,
        *,
        workstation_action: dict[str, Any] | None = None,
    ) -> IssueSlice:
        """Restore one retained Issue Slice subtree to the active Mission Work tree."""
        return self._set_issue_archived(
            issue_id,
            archived=False,
            workstation_action=workstation_action,
        )

    def _set_issue_archived(
        self,
        issue_id: str,
        *,
        archived: bool,
        workstation_action: dict[str, Any] | None,
    ) -> IssueSlice:
        with self._runtime_lock(exclusive=True):
            data = self._read_runtime_payload()
            for candidate_id, runtime in data.get("issues", {}).items():
                if candidate_id in self.issues and isinstance(runtime, dict):
                    self.issues[candidate_id].apply_runtime(runtime)
            issue = self._issue(issue_id)
            current_archived = data.get("archived_issue_ids", [])
            if (
                not isinstance(current_archived, list)
                or not all(isinstance(candidate, str) and candidate.strip() for candidate in current_archived)
                or len(current_archived) != len(set(current_archived))
                or any(candidate not in self.issues for candidate in current_archived)
            ):
                raise AlbertError("Mission archive state is invalid")
            archived_ids = set(current_archived)
            if archived:
                if (
                    issue.review_state not in {"pr-ready", "complete"}
                    and issue.tracker_status.lower() != "merged"
                ):
                    raise AlbertError(
                        f"{issue_id} must have accepted evidence or be tracker-merged before it can be archived."
                    )
                if issue_id in archived_ids:
                    raise AlbertError(f"{issue_id} is already archived.")
            elif issue_id not in archived_ids:
                raise AlbertError(f"{issue_id} is not archived.")

            workstation_actions = self._merge_workstation_action_ledgers(
                data.get("workstation_actions", {}),
                {},
            )
            if workstation_action:
                correlation_id = workstation_action.get("correlation_id")
                if not isinstance(correlation_id, str) or not correlation_id.strip():
                    raise AlbertError("Workstation action correlation id must not be empty")
                workstation_actions = self._merge_workstation_action_ledgers(
                    workstation_actions,
                    {correlation_id: dict(workstation_action)},
                )

            if archived:
                archived_ids.add(issue_id)
                event = f"{issue_id} archived from Mission Work; history retained."
            else:
                archived_ids.remove(issue_id)
                event = f"{issue_id} restored to Mission Work with history retained."
            timeline = list(data.get("timeline", []))
            timeline.append(event)
            data["archived_issue_ids"] = sorted(archived_ids)
            data["workstation_actions"] = workstation_actions
            data["timeline"] = timeline
            self._write_runtime_payload(data)

        self.archived_issue_ids = archived_ids
        self.workstation_actions = workstation_actions
        self.timeline = timeline
        return issue

    def record_frontier_review(
        self,
        session_id: str,
        outcome: str,
        *,
        reason: str,
        failure_type: str = "",
        limitations: list[str] | None = None,
        allowed_session_statuses: set[str] | None = None,
        workspace_action: dict[str, Any] | None = None,
    ) -> ReviewDecision:
        outcome = _normalize_review_outcome(outcome)
        with self._runtime_lock(exclusive=True):
            data = self._read_runtime_payload()
            raw_session = data.get("sessions", {}).get(session_id)
            if not isinstance(raw_session, dict):
                raise AlbertError(f"Unknown Local Agent session: {session_id}")
            session = LocalAgentSession.from_dict(raw_session)
            self.sessions[session_id] = session
            for issue_id, runtime in data.get("issues", {}).items():
                if issue_id in self.issues and isinstance(runtime, dict):
                    self.issues[issue_id].apply_runtime(runtime)
            self.reviews = [
                ReviewDecision.from_dict(item)
                for item in data.get("reviews", [])
                if isinstance(item, dict)
            ]
            self.delegations = {
                issue_id: DelegationDecision.from_dict(item)
                for issue_id, item in data.get("delegations", {}).items()
                if isinstance(item, dict)
            }
            self.command_policy = dict(data.get("command_policy", {}))
            self.timeline = list(data.get("timeline", []))
            if (
                allowed_session_statuses is not None
                and session.status not in allowed_session_statuses
            ):
                raise LaunchBlockedError(
                    f"{session_id} cannot be reviewed from {session.status}; "
                    "evidence-ready or terminal state is required."
                )
            if (
                outcome in {"Approved", "Approved with limitations"}
                and not session.evidence_valid
            ):
                raise EvidenceValidationError(
                    "Approved outcomes require a valid Evidence Package."
                )
            next_action = self._next_action_for_review(
                session_id,
                outcome,
                failure_type,
            )
            decision = ReviewDecision(
                session_id=session_id,
                issue_id=session.issue_id,
                outcome=outcome,
                reason=reason,
                next_action=next_action,
                limitations=limitations or [],
                workspace_action=dict(workspace_action or {}),
            )
            self.reviews.append(decision)
            session.status = "reviewed"
            issue = self.issues.get(session.issue_id)
            if outcome in {"Approved", "Approved with limitations"}:
                session.cleanup_eligible = True
                if issue:
                    issue.review_state = "pr-ready"
            elif issue and outcome == "Needs repair":
                issue.review_state = "needs-repair"
            elif issue and outcome == "Needs human review":
                issue.review_state = "needs-human-review"
            elif issue and outcome == "Rejected":
                issue.review_state = "rejected"
            self._record(
                f"{session.issue_id} frontier review: {outcome}; "
                f"next action {next_action}."
            )
            data["issues"] = {
                issue_id: item.to_runtime()
                for issue_id, item in self.issues.items()
            }
            sessions = dict(data.get("sessions", {}))
            sessions[session_id] = session.to_dict()
            data["sessions"] = sessions
            data["reviews"] = [item.to_dict() for item in self.reviews]
            data["delegations"] = {
                issue_id: item.to_dict()
                for issue_id, item in self.delegations.items()
            }
            data["command_policy"] = self.command_policy
            data["timeline"] = self.timeline
            self._write_runtime_payload(data)
            return decision

    def generate_mission_records(self) -> Path:
        mission_dir = self.target_repo / "docs" / "missions" / self.mission_id
        issue_dir = mission_dir / "issues"
        issue_dir.mkdir(parents=True, exist_ok=True)
        summary = self.board_summary()
        next_action = self._next_action()
        self._write(
            mission_dir / "README.md",
            "\n".join(
                [
                    f"# Mission {self.mission_id}",
                    "",
                    f"Product Requirements Document: {self.prd_title}",
                    f"Status: {len(summary['approved_issue_ids'])}/{summary['issue_count']} Issue Slices approved",
                    f"Next action: {next_action}",
                    "",
                    "## Issue Slices",
                    *[f"- {issue_id}: {self.issues[issue_id].title} ({self.issues[issue_id].review_state})" for issue_id in summary["ordered_issue_ids"]],
                    "",
                ]
            ),
        )
        self._write(mission_dir / "timeline.md", "# Timeline\n\n" + "\n".join(f"- {event}" for event in self.timeline) + "\n")
        self._write(
            mission_dir / "local-agent-tracker.md",
            "# Local Agent Tracker\n\n"
            + "\n".join(
                f"- {session.session_id}: {session.issue_id} assigned to {session.assigned_agent}; status {session.status}"
                for session in self.sessions.values()
            )
            + "\n",
        )
        self._write(
            mission_dir / "evidence-index.md",
            "# Evidence Index\n\n"
            + "\n".join(self._evidence_index_lines())
            + "\n",
        )
        self._write(
            mission_dir / "frontier-review-summary.md",
            "# Frontier Review Summary\n\n"
            + "\n".join(
                f"- {review.issue_id}: {review.outcome}; next action {review.next_action}; reason {review.reason}"
                for review in self.reviews
            )
            + "\n",
        )
        for issue in self.issues.values():
            session_lines = [
                f"- Session {session.session_id}: status {session.status}"
                for session in self.sessions.values()
                if session.issue_id == issue.id
            ]
            self._write(
                issue_dir / f"{issue.id}.md",
                "\n".join(
                    [
                        f"# {issue.id} - {issue.title}",
                        "",
                        f"Tracker issue: {issue.source_path}",
                        f"Execution status: {issue.review_state}",
                        f"Review status: {issue.review_state}",
                        "",
                        "## Local Agent Activity",
                        *(session_lines or ["- No Local Agent session yet."]),
                        "",
                    ]
                ),
            )
        self._persist()
        return mission_dir

    def prepare_pr(self, issue_id: str, *, gh_available: bool) -> PrPreparation:
        issue = self._issue(issue_id)
        reviews = [review for review in self.reviews if review.issue_id == issue_id]
        approved = [review for review in reviews if review.outcome in {"Approved", "Approved with limitations"}]
        if not approved:
            raise AlbertError(f"{issue_id} is not PR-ready.")
        branch_name = f"albert/{self.mission_id}/{issue.id}-{issue.slug}"
        title = f"{issue.id}: {issue.title}"
        evidence_lines = self._evidence_index_lines(issue_id=issue_id)
        body_lines = [
            f"# {title}",
            "",
            "## Issue Slice",
            issue.what_to_build,
            "",
            "## What Changed",
            "See the linked Evidence Package and Local Agent activity for implementation details.",
            "",
            "## Acceptance Criteria",
            *[f"- {criterion}" for criterion in issue.acceptance_criteria],
            "",
            "## Evidence",
            *(evidence_lines or ["- Evidence recorded in app-local runtime state."]),
            "",
            "## Frontier Review",
            *[f"- {review.outcome}: {review.reason}" for review in reviews],
            "",
            "## Local Agent Activity",
            *[
                f"- {session.session_id}: {session.status}"
                for session in self.sessions.values()
                if session.issue_id == issue_id
            ],
            "",
        ]
        if gh_available:
            create_command = f"gh pr create --head {branch_name} --title {json.dumps(title)} --body-file <generated-body-file>"
        else:
            body_lines.extend(
                [
                    "## Manual PR instructions",
                    f"Push branch `{branch_name}` and open a PR with this summary.",
                    "Do not auto-merge; final merge is human-only.",
                    "",
                ]
            )
            create_command = ""
        self._record(f"{issue_id} prepared for PR on {branch_name}.")
        self._persist()
        return PrPreparation(
            issue_id=issue_id,
            branch_name=branch_name,
            title=title,
            body="\n".join(body_lines),
            create_command=create_command,
            merge_approved=False,
        )

    @staticmethod
    def default_evidence_requirements() -> list[str]:
        return [
            "changed files",
            "diff summary",
            "commands run",
            "test results",
            "known risks",
            "proposed context updates",
        ]

    def _load_prd_title(self) -> str:
        prd_path = self.tracker_dir / "PRD.md"
        if not prd_path.exists():
            raise AlbertError(f"Missing Product Requirements Document: {prd_path}")
        for line in prd_path.read_text(encoding="utf-8").splitlines():
            if line.startswith("# "):
                return line.removeprefix("# ").strip()
        return prd_path.stem

    def _load_issues(self) -> dict[str, IssueSlice]:
        issues_dir = self.issues_dir
        if not issues_dir.exists():
            if self.allow_empty_tracker:
                return {}
            raise AlbertError(f"Missing issues directory: {issues_dir}")
        issues: dict[str, IssueSlice] = {}
        for path in sorted(issues_dir.glob("*.md")):
            if path.name.upper() in {"README.MD", "PRD.MD"}:
                continue
            if _record_type(path.read_text(encoding="utf-8")) == "prd":
                continue
            issue = self._parse_issue(path)
            issues[issue.id] = issue
        if not issues and not self.allow_empty_tracker:
            raise AlbertError(f"No Issue Slice records found in {issues_dir}")
        return issues

    def _parse_issue(self, path: Path) -> IssueSlice:
        match = re.match(r"(?P<num>\d+)-(?P<slug>.+)\.md$", path.name)
        if not match:
            raise AlbertError(f"Issue file must start with a number: {path.name}")
        issue_id = f"ISS-{int(match.group('num')):02d}"
        slug = _slug(match.group("slug"))
        text = path.read_text(encoding="utf-8")
        metadata = _metadata(text)
        sections = _sections(text)
        what = sections.get("What to build", "").strip()
        acceptance = _checklist_items(sections.get("Acceptance criteria", ""))
        blockers = _issue_refs(sections.get("Blocked by", ""), issues_dir=path.parent)
        if not what:
            raise AlbertError(f"{path.name} is missing a What to build section.")
        if not acceptance:
            raise AlbertError(f"{path.name} is missing acceptance criteria.")
        title = slug.replace("-", " ").title()
        evidence = _checklist_items(sections.get("Evidence required", ""))
        status = metadata.get("status", "")
        review_state = {
            "complete": "complete",
            "completed": "complete",
            "approved": "approved",
            "pr-ready": "pr-ready",
        }.get(status.lower(), "needs-review")
        runtime_status = review_state
        return IssueSlice(
            id=issue_id,
            slug=slug,
            title=title,
            status=runtime_status,
            tracker_status=status,
            type=metadata.get("type", ""),
            risk=metadata.get("risk", "Medium"),
            suggested_agent=metadata.get("suggested agent", "qwen-coder-local-1"),
            assigned_agent=metadata.get("assigned agent", metadata.get("suggested agent", "qwen-coder-local-1")),
            what_to_build=what,
            acceptance_criteria=acceptance,
            blocked_by=blockers,
            source_path=str(path),
            evidence_requirements=evidence,
            review_state=review_state,
            locked=review_state in {"approved", "pr-ready", "complete"},
        )

    def _load_runtime(self) -> None:
        if not self.runtime_path.exists():
            return
        data = json.loads(self.runtime_path.read_text(encoding="utf-8"))
        for issue_id, runtime in data.get("issues", {}).items():
            if issue_id in self.issues:
                self.issues[issue_id].apply_runtime(runtime)
        self.sessions = {
            session_id: LocalAgentSession.from_dict(session)
            for session_id, session in data.get("sessions", {}).items()
        }
        self.reviews = [ReviewDecision.from_dict(item) for item in data.get("reviews", [])]
        self.delegations = {
            issue_id: DelegationDecision.from_dict(item)
            for issue_id, item in data.get("delegations", {}).items()
        }
        self.command_policy = dict(data.get("command_policy", {}))
        self.workstation_actions = self._merge_workstation_action_ledgers(
            {},
            data.get("workstation_actions", {}),
        )
        archived = data.get("archived_issue_ids", [])
        if (
            not isinstance(archived, list)
            or not all(isinstance(issue_id, str) and issue_id.strip() for issue_id in archived)
            or len(archived) != len(set(archived))
            or any(issue_id not in self.issues for issue_id in archived)
        ):
            raise AlbertError("Mission archive state is invalid")
        self.archived_issue_ids = set(archived)
        self.supervision = self._supervision_from_payload(data)
        self.timeline = list(data.get("timeline", []))

    def _reconcile_abandoned_sessions(self) -> None:
        """Run token-free startup reconciliation through the durable ledger."""

        pending_receipts = [
            str(intent.get("receipt_id"))
            for intent in self.supervision.get("intents", {}).values()
            if isinstance(intent, dict)
            and intent.get("status") == "pending"
            and isinstance(intent.get("receipt_id"), str)
        ]
        for receipt_id in pending_receipts:
            self._apply_supervision_intent(receipt_id)

        observer = self.supervision.get("observers", {}).get(
            "startup-reconciliation",
            {},
        )
        cursor = observer.get("cursor", 0) if isinstance(observer, dict) else 0
        sequence = cursor + 1 if isinstance(cursor, int) and cursor >= 0 else 1
        for session in sorted(self.sessions.values(), key=lambda item: item.session_id):
            if session.status != "running":
                continue
            owner_signal, process_group_signal = self._probe_runner_boundary(session)
            if (
                owner_signal == "live-exact"
                and process_group_signal == "live-exact"
                and not session.runner_result
            ):
                # A healthy reconciliation sweep is deliberately silent and does
                # not manufacture an observer event or durable user-facing record.
                continue
            result_signal = "absent"
            result_digest = ""
            if session.runner_result:
                candidate = session.runner_result
                candidate_without_digest = {
                    key: value for key, value in candidate.items() if key != "digest"
                }
                result_digest = str(candidate.get("digest", ""))
                result_signal = (
                    "exact-valid"
                    if result_digest
                    and result_digest == self._runner_result_digest(candidate_without_digest)
                    else "invalid"
                )
            self.observe_runner(
                RunnerObservation(
                    source_id="startup-reconciliation",
                    source_incarnation="canonical-v1",
                    sequence=sequence,
                    mission_id=self.mission_id,
                    session_id=session.session_id,
                    session_revision=session.revision,
                    runner_operation_id=session.runner_operation_id,
                    owner_signal=owner_signal,
                    process_group_signal=process_group_signal,
                    worktree_identity=session.worktree_identity,
                    result_signal=result_signal,
                    result_digest=result_digest,
                )
            )
            sequence += 1

    def _runtime_payload(self) -> dict[str, Any]:
        return {
            "mission_id": self.mission_id,
            "project_key": self.project_key,
            "prd_title": self.prd_title,
            "issues": {issue_id: issue.to_runtime() for issue_id, issue in self.issues.items()},
            "sessions": {session_id: session.to_dict() for session_id, session in self.sessions.items()},
            "reviews": [review.to_dict() for review in self.reviews],
            "delegations": {issue_id: decision.to_dict() for issue_id, decision in self.delegations.items()},
            "command_policy": self.command_policy,
            "workstation_actions": self.workstation_actions,
            "archived_issue_ids": sorted(self.archived_issue_ids),
            "supervision": self.supervision,
            "timeline": self.timeline,
        }

    def _persist(self, *, _runtime_lock_held: bool = False) -> None:
        self.runtime_dir.mkdir(parents=True, exist_ok=True)
        data = self._runtime_payload()
        lock = (
            nullcontext()
            if _runtime_lock_held
            else self._runtime_lock(exclusive=True)
        )
        with lock:
            if self.runtime_path.exists():
                latest = self._read_runtime_payload()
                data["sessions"] = self._merge_session_payloads(
                    latest.get("sessions", {}),
                    data["sessions"],
                )
                latest_timeline = list(latest.get("timeline", []))
                data["timeline"] = latest_timeline + [
                    item for item in self.timeline if item not in latest_timeline
                ]
                data["command_policy"] = {
                    **dict(latest.get("command_policy", {})),
                    **self.command_policy,
                }
                data["workstation_actions"] = self._merge_workstation_action_ledgers(
                    latest.get("workstation_actions", {}),
                    self.workstation_actions,
                )
                latest_archived = latest.get("archived_issue_ids", [])
                if not isinstance(latest_archived, list) or not all(
                    isinstance(issue_id, str) for issue_id in latest_archived
                ):
                    raise AlbertError("Mission archive state is invalid")
                # Archive transitions are written through _set_issue_archived while
                # holding this same lock.  A generic persist from a stale mission
                # instance must not revive an Issue Slice another instance restored.
                # Preserve the latest canonical archive state here instead.
                data["archived_issue_ids"] = sorted(latest_archived)
                data["supervision"] = self._supervision_from_payload(latest)
            self._write_runtime_payload(data)
        reconciled_sessions: dict[str, LocalAgentSession] = {}
        for session_id, session_data in data["sessions"].items():
            existing = self.sessions.get(session_id)
            if existing is not None and existing.to_dict() == session_data:
                reconciled_sessions[session_id] = existing
            else:
                reconciled_sessions[session_id] = LocalAgentSession.from_dict(
                    session_data
                )
        self.sessions = reconciled_sessions
        self.timeline = list(data["timeline"])
        self.command_policy = dict(data["command_policy"])
        self.workstation_actions = {
            correlation_id: dict(marker)
            for correlation_id, marker in data["workstation_actions"].items()
        }
        self.archived_issue_ids = set(data.get("archived_issue_ids", []))
        self.supervision = self._supervision_from_payload(data)

    @staticmethod
    def _merge_workstation_action_ledgers(
        existing: Any,
        incoming: Any,
    ) -> dict[str, dict[str, Any]]:
        if not isinstance(existing, dict) or not isinstance(incoming, dict):
            raise AlbertError("mission runtime workstation actions must be an object")
        merged: dict[str, dict[str, Any]] = {}
        for ledger in (existing, incoming):
            for correlation_id, raw_marker in ledger.items():
                if not isinstance(correlation_id, str) or not correlation_id.strip():
                    raise AlbertError(
                        "mission runtime workstation action correlation id must not be empty"
                    )
                if not isinstance(raw_marker, dict):
                    raise AlbertError(
                        "mission runtime workstation action marker must be an object"
                    )
                marker = dict(raw_marker)
                if marker.get("correlation_id") != correlation_id:
                    raise AlbertError(
                        "mission runtime workstation action marker correlation id does not match"
                    )
                prior = merged.get(correlation_id)
                if prior is not None and prior != marker:
                    raise AlbertError(
                        "Workstation action correlation id was already used for a different "
                        "request boundary."
                    )
                merged[correlation_id] = marker
        return merged

    def _remember_workstation_action(self, marker: dict[str, Any]) -> None:
        correlation_id = marker.get("correlation_id")
        incoming = {correlation_id: dict(marker)} if isinstance(correlation_id, str) else {}
        if not incoming:
            raise AlbertError("Workstation action correlation id must not be empty")
        self.workstation_actions = self._merge_workstation_action_ledgers(
            self.workstation_actions,
            incoming,
        )

    @classmethod
    def _merge_session_payloads(
        cls,
        latest_sessions: Any,
        proposed_sessions: Any,
    ) -> dict[str, Any]:
        latest = latest_sessions if isinstance(latest_sessions, dict) else {}
        proposed = proposed_sessions if isinstance(proposed_sessions, dict) else {}
        merged: dict[str, Any] = {
            session_id: session_data
            for session_id, session_data in latest.items()
            if isinstance(session_data, dict)
        }
        for session_id, proposed_data in proposed.items():
            if not isinstance(proposed_data, dict):
                continue
            latest_data = merged.get(session_id)
            if latest_data is None:
                merged[session_id] = proposed_data
                continue
            merged[session_id] = cls._later_session_payload(
                latest_data,
                proposed_data,
            )
        return merged

    @staticmethod
    def _later_session_payload(
        latest_data: dict[str, Any],
        proposed_data: dict[str, Any],
    ) -> dict[str, Any]:
        latest_session = LocalAgentSession.from_dict(latest_data)
        proposed_session = LocalAgentSession.from_dict(proposed_data)
        latest_status = latest_session.status
        proposed_status = proposed_session.status
        if latest_status == "cancelled":
            return latest_data
        if proposed_status == "cancelled":
            return proposed_data
        if proposed_session.revision > latest_session.revision:
            return proposed_data
        if proposed_session.revision < latest_session.revision:
            return latest_data
        lifecycle_rank = {
            "queued": 0,
            "launched": 0,
            "running": 1,
            "completed": 2,
            "failed": 2,
            "evidence-ready": 3,
            "reviewed": 4,
            "complete": 5,
        }
        latest_rank = lifecycle_rank.get(latest_status, 0)
        proposed_rank = lifecycle_rank.get(proposed_status, 0)
        if proposed_rank > latest_rank:
            return proposed_data
        return latest_data

    @contextmanager
    def _runtime_lock(self, *, exclusive: bool):
        self.runtime_dir.mkdir(parents=True, exist_ok=True)
        lock_path = self.runtime_dir / ".runtime.lock"
        with lock_path.open("a+", encoding="utf-8") as lock_file:
            operation = fcntl.LOCK_EX if exclusive else fcntl.LOCK_SH
            fcntl.flock(lock_file.fileno(), operation)
            try:
                yield
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)

    @contextmanager
    def _session_launch_lock(self):
        self.runtime_dir.mkdir(parents=True, exist_ok=True)
        lock_path = self.runtime_dir / ".repair-launch.lock"
        with lock_path.open("a+", encoding="utf-8") as lock_file:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)

    def _read_runtime_payload(self) -> dict[str, Any]:
        if not self.runtime_path.exists():
            raise AlbertError("mission runtime does not exist")
        try:
            data = json.loads(self.runtime_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise AlbertError(f"unable to read mission runtime: {exc}") from exc
        if not isinstance(data, dict):
            raise AlbertError("mission runtime must be a JSON object")
        return data

    def _write_runtime_payload(self, data: dict[str, Any]) -> None:
        payload = json.dumps(data, indent=2, sort_keys=True)
        temporary_path: Path | None = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w",
                encoding="utf-8",
                dir=self.runtime_dir,
                prefix=f".{self.runtime_path.name}.",
                suffix=".tmp",
                delete=False,
            ) as temporary:
                temporary.write(payload)
                temporary_path = Path(temporary.name)
            temporary_path.replace(self.runtime_path)
        finally:
            if temporary_path is not None and temporary_path.exists():
                temporary_path.unlink()

    def _persist_session_update(
        self,
        session: LocalAgentSession,
        *,
        expected_statuses: set[str] | None = None,
        timeline_message: str = "",
    ) -> LocalAgentSession:
        with self._runtime_lock(exclusive=True):
            data = self._read_runtime_payload()
            sessions = data.setdefault("sessions", {})
            latest_data = sessions.get(session.session_id)
            if not isinstance(latest_data, dict):
                raise AlbertError(f"Unknown Local Agent session: {session.session_id}")
            latest = LocalAgentSession.from_dict(latest_data)
            if expected_statuses is not None and latest.status not in expected_statuses:
                raise LaunchBlockedError(
                    f"{session.session_id} cannot transition from {latest.status}; "
                    f"expected {', '.join(sorted(expected_statuses))}."
                )
            if latest.status == "cancelled" and session.status != "cancelled":
                self.sessions[session.session_id] = latest
                self.timeline = list(data.get("timeline", []))
                return latest
            session.revision = latest.revision + 1
            sessions[session.session_id] = session.to_dict()
            timeline = list(data.get("timeline", []))
            if timeline_message:
                timeline.append(timeline_message)
            data["timeline"] = timeline
            self._write_runtime_payload(data)
            self.timeline = timeline
            self.sessions[session.session_id] = session
            return session

    def _refresh_persisted_session(self, session_id: str) -> LocalAgentSession:
        with self._runtime_lock(exclusive=False):
            data = self._read_runtime_payload()
            session_data = data.get("sessions", {}).get(session_id)
            if not isinstance(session_data, dict):
                raise AlbertError(f"Unknown Local Agent session: {session_id}")
            session = LocalAgentSession.from_dict(session_data)
        self.sessions[session_id] = session
        return session

    def _raise_if_cancelled(self, session: LocalAgentSession) -> None:
        if session.runner_pid is None:
            return
        latest = self._refresh_persisted_session(session.session_id)
        if latest.status != "cancelled":
            return
        session.status = latest.status
        session.runner_ended_at = latest.runner_ended_at
        session.runner_pid = None
        session.runner_identity = ""
        session.runner_process_pid = None
        session.runner_process_identity = ""
        session.runner_process_token = ""
        session.cancel_requested_at = latest.cancel_requested_at
        session.cancel_reason = latest.cancel_reason
        raise SessionCancelledError(
            f"{session.session_id} cancelled: {latest.cancel_reason or 'cancel requested'}"
        )

    def _issue(self, issue_id: str) -> IssueSlice:
        if issue_id not in self.issues:
            raise AlbertError(f"Unknown Issue Slice: {issue_id}")
        return self.issues[issue_id]

    def _session(self, session_id: str) -> LocalAgentSession:
        if session_id not in self.sessions:
            raise AlbertError(f"Unknown Local Agent session: {session_id}")
        return self.sessions[session_id]

    def _agent_config_for(self, agent_id: str) -> dict[str, str] | None:
        agent = self.agent_registry.find(agent_id)
        return agent.to_dict() if agent else None

    def _router_agent(self) -> AgentConfig:
        for agent in self.agent_registry.agents:
            if (
                agent.routing.casefold() == "router"
                and is_eligible_controller_agent(agent)
            ):
                return agent
        for agent in self.agent_registry.agents:
            if (
                agent.routing.casefold() == "frontier"
                and is_eligible_controller_agent(agent)
            ):
                return agent
        for agent in self.agent_registry.agents:
            if (
                not agent.routing.strip()
                and agent.role.casefold() == "frontier"
                and is_eligible_controller_agent(agent)
            ):
                return agent
        raise AlbertError(
            "No available, ungated local Frontier router agent is configured."
        )

    def _has_router_agent(self) -> bool:
        try:
            self._router_agent()
        except AlbertError:
            return False
        return True

    def _ensure_delegation_approved(self, issue: IssueSlice, agent_config: AgentConfig | None) -> None:
        if not agent_config:
            return
        decision = self.delegations.get(issue.id)
        approval_required = agent_config.requires_approval or is_cloud_model(
            agent_config.model
        )
        if not approval_required:
            return
        if not decision or decision.recommended_agent != agent_config.id or not decision.approved:
            raise LaunchBlockedError(f"{issue.id} delegation requires approval before launch.")

    def _delegation_prompt(self, issue: IssueSlice, router: AgentConfig) -> str:
        candidates = [agent.to_dict() for agent in self._delegation_candidates()]
        return "\n".join(
            [
                "You are Albert's Frontier router.",
                f"Router model: {router.model or router.id}",
                "Choose exactly one worker for this Issue Slice.",
                "Return only JSON with this schema:",
                '{"complexity": "low|medium|high|architectural", "recommended_agent": "agent id", "requires_approval": false, "reason": "short reason"}',
                "Use Gemma workers for low and medium work. Use qwen2.5-coder-14b for complex long-horizon coding. Use deepseek-r1-14b for architectural reasoning or repeated-failure work.",
                "",
                "Issue Slice:",
                json.dumps(
                    {
                        "issue_id": issue.id,
                        "title": issue.title,
                        "goal": issue.what_to_build,
                        "acceptance_criteria": issue.acceptance_criteria,
                        "risk": issue.risk,
                        "type": issue.type,
                    },
                    indent=2,
                    sort_keys=True,
                ),
                "",
                "Candidate agents:",
                json.dumps(candidates, indent=2, sort_keys=True),
                "",
            ]
        )

    def _delegation_candidates(self) -> list[AgentConfig]:
        manual_agents = self.assignment_agents()
        return [
            agent
            for agent in self.agent_registry.agents
            if agent.availability.casefold() == "available"
            and (
                (
                    agent in manual_agents
                    and not agent.requires_approval
                    and not is_cloud_model(agent.model)
                )
                or (
                    agent.delegate_only
                    and (
                        agent.routing.casefold() == "delegate"
                        or (
                            not agent.routing.strip()
                            and agent.role.casefold() == "delegate-agent"
                        )
                    )
                )
            )
        ]

    def _session_worktree_path(self, session_id: str) -> Path:
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", session_id):
            raise AlbertError(f"unsafe session id {session_id!r}")
        worktree_root = (
            self.target_repo.parent / ".albert-worktrees" / self.target_repo.name
            / self.project_key
        ).resolve()
        worktree_path = (worktree_root / session_id).resolve()
        if worktree_path.parent != worktree_root:
            raise AlbertError(f"unsafe session worktree path for {session_id!r}")
        return worktree_path

    def current_worktree_identity(self, session_id: str) -> str:
        """Return an exact managed-path identity, or empty when ownership is unclear."""

        return self._worktree_identity_for_session(self._session(session_id))

    def _worktree_identity_for_session(self, session: LocalAgentSession) -> str:
        session_id = session.session_id
        expected_path = self._session_worktree_path(session_id)
        try:
            canonical_path = session.worktree_path.resolve(strict=True)
        except OSError:
            return ""
        if (
            session.worktree_path.is_symlink()
            or not canonical_path.is_dir()
            or _runtime_identity_path(canonical_path)
            != _runtime_identity_path(expected_path)
            or _runtime_identity_path(canonical_path)
            == _runtime_identity_path(self.target_repo)
        ):
            return ""

        git_pointer = canonical_path / ".git"
        if not git_pointer.exists():
            payload = f"directory\n{self.mission_id}\n{session_id}\n{canonical_path}"
            return "managed-directory:" + sha256(payload.encode("utf-8")).hexdigest()
        if git_pointer.is_symlink() or not git_pointer.is_file():
            return ""
        try:
            pointer_text = git_pointer.read_text(encoding="utf-8")
        except (OSError, UnicodeError):
            return ""
        if len(pointer_text.encode("utf-8")) > 4_096 or not pointer_text.startswith(
            "gitdir: "
        ):
            return ""
        admin_value = pointer_text.removeprefix("gitdir: ").strip()
        admin_path = Path(admin_value)
        if not admin_path.is_absolute():
            admin_path = git_pointer.parent / admin_path
        try:
            admin_path = admin_path.resolve(strict=True)
            registration = (admin_path / "gitdir").read_text(encoding="utf-8").strip()
            registered_pointer = Path(registration).resolve(strict=True)
        except (OSError, UnicodeError):
            return ""
        if (
            _runtime_identity_path(registered_pointer)
            != _runtime_identity_path(git_pointer)
            or admin_path.name != canonical_path.name
        ):
            return ""
        payload = (
            f"git-worktree\n{self.mission_id}\n{session_id}\n"
            f"{canonical_path}\n{admin_path}"
        )
        return "managed-git:" + sha256(payload.encode("utf-8")).hexdigest()

    def _prepare_session_worktree(self, session_id: str) -> Path:
        worktree_path = self._session_worktree_path(session_id)
        if worktree_path.exists():
            raise AlbertError(f"session worktree already exists: {worktree_path}")
        worktree_path.parent.mkdir(parents=True, exist_ok=True)

        git_root = self._target_git_root()
        if git_root is None:
            worktree_path.mkdir()
            return worktree_path
        if _runtime_identity_path(git_root) != _runtime_identity_path(self.target_repo):
            raise AlbertError(
                "target repository must be the Git worktree root before launching a session"
            )
        try:
            completed = _run_bounded_process(
                [
                    "git",
                    "-C",
                    str(git_root),
                    "worktree",
                    "add",
                    "--detach",
                    str(worktree_path),
                    "HEAD",
                ],
                timeout_seconds=_GIT_SNAPSHOT_TIMEOUT_SECONDS,
                output_limit_bytes=_GIT_COMMAND_OUTPUT_BYTES_LIMIT,
            )
        except OSError as exc:
            raise AlbertError(f"unable to create Git worktree: {exc}") from exc
        if completed.returncode != 0:
            reason = (
                self._subprocess_output_text(completed.stderr).strip()
                or self._subprocess_output_text(completed.stdout).strip()
                or "unknown Git error"
            )
            raise AlbertError(f"unable to create Git worktree for {session_id}: {reason}")
        return worktree_path

    def _ensure_session_worktree(self, session: LocalAgentSession) -> None:
        expected_path = self._session_worktree_path(session.session_id)
        if self._target_git_root() is None:
            session.worktree_path = expected_path
            if self._worktree_baseline_is_prepared(session):
                if (
                    not expected_path.exists()
                    or expected_path.is_symlink()
                    or not expected_path.is_dir()
                ):
                    raise AlbertError(
                        f"prepared session worktree is unavailable: {expected_path}"
                    )
                self._resume_repair_worktree_overlay(session)
                return
            expected_path.mkdir(parents=True, exist_ok=True)
            copied, skipped_count, copied_bytes = self._copy_directory_source_files(
                expected_path
            )
            repository_snapshot = {
                "kind": "directory",
                "tracked_diff_applied": False,
                "source_files_copied": copied,
                "source_files_copied_bytes": copied_bytes,
                "source_files_skipped_count": skipped_count,
                "source_scan_limit": _DIRECTORY_SOURCE_SCAN_LIMIT,
            }
            self._capture_and_persist_worktree_baseline(
                session,
                repository_snapshot,
            )
            self._resume_repair_worktree_overlay(session)
            return
        session.worktree_path = expected_path
        with self._worktree_lock():
            try:
                completed = _run_bounded_process(
                    [
                        "git",
                        "-C",
                        str(session.worktree_path),
                        "rev-parse",
                        "--show-toplevel",
                    ],
                    timeout_seconds=_GIT_SNAPSHOT_TIMEOUT_SECONDS,
                    output_limit_bytes=_GIT_COMMAND_OUTPUT_BYTES_LIMIT,
                )
            except OSError as exc:
                raise AlbertError(
                    f"unable to inspect session Git worktree: {exc}"
                ) from exc
            if completed.returncode in {
                124,
                _PROCESS_OUTPUT_LIMIT_EXIT_STATUS,
            }:
                reason = (
                    self._subprocess_output_text(completed.stderr).strip()
                    or "bounded Git inspection failed"
                )
                raise AlbertError(
                    f"unable to inspect session Git worktree: {reason}"
                )
            if (
                completed.returncode == 0
                and _runtime_identity_path(Path(completed.stdout.strip()).resolve())
                == _runtime_identity_path(session.worktree_path.resolve())
                and _runtime_identity_path(session.worktree_path.resolve())
                != _runtime_identity_path(self.target_repo)
            ):
                if not self._worktree_baseline_is_prepared(session):
                    repository_snapshot = self._overlay_target_worktree_state(
                        session
                    )
                    self._capture_and_persist_worktree_baseline(
                        session,
                        repository_snapshot,
                    )
                self._resume_repair_worktree_overlay(session)
                return
            session.worktree_path = self._prepare_session_worktree(
                session.session_id
            )
            repository_snapshot = self._overlay_target_worktree_state(
                session
            )
            self._capture_and_persist_worktree_baseline(
                session,
                repository_snapshot,
            )
            self._resume_repair_worktree_overlay(session)

    def _worktree_baseline_is_prepared(self, session: LocalAgentSession) -> bool:
        marker = session.repository_snapshot.get("preparation")
        if marker is None:
            return False
        if not isinstance(marker, dict) or marker.get("schema_version") != (
            _WORKTREE_PREPARATION_SCHEMA_VERSION
        ):
            raise AlbertError(
                f"{session.session_id} has an invalid worktree preparation marker."
            )
        state = marker.get("state")
        if state in {"target-overlay-pending", "target-overlay-applied"}:
            return False
        if state != "baseline-captured" or set(marker) != {
            "schema_version",
            "state",
        }:
            raise AlbertError(
                f"{session.session_id} has an invalid worktree preparation marker."
            )
        if not isinstance(session.repository_snapshot.get("review_baseline"), dict):
            raise AlbertError(
                f"{session.session_id} prepared worktree has no immutable review baseline."
            )
        return True

    def _capture_and_persist_worktree_baseline(
        self,
        session: LocalAgentSession,
        repository_snapshot: dict[str, Any],
    ) -> None:
        baseline_fingerprints = self._baseline_fingerprints(session.worktree_path)
        repository_snapshot["review_baseline"] = self._capture_review_baseline(
            session,
            baseline_fingerprints,
        )
        repository_snapshot["preparation"] = {
            "schema_version": _WORKTREE_PREPARATION_SCHEMA_VERSION,
            "state": "baseline-captured",
        }
        session.repository_snapshot = repository_snapshot
        session.baseline_fingerprints = baseline_fingerprints
        if isinstance(session.task_packet.get("repair_context"), dict):
            persisted = self._persist_session_update(
                session,
                expected_statuses={"running"},
            )
            if persisted.status == "cancelled":
                raise SessionCancelledError(
                    f"{session.session_id} cancelled while preparing its worktree"
                )

    def _resume_repair_worktree_overlay(self, session: LocalAgentSession) -> None:
        if not isinstance(session.task_packet.get("repair_context"), dict):
            return
        repair_overlay = session.repository_snapshot.get("repair_overlay")
        if isinstance(repair_overlay, dict) and "state" not in repair_overlay:
            # Compatibility for worktrees prepared before staged repair overlays.
            return
        if repair_overlay is None:
            repair_overlay = self._stage_prior_session_state(session)
            session.repository_snapshot["repair_overlay"] = repair_overlay
            persisted = self._persist_session_update(
                session,
                expected_statuses={"running"},
            )
            if persisted.status == "cancelled":
                raise SessionCancelledError(
                    f"{session.session_id} cancelled while preparing its worktree"
                )
        if not isinstance(repair_overlay, dict):
            raise AlbertError(
                f"{session.session_id} has an invalid repair overlay marker."
            )
        state = repair_overlay.get("state")
        if state == "applied":
            return
        if state != "pending":
            raise AlbertError(
                f"{session.session_id} has an invalid repair overlay marker."
            )
        applied_overlay = self._overlay_prior_session_state(session)
        if applied_overlay is None:
            raise AlbertError(
                f"{session.session_id} repair preparation produced no prior overlay."
            )
        session.repository_snapshot["repair_overlay"] = applied_overlay
        persisted = self._persist_session_update(
            session,
            expected_statuses={"running"},
        )
        if persisted.status == "cancelled":
            raise SessionCancelledError(
                f"{session.session_id} cancelled while preparing its worktree"
            )

    @contextmanager
    def _worktree_lock(self):
        self.runtime_dir.mkdir(parents=True, exist_ok=True)
        lock_path = self.runtime_dir / ".worktree.lock"
        with lock_path.open("a+", encoding="utf-8") as lock_file:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)

    def _overlay_target_worktree_state(
        self,
        session: LocalAgentSession,
    ) -> dict[str, Any]:
        worktree_path = session.worktree_path
        marker = session.repository_snapshot.get("preparation")
        marker_state = marker.get("state") if isinstance(marker, dict) else ""
        if marker_state in {"target-overlay-pending", "target-overlay-applied"}:
            repository_snapshot = dict(session.repository_snapshot)
            diff_text = self._read_target_overlay_artifact(
                session,
                repository_snapshot,
            )
        else:
            try:
                diff = _run_bounded_process(
                    [
                        "git",
                        "-C",
                        str(self.target_repo),
                        "diff",
                        "--binary",
                        "--full-index",
                        "--no-ext-diff",
                        "HEAD",
                        "--",
                    ],
                    timeout_seconds=_GIT_SNAPSHOT_TIMEOUT_SECONDS,
                    output_limit_bytes=_GIT_SNAPSHOT_BYTES_LIMIT,
                )
            except OSError as exc:
                raise AlbertError(
                    f"unable to capture target repository changes: {exc}"
                ) from exc
            if (
                diff.returncode == _PROCESS_OUTPUT_LIMIT_EXIT_STATUS
                and "output exceeded" in diff.stderr.lower()
            ):
                raise AlbertError(
                    "target repository snapshot exceeds the "
                    f"{_GIT_SNAPSHOT_BYTES_LIMIT}-byte capture limit; "
                    "commit, stash, or split the parent workspace changes before launching"
                )
            if diff.returncode != 0:
                reason = self._subprocess_output_text(diff.stderr).strip()
                raise AlbertError(
                    "unable to capture target repository changes: "
                    f"{reason or 'unknown Git error'}"
                )
            diff_text = diff.stdout
            diff_payload = diff_text.encode("utf-8")
            artifact_path = (
                self.runtime_dir
                / "sessions"
                / session.session_id
                / "target-overlay.patch"
            )
            self._write(artifact_path, diff_text)
            untracked_snapshot = self._stage_target_untracked_sources(session)
            repository_snapshot = {
                "kind": "git-worktree",
                "tracked_diff_applied": False,
                "tracked_diff_bytes": len(diff_payload),
                "target_diff_artifact": str(artifact_path),
                "target_diff_sha256": sha256(diff_payload).hexdigest(),
                "preparation": {
                    "schema_version": _WORKTREE_PREPARATION_SCHEMA_VERSION,
                    "state": "target-overlay-pending",
                },
                **untracked_snapshot,
            }
            session.repository_snapshot = repository_snapshot
            self._persist_worktree_preparation(session)

        if marker_state != "target-overlay-applied":
            self._apply_prepared_target_diff(session, diff_text)
            self._apply_staged_target_untracked_sources(
                session,
                repository_snapshot,
            )
            repository_snapshot.update(
                {
                    "tracked_diff_applied": bool(diff_text),
                    "preparation": {
                        "schema_version": _WORKTREE_PREPARATION_SCHEMA_VERSION,
                        "state": "target-overlay-applied",
                    },
                }
            )
            session.repository_snapshot = repository_snapshot
            self._persist_worktree_preparation(session)
        return repository_snapshot

    def _persist_worktree_preparation(self, session: LocalAgentSession) -> None:
        persisted = self._persist_session_update(
            session,
            expected_statuses={"running"},
        )
        if persisted.status == "cancelled":
            raise SessionCancelledError(
                f"{session.session_id} cancelled while preparing its worktree"
            )

    def _read_target_overlay_artifact(
        self,
        session: LocalAgentSession,
        repository_snapshot: dict[str, Any],
    ) -> str:
        expected_path = (
            self.runtime_dir
            / "sessions"
            / session.session_id
            / "target-overlay.patch"
        ).absolute()
        artifact_value = repository_snapshot.get("target_diff_artifact")
        digest = repository_snapshot.get("target_diff_sha256")
        if not isinstance(artifact_value, str) or not isinstance(digest, str):
            raise AlbertError(
                f"{session.session_id} has an invalid target overlay marker."
            )
        artifact_path = Path(artifact_value).absolute()
        if (
            _runtime_identity_path(artifact_path)
            != _runtime_identity_path(expected_path)
            or artifact_path.is_symlink()
            or not artifact_path.is_file()
        ):
            raise AlbertError(
                f"{session.session_id} target overlay artifact is unavailable or unsafe."
            )
        try:
            payload = _read_bounded_bytes(
                artifact_path,
                _GIT_SNAPSHOT_BYTES_LIMIT,
            )
        except OSError as exc:
            raise AlbertError(
                f"unable to read {session.session_id} target overlay artifact: {exc}"
            ) from exc
        if len(payload) > _GIT_SNAPSHOT_BYTES_LIMIT:
            raise AlbertError(
                f"{session.session_id} target overlay artifact exceeds its capture limit."
            )
        if sha256(payload).hexdigest() != digest:
            raise AlbertError(
                f"{session.session_id} target overlay artifact failed integrity validation."
            )
        try:
            return payload.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise AlbertError(
                f"{session.session_id} target overlay artifact is not UTF-8."
            ) from exc

    def _apply_prepared_target_diff(
        self,
        session: LocalAgentSession,
        diff_text: str,
    ) -> None:
        if not diff_text:
            return
        base_argv = [
            "git",
            "-C",
            str(session.worktree_path),
            "apply",
            "--binary",
            "--whitespace=nowarn",
        ]

        def run_apply(*arguments: str) -> subprocess.CompletedProcess[str]:
            try:
                return _run_bounded_process(
                    [*base_argv, *arguments, "-"],
                    input_text=diff_text,
                    timeout_seconds=_GIT_SNAPSHOT_TIMEOUT_SECONDS,
                    output_limit_bytes=_GIT_COMMAND_OUTPUT_BYTES_LIMIT,
                )
            except OSError as exc:
                raise AlbertError(
                    f"unable to apply target repository changes: {exc}"
                ) from exc

        forward_check = run_apply("--check")
        if forward_check.returncode == 0:
            applied = run_apply()
            if applied.returncode == 0:
                return
            reason = self._subprocess_output_text(applied.stderr).strip()
            raise AlbertError(
                "unable to apply target repository changes: "
                f"{reason or 'unknown Git error'}"
            )

        reverse_check = run_apply("--reverse", "--check")
        if reverse_check.returncode == 0:
            return
        forward_reason = self._subprocess_output_text(forward_check.stderr).strip()
        reverse_reason = self._subprocess_output_text(reverse_check.stderr).strip()
        raise AlbertError(
            "unable to recover target repository changes because the prepared patch is "
            "neither fully unapplied nor fully applied: "
            f"{forward_reason or reverse_reason or 'unknown Git error'}"
        )

    def _stage_prior_session_state(
        self,
        session: LocalAgentSession,
    ) -> dict[str, Any]:
        repair_context = session.task_packet.get("repair_context")
        if not isinstance(repair_context, dict):
            raise AlbertError(
                f"{session.session_id} has no repair context to stage."
            )
        prior_session_id = repair_context.get("prior_session_id")
        if not isinstance(prior_session_id, str) or not prior_session_id.strip():
            raise AlbertError(
                f"{session.session_id} repair context has no prior session id."
            )
        prior_session = self.sessions.get(prior_session_id)
        if prior_session is None:
            raise AlbertError(
                f"{session.session_id} repair context references unknown session "
                f"{prior_session_id}."
            )
        prior_worktree = self._session_worktree_path(prior_session_id)
        if not prior_worktree.exists():
            raise AlbertError(
                f"{session.session_id} cannot inherit missing prior session worktree "
                f"{prior_session_id}."
            )

        changed_files = self._worktree_changed_files(
            prior_worktree,
            prior_session.baseline_fingerprints,
        )
        if len(changed_files) > _REPAIR_INHERITED_FILE_LIMIT:
            raise AlbertError(
                f"{session.session_id} prior session patch exceeds the "
                f"{_REPAIR_INHERITED_FILE_LIMIT}-file repair limit."
            )

        staging_root = (
            self.runtime_dir
            / "sessions"
            / session.session_id
            / "repair-overlay"
        )
        files_root = staging_root / "files"
        staging_root.parent.mkdir(parents=True, exist_ok=True)
        if staging_root.is_symlink():
            raise AlbertError(
                f"unsafe repair overlay staging directory for {session.session_id}"
            )
        staging_root.mkdir(exist_ok=True)
        if files_root.is_symlink():
            raise AlbertError(
                f"unsafe repair overlay staging directory for {session.session_id}"
            )
        files_root.mkdir(exist_ok=True)
        if (
            staging_root.is_symlink()
            or files_root.is_symlink()
            or not staging_root.is_dir()
            or not files_root.is_dir()
        ):
            raise AlbertError(
                f"unsafe repair overlay staging directory for {session.session_id}"
            )

        entries: list[dict[str, Any]] = []
        applied: list[str] = []
        deleted: list[str] = []
        inherited_bytes = 0
        for relative_path in changed_files:
            if self.classify_file_for_frontier(relative_path) != "Normal":
                raise AlbertError(
                    f"{session.session_id} prior session patch contains a non-visible "
                    f"path: {relative_path}."
                )
            path = PurePosixPath(relative_path.replace("\\", "/"))
            source_entry = prior_worktree.joinpath(*path.parts)
            destination_entry = session.worktree_path.joinpath(*path.parts)
            if source_entry.is_symlink() or destination_entry.is_symlink():
                raise AlbertError(
                    f"{session.session_id} prior session patch contains a symlink path: "
                    f"{relative_path}."
                )
            source = self._safe_worktree_destination(
                prior_worktree,
                relative_path,
            )
            destination = self._safe_worktree_destination(
                session.worktree_path,
                relative_path,
            )
            if not self._model_path_is_allowed(session, destination):
                raise AlbertError(
                    f"{session.session_id} prior session patch is outside allowed_paths: "
                    f"{relative_path}."
                )
            if not source.exists():
                entries.append(
                    {
                        "path": relative_path,
                        "state": "deleted",
                        "bytes": 0,
                    }
                )
                deleted.append(relative_path)
                continue
            if not source.is_file():
                raise AlbertError(
                    f"{session.session_id} prior session patch path is not a file: "
                    f"{relative_path}."
                )
            try:
                payload = _read_bounded_bytes(
                    source,
                    _REPAIR_INHERITED_FILE_BYTES_LIMIT,
                )
            except OSError as exc:
                raise AlbertError(
                    f"{session.session_id} cannot read prior session patch path "
                    f"{relative_path}: {exc}"
                ) from exc
            if len(payload) > _REPAIR_INHERITED_FILE_BYTES_LIMIT:
                raise AlbertError(
                    f"{session.session_id} prior session patch path exceeds the "
                    f"{_REPAIR_INHERITED_FILE_BYTES_LIMIT}-byte file limit: "
                    f"{relative_path}."
                )
            if inherited_bytes + len(payload) > _REPAIR_INHERITED_TOTAL_BYTES_LIMIT:
                raise AlbertError(
                    f"{session.session_id} prior session patch exceeds the "
                    f"{_REPAIR_INHERITED_TOTAL_BYTES_LIMIT}-byte total limit."
                )
            try:
                mode = source.lstat().st_mode & 0o777
            except OSError as exc:
                raise AlbertError(
                    f"{session.session_id} cannot inspect prior session patch path "
                    f"{relative_path}: {exc}"
                ) from exc
            artifact_name = sha256(
                relative_path.encode("utf-8", errors="surrogateescape")
            ).hexdigest()
            artifact_path = files_root / artifact_name
            if artifact_path.is_symlink():
                raise AlbertError(
                    f"unsafe repair overlay staging artifact for {relative_path}"
                )
            artifact_path.write_bytes(payload)
            entries.append(
                {
                    "path": relative_path,
                    "state": "file",
                    "artifact": artifact_name,
                    "sha256": sha256(payload).hexdigest(),
                    "bytes": len(payload),
                    "mode": mode,
                }
            )
            applied.append(relative_path)
            inherited_bytes += len(payload)

        manifest = {
            "schema_version": 1,
            "prior_session_id": prior_session_id,
            "entries": entries,
            "inherited_bytes": inherited_bytes,
            "file_limit": _REPAIR_INHERITED_FILE_LIMIT,
            "file_bytes_limit": _REPAIR_INHERITED_FILE_BYTES_LIMIT,
            "total_bytes_limit": _REPAIR_INHERITED_TOTAL_BYTES_LIMIT,
        }
        manifest_text = json.dumps(manifest, indent=2, sort_keys=True) + "\n"
        manifest_payload = manifest_text.encode("utf-8")
        if len(manifest_payload) > _REPAIR_INHERITED_MANIFEST_BYTES_LIMIT:
            raise AlbertError(
                f"{session.session_id} repair overlay manifest exceeds the "
                f"{_REPAIR_INHERITED_MANIFEST_BYTES_LIMIT}-byte limit."
            )
        manifest_path = staging_root / "manifest.json"
        if manifest_path.is_symlink():
            raise AlbertError(
                f"unsafe repair overlay manifest for {session.session_id}"
            )
        self._write(manifest_path, manifest_text)
        return {
            "schema_version": 1,
            "state": "pending",
            "prior_session_id": prior_session_id,
            "manifest_artifact": str(manifest_path),
            "manifest_sha256": sha256(manifest_payload).hexdigest(),
            "applied_files": applied,
            "deleted_files": deleted,
            "inherited_bytes": inherited_bytes,
            "file_limit": _REPAIR_INHERITED_FILE_LIMIT,
            "file_bytes_limit": _REPAIR_INHERITED_FILE_BYTES_LIMIT,
            "total_bytes_limit": _REPAIR_INHERITED_TOTAL_BYTES_LIMIT,
        }

    def _overlay_prior_session_state(
        self,
        session: LocalAgentSession,
    ) -> dict[str, Any] | None:
        repair_context = session.task_packet.get("repair_context")
        if not isinstance(repair_context, dict):
            return None
        marker = session.repository_snapshot.get("repair_overlay")
        if not isinstance(marker, dict) or marker.get("state") != "pending":
            raise AlbertError(
                f"{session.session_id} has no pending repair overlay marker."
            )
        for entry, payload in self._validated_repair_overlay_entries(
            session,
            marker,
        ):
            relative_path = entry["path"]
            path = PurePosixPath(relative_path)
            destination_entry = session.worktree_path.joinpath(*path.parts)
            if destination_entry.is_symlink():
                raise AlbertError(
                    f"{session.session_id} repair overlay destination is a symlink: "
                    f"{relative_path}."
                )
            destination = self._safe_worktree_destination(
                session.worktree_path,
                relative_path,
            )
            if entry["state"] == "deleted":
                if destination.exists() and destination.is_dir():
                    raise AlbertError(
                        f"{session.session_id} cannot inherit deletion of directory "
                        f"{relative_path}."
                    )
                destination.unlink(missing_ok=True)
                continue
            if destination.exists() and destination.is_dir():
                raise AlbertError(
                    f"{session.session_id} cannot replace directory {relative_path}."
                )
            destination.parent.mkdir(parents=True, exist_ok=True)
            assert payload is not None
            destination.write_bytes(payload)
            destination.chmod(entry["mode"])
        return {**marker, "state": "applied"}

    def _validated_repair_overlay_entries(
        self,
        session: LocalAgentSession,
        marker: dict[str, Any],
    ) -> list[tuple[dict[str, Any], bytes | None]]:
        expected_root = (
            self.runtime_dir
            / "sessions"
            / session.session_id
            / "repair-overlay"
        ).absolute()
        expected_manifest = expected_root / "manifest.json"
        files_root = expected_root / "files"
        if (
            expected_root.is_symlink()
            or files_root.is_symlink()
            or not expected_root.is_dir()
            or not files_root.is_dir()
        ):
            raise AlbertError(
                f"{session.session_id} repair overlay staging directory is unavailable or unsafe."
            )
        manifest_value = marker.get("manifest_artifact")
        manifest_digest = marker.get("manifest_sha256")
        if not isinstance(manifest_value, str) or not isinstance(
            manifest_digest,
            str,
        ):
            raise AlbertError(
                f"{session.session_id} repair overlay marker is invalid."
            )
        manifest_path = Path(manifest_value).absolute()
        if (
            _runtime_identity_path(manifest_path)
            != _runtime_identity_path(expected_manifest)
            or manifest_path.is_symlink()
            or not manifest_path.is_file()
        ):
            raise AlbertError(
                f"{session.session_id} repair overlay manifest is unavailable or unsafe."
            )
        try:
            manifest_payload = _read_bounded_bytes(
                manifest_path,
                _REPAIR_INHERITED_MANIFEST_BYTES_LIMIT,
            )
        except OSError as exc:
            raise AlbertError(
                f"unable to read {session.session_id} repair overlay manifest: {exc}"
            ) from exc
        if (
            len(manifest_payload) > _REPAIR_INHERITED_MANIFEST_BYTES_LIMIT
            or sha256(manifest_payload).hexdigest() != manifest_digest
        ):
            raise AlbertError(
                f"{session.session_id} repair overlay manifest failed integrity validation."
            )
        try:
            manifest = json.loads(manifest_payload.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise AlbertError(
                f"{session.session_id} repair overlay manifest is malformed."
            ) from exc
        entries = manifest.get("entries") if isinstance(manifest, dict) else None
        prior_session_id = manifest.get("prior_session_id") if isinstance(
            manifest,
            dict,
        ) else None
        if (
            manifest.get("schema_version") != 1
            or not isinstance(entries, list)
            or len(entries) > _REPAIR_INHERITED_FILE_LIMIT
            or not isinstance(prior_session_id, str)
            or prior_session_id != marker.get("prior_session_id")
        ):
            raise AlbertError(
                f"{session.session_id} repair overlay manifest is invalid."
            )

        validated: list[tuple[dict[str, Any], bytes | None]] = []
        applied: list[str] = []
        deleted: list[str] = []
        paths: set[str] = set()
        artifacts: set[str] = set()
        inherited_bytes = 0
        for raw_entry in entries:
            if not isinstance(raw_entry, dict):
                raise AlbertError(
                    f"{session.session_id} repair overlay manifest entry is invalid."
                )
            relative_path = raw_entry.get("path")
            state = raw_entry.get("state")
            path = (
                PurePosixPath(relative_path)
                if isinstance(relative_path, str)
                else PurePosixPath()
            )
            if (
                not isinstance(relative_path, str)
                or not path.parts
                or path.is_absolute()
                or ".." in path.parts
                or "\\" in relative_path
                or relative_path in paths
                or state not in {"file", "deleted"}
            ):
                raise AlbertError(
                    f"{session.session_id} repair overlay manifest entry is invalid."
                )
            destination = self._safe_worktree_destination(
                session.worktree_path,
                relative_path,
            )
            if not self._model_path_is_allowed(session, destination):
                raise AlbertError(
                    f"{session.session_id} repair overlay is outside allowed_paths: "
                    f"{relative_path}."
                )
            paths.add(relative_path)
            if state == "deleted":
                if raw_entry.get("bytes") != 0:
                    raise AlbertError(
                        f"{session.session_id} repair overlay deletion is invalid."
                    )
                deleted.append(relative_path)
                validated.append((dict(raw_entry), None))
                continue
            artifact_name = raw_entry.get("artifact")
            digest = raw_entry.get("sha256")
            size = raw_entry.get("bytes")
            mode = raw_entry.get("mode")
            expected_artifact = sha256(
                relative_path.encode("utf-8", errors="surrogateescape")
            ).hexdigest()
            if (
                artifact_name != expected_artifact
                or artifact_name in artifacts
                or not isinstance(digest, str)
                or re.fullmatch(r"[0-9a-f]{64}", digest) is None
                or not isinstance(size, int)
                or isinstance(size, bool)
                or size < 0
                or size > _REPAIR_INHERITED_FILE_BYTES_LIMIT
                or not isinstance(mode, int)
                or isinstance(mode, bool)
                or mode < 0
                or mode > 0o777
            ):
                raise AlbertError(
                    f"{session.session_id} repair overlay file entry is invalid."
                )
            artifact_path = files_root / artifact_name
            if artifact_path.is_symlink() or not artifact_path.is_file():
                raise AlbertError(
                    f"{session.session_id} repair overlay artifact is unavailable or unsafe."
                )
            try:
                payload = _read_bounded_bytes(
                    artifact_path,
                    _REPAIR_INHERITED_FILE_BYTES_LIMIT,
                )
            except OSError as exc:
                raise AlbertError(
                    f"unable to read {session.session_id} repair overlay artifact: {exc}"
                ) from exc
            if len(payload) != size or sha256(payload).hexdigest() != digest:
                raise AlbertError(
                    f"{session.session_id} repair overlay artifact failed integrity validation."
                )
            inherited_bytes += len(payload)
            if inherited_bytes > _REPAIR_INHERITED_TOTAL_BYTES_LIMIT:
                raise AlbertError(
                    f"{session.session_id} repair overlay exceeds its total byte limit."
                )
            artifacts.add(artifact_name)
            applied.append(relative_path)
            validated.append((dict(raw_entry), payload))

        if (
            manifest.get("inherited_bytes") != inherited_bytes
            or marker.get("inherited_bytes") != inherited_bytes
            or marker.get("applied_files") != applied
            or marker.get("deleted_files") != deleted
        ):
            raise AlbertError(
                f"{session.session_id} repair overlay marker does not match its manifest."
            )
        return validated

    def _stage_target_untracked_sources(
        self,
        session: LocalAgentSession,
    ) -> dict[str, Any]:
        candidates = self._bounded_git_paths(
            self.target_repo,
            ["ls-files", "--others", "--exclude-standard", "-z", "--"],
            description="untracked source file listing",
            required=True,
        )
        assert candidates is not None
        entries: list[dict[str, Any]] = []
        skipped_count = 0
        staged_bytes = 0
        staging_root = (
            self.runtime_dir
            / "sessions"
            / session.session_id
            / "target-untracked"
        )
        files_root = staging_root / "files"
        staging_root.parent.mkdir(parents=True, exist_ok=True)
        if staging_root.is_symlink():
            raise AlbertError(
                f"unsafe target untracked staging directory for {session.session_id}"
            )
        staging_root.mkdir(exist_ok=True)
        if files_root.is_symlink():
            raise AlbertError(
                f"unsafe target untracked staging directory for {session.session_id}"
            )
        files_root.mkdir(exist_ok=True)
        if (
            staging_root.is_symlink()
            or files_root.is_symlink()
            or not staging_root.is_dir()
            or not files_root.is_dir()
        ):
            raise AlbertError(
                f"unsafe target untracked staging directory for {session.session_id}"
            )
        excluded_parts = {
            ".git",
            ".albert-worktrees",
            ".venv",
            "__pycache__",
            "build",
            "dist",
            "node_modules",
            "target",
            "venv",
        }
        for relative_path in candidates:
            normalized = relative_path.replace("\\", "/")
            path = PurePosixPath(normalized)
            if (
                not path.parts
                or ".." in path.parts
                or any(part in excluded_parts for part in path.parts)
                or self.classify_file_for_frontier(normalized) != "Normal"
            ):
                skipped_count += 1
                continue
            source = self.target_repo.joinpath(*path.parts)
            if source.is_symlink() or not source.is_file():
                skipped_count += 1
                continue
            if (
                source.name not in _TEXT_SOURCE_NAMES
                and source.suffix.lower() not in _TEXT_SOURCE_SUFFIXES
            ):
                skipped_count += 1
                continue
            try:
                payload = _read_bounded_bytes(
                    source,
                    _UNTRACKED_SOURCE_FILE_BYTES_LIMIT,
                )
            except OSError:
                skipped_count += 1
                continue
            if (
                len(payload) > _UNTRACKED_SOURCE_FILE_BYTES_LIMIT
                or b"\0" in payload
                or len(entries) >= _UNTRACKED_SOURCE_FILE_LIMIT
                or staged_bytes + len(payload) > _UNTRACKED_SOURCE_TOTAL_BYTES_LIMIT
            ):
                skipped_count += 1
                continue
            try:
                destination_entry = session.worktree_path.joinpath(*path.parts)
                if destination_entry.is_symlink():
                    raise AlbertError(
                        f"target untracked destination is a symlink: {normalized}"
                    )
                self._safe_worktree_destination(
                    session.worktree_path,
                    normalized,
                )
                mode = source.lstat().st_mode & 0o777
            except (AlbertError, OSError):
                skipped_count += 1
                continue
            artifact_name = sha256(
                normalized.encode("utf-8", errors="surrogateescape")
            ).hexdigest()
            artifact_path = files_root / artifact_name
            if artifact_path.is_symlink():
                raise AlbertError(
                    f"unsafe target untracked staging artifact for {normalized}"
                )
            artifact_path.write_bytes(payload)
            entries.append(
                {
                    "path": normalized,
                    "artifact": artifact_name,
                    "sha256": sha256(payload).hexdigest(),
                    "bytes": len(payload),
                    "mode": mode,
                }
            )
            staged_bytes += len(payload)

        manifest = {
            "schema_version": 1,
            "entries": entries,
            "skipped_count": skipped_count,
            "staged_bytes": staged_bytes,
            "file_limit": _UNTRACKED_SOURCE_FILE_LIMIT,
            "file_bytes_limit": _UNTRACKED_SOURCE_FILE_BYTES_LIMIT,
            "total_bytes_limit": _UNTRACKED_SOURCE_TOTAL_BYTES_LIMIT,
        }
        manifest_text = json.dumps(manifest, indent=2, sort_keys=True) + "\n"
        manifest_payload = manifest_text.encode("utf-8")
        if len(manifest_payload) > _UNTRACKED_SOURCE_MANIFEST_BYTES_LIMIT:
            raise AlbertError(
                f"{session.session_id} target untracked manifest exceeds the "
                f"{_UNTRACKED_SOURCE_MANIFEST_BYTES_LIMIT}-byte limit"
            )
        manifest_path = staging_root / "manifest.json"
        if manifest_path.is_symlink():
            raise AlbertError(
                f"unsafe target untracked manifest for {session.session_id}"
            )
        self._write(manifest_path, manifest_text)
        return {
            "untracked_manifest_artifact": str(manifest_path),
            "untracked_manifest_sha256": sha256(manifest_payload).hexdigest(),
            "untracked_files_copied": [entry["path"] for entry in entries],
            "untracked_files_copied_bytes": staged_bytes,
            "untracked_files_skipped_count": skipped_count,
        }

    def _apply_staged_target_untracked_sources(
        self,
        session: LocalAgentSession,
        repository_snapshot: dict[str, Any],
    ) -> None:
        for entry, payload in self._validated_target_untracked_entries(
            session,
            repository_snapshot,
        ):
            path = PurePosixPath(entry["path"])
            destination_entry = session.worktree_path.joinpath(*path.parts)
            if destination_entry.is_symlink():
                raise AlbertError(
                    f"{session.session_id} target untracked destination is a symlink: "
                    f"{entry['path']}"
                )
            destination = self._safe_worktree_destination(
                session.worktree_path,
                entry["path"],
            )
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(payload)
            destination.chmod(entry["mode"])

    def _validated_target_untracked_entries(
        self,
        session: LocalAgentSession,
        repository_snapshot: dict[str, Any],
    ) -> list[tuple[dict[str, Any], bytes]]:
        expected_root = (
            self.runtime_dir
            / "sessions"
            / session.session_id
            / "target-untracked"
        ).absolute()
        expected_manifest = expected_root / "manifest.json"
        files_root = expected_root / "files"
        if (
            expected_root.is_symlink()
            or files_root.is_symlink()
            or not expected_root.is_dir()
            or not files_root.is_dir()
        ):
            raise AlbertError(
                f"{session.session_id} target untracked staging directory is unavailable or unsafe."
            )
        manifest_value = repository_snapshot.get("untracked_manifest_artifact")
        manifest_digest = repository_snapshot.get("untracked_manifest_sha256")
        if not isinstance(manifest_value, str) or not isinstance(
            manifest_digest,
            str,
        ):
            raise AlbertError(
                f"{session.session_id} has no valid target untracked manifest marker."
            )
        manifest_path = Path(manifest_value).absolute()
        if (
            _runtime_identity_path(manifest_path)
            != _runtime_identity_path(expected_manifest)
            or manifest_path.is_symlink()
            or not manifest_path.is_file()
        ):
            raise AlbertError(
                f"{session.session_id} target untracked manifest is unavailable or unsafe."
            )
        try:
            manifest_payload = _read_bounded_bytes(
                manifest_path,
                _UNTRACKED_SOURCE_MANIFEST_BYTES_LIMIT,
            )
        except OSError as exc:
            raise AlbertError(
                f"unable to read {session.session_id} target untracked manifest: {exc}"
            ) from exc
        if (
            len(manifest_payload) > _UNTRACKED_SOURCE_MANIFEST_BYTES_LIMIT
            or sha256(manifest_payload).hexdigest() != manifest_digest
        ):
            raise AlbertError(
                f"{session.session_id} target untracked manifest failed integrity validation."
            )
        try:
            manifest = json.loads(manifest_payload.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise AlbertError(
                f"{session.session_id} target untracked manifest is malformed."
            ) from exc
        entries = manifest.get("entries") if isinstance(manifest, dict) else None
        if (
            not isinstance(entries, list)
            or manifest.get("schema_version") != 1
            or len(entries) > _UNTRACKED_SOURCE_FILE_LIMIT
        ):
            raise AlbertError(
                f"{session.session_id} target untracked manifest is invalid."
            )
        staged_bytes = manifest.get("staged_bytes")
        skipped_count = manifest.get("skipped_count")
        if (
            not isinstance(staged_bytes, int)
            or isinstance(staged_bytes, bool)
            or staged_bytes < 0
            or staged_bytes > _UNTRACKED_SOURCE_TOTAL_BYTES_LIMIT
            or not isinstance(skipped_count, int)
            or isinstance(skipped_count, bool)
            or skipped_count < 0
        ):
            raise AlbertError(
                f"{session.session_id} target untracked manifest bounds are invalid."
            )

        validated: list[tuple[dict[str, Any], bytes]] = []
        paths: set[str] = set()
        artifacts: set[str] = set()
        total_bytes = 0
        for raw_entry in entries:
            if not isinstance(raw_entry, dict):
                raise AlbertError(
                    f"{session.session_id} target untracked manifest entry is invalid."
                )
            relative_path = raw_entry.get("path")
            artifact_name = raw_entry.get("artifact")
            digest = raw_entry.get("sha256")
            size = raw_entry.get("bytes")
            mode = raw_entry.get("mode")
            path = (
                PurePosixPath(relative_path.replace("\\", "/"))
                if isinstance(relative_path, str)
                else PurePosixPath()
            )
            expected_artifact = (
                sha256(
                    relative_path.encode("utf-8", errors="surrogateescape")
                ).hexdigest()
                if isinstance(relative_path, str)
                else ""
            )
            if (
                not isinstance(relative_path, str)
                or not path.parts
                or path.is_absolute()
                or ".." in path.parts
                or "\\" in relative_path
                or relative_path in paths
                or artifact_name != expected_artifact
                or artifact_name in artifacts
                or not isinstance(digest, str)
                or re.fullmatch(r"[0-9a-f]{64}", digest) is None
                or not isinstance(size, int)
                or isinstance(size, bool)
                or size < 0
                or size > _UNTRACKED_SOURCE_FILE_BYTES_LIMIT
                or not isinstance(mode, int)
                or isinstance(mode, bool)
                or mode < 0
                or mode > 0o777
            ):
                raise AlbertError(
                    f"{session.session_id} target untracked manifest entry is invalid."
                )
            artifact_path = files_root / artifact_name
            if artifact_path.is_symlink() or not artifact_path.is_file():
                raise AlbertError(
                    f"{session.session_id} target untracked artifact is unavailable or unsafe."
                )
            try:
                payload = _read_bounded_bytes(
                    artifact_path,
                    _UNTRACKED_SOURCE_FILE_BYTES_LIMIT,
                )
            except OSError as exc:
                raise AlbertError(
                    f"unable to read {session.session_id} target untracked artifact: {exc}"
                ) from exc
            if len(payload) != size or sha256(payload).hexdigest() != digest:
                raise AlbertError(
                    f"{session.session_id} target untracked artifact failed integrity validation."
                )
            total_bytes += len(payload)
            if total_bytes > _UNTRACKED_SOURCE_TOTAL_BYTES_LIMIT:
                raise AlbertError(
                    f"{session.session_id} target untracked artifacts exceed their total limit."
                )
            paths.add(relative_path)
            artifacts.add(artifact_name)
            validated.append((dict(raw_entry), payload))

        if (
            total_bytes != staged_bytes
            or repository_snapshot.get("untracked_files_copied")
            != [entry["path"] for entry, _payload in validated]
            or repository_snapshot.get("untracked_files_copied_bytes")
            != staged_bytes
            or repository_snapshot.get("untracked_files_skipped_count")
            != skipped_count
        ):
            raise AlbertError(
                f"{session.session_id} target untracked marker does not match its manifest."
            )
        return validated

    def _copy_directory_source_files(
        self,
        worktree_path: Path,
    ) -> tuple[list[str], int, int]:
        copied: list[str] = []
        skipped_count = 0
        copied_bytes = 0
        remaining_entries = _DIRECTORY_SOURCE_SCAN_LIMIT
        excluded_parts = {
            ".albert-worktrees",
            ".git",
            ".venv",
            "__pycache__",
            "build",
            "dist",
            "node_modules",
            "target",
            "venv",
        }
        pending_directories = [self.target_repo]
        while pending_directories and remaining_entries > 0:
            directory = pending_directories.pop()
            discovered_entries: list[os.DirEntry[str]] = []
            try:
                entries = os.scandir(directory)
            except OSError:
                skipped_count += 1
                continue
            directory_exhausted = False
            with entries:
                while remaining_entries > 0:
                    try:
                        entry = next(entries)
                    except StopIteration:
                        directory_exhausted = True
                        break
                    discovered_entries.append(entry)
                    remaining_entries -= 1
            child_directories: list[Path] = []
            for entry in sorted(discovered_entries, key=lambda candidate: candidate.name):
                try:
                    if entry.is_symlink():
                        skipped_count += 1
                        continue
                    if entry.is_dir(follow_symlinks=False):
                        if entry.name not in excluded_parts:
                            child_directories.append(Path(entry.path))
                        continue
                    if not entry.is_file(follow_symlinks=False):
                        skipped_count += 1
                        continue
                except OSError:
                    skipped_count += 1
                    continue
                source = Path(entry.path)
                relative_path = source.relative_to(self.target_repo).as_posix()
                if (
                    source.is_symlink()
                    or self.classify_file_for_frontier(relative_path) != "Normal"
                    or (
                        source.name not in _TEXT_SOURCE_NAMES
                        and source.suffix.lower() not in _TEXT_SOURCE_SUFFIXES
                    )
                ):
                    skipped_count += 1
                    continue
                try:
                    payload = _read_bounded_bytes(
                        source,
                        _UNTRACKED_SOURCE_FILE_BYTES_LIMIT,
                    )
                except OSError:
                    skipped_count += 1
                    continue
                if (
                    len(payload) > _UNTRACKED_SOURCE_FILE_BYTES_LIMIT
                    or b"\0" in payload
                    or len(copied) >= _UNTRACKED_SOURCE_FILE_LIMIT
                    or copied_bytes + len(payload)
                    > _UNTRACKED_SOURCE_TOTAL_BYTES_LIMIT
                ):
                    skipped_count += 1
                    continue
                try:
                    destination = self._safe_worktree_destination(
                        worktree_path,
                        relative_path,
                    )
                except AlbertError:
                    skipped_count += 1
                    continue
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(payload)
                shutil.copymode(source, destination, follow_symlinks=False)
                copied.append(relative_path)
                copied_bytes += len(payload)
            pending_directories.extend(
                reversed(sorted(child_directories, key=lambda path: path.name))
            )
            if not directory_exhausted and remaining_entries == 0:
                skipped_count += 1
                break
        return copied, skipped_count, copied_bytes

    def _baseline_fingerprints(self, worktree_path: Path) -> dict[str, str]:
        return {
            relative_path: self._worktree_file_fingerprint(
                worktree_path,
                relative_path,
            )
            for relative_path in self._worktree_changed_files(worktree_path)
        }

    def _capture_review_baseline(
        self,
        session: LocalAgentSession,
        baseline_fingerprints: dict[str, str],
    ) -> dict[str, Any]:
        paths = sorted(baseline_fingerprints)
        if len(paths) > _REVIEW_BASELINE_FILE_LIMIT:
            raise AlbertError(
                "target repository launch baseline exceeds the "
                f"{_REVIEW_BASELINE_FILE_LIMIT}-file capture limit; "
                "commit, stash, or split the parent workspace changes before launching"
            )
        baseline_root = (
            self.runtime_dir
            / "sessions"
            / session.session_id
            / "review-baseline"
        )
        baseline_root.mkdir(parents=True, exist_ok=True)
        if baseline_root.is_symlink() or not baseline_root.is_dir():
            raise AlbertError(
                f"unsafe review baseline directory for {session.session_id}"
            )
        entries: dict[str, dict[str, Any]] = {}
        captured_bytes = 0
        for relative_path in paths:
            candidate = self._safe_worktree_destination(
                session.worktree_path,
                relative_path,
            )
            if not candidate.exists() and not candidate.is_symlink():
                entries[relative_path] = {"state": "missing", "size": 0}
                continue
            try:
                size = candidate.lstat().st_size
            except OSError as exc:
                raise AlbertError(
                    "unable to capture target repository launch baseline for "
                    f"{relative_path}: {exc}"
                ) from exc
            if candidate.is_symlink() or not candidate.is_file():
                entries[relative_path] = {
                    "state": "unsupported",
                    "size": size,
                }
                continue
            if size > _REVIEW_BASELINE_FILE_BYTES_LIMIT:
                entries[relative_path] = {"state": "oversized", "size": size}
                continue
            try:
                payload = _read_bounded_bytes(
                    candidate,
                    _REVIEW_BASELINE_FILE_BYTES_LIMIT,
                )
            except OSError as exc:
                raise AlbertError(
                    "unable to capture target repository launch baseline for "
                    f"{relative_path}: {exc}"
                ) from exc
            if len(payload) > _REVIEW_BASELINE_FILE_BYTES_LIMIT:
                entries[relative_path] = {"state": "oversized", "size": size}
                continue
            if captured_bytes + len(payload) > _REVIEW_BASELINE_TOTAL_BYTES_LIMIT:
                raise AlbertError(
                    "target repository launch baseline exceeds the "
                    f"{_REVIEW_BASELINE_TOTAL_BYTES_LIMIT}-byte capture limit; "
                    "commit, stash, or split the parent workspace changes before launching"
                )
            snapshot_name = sha256(
                relative_path.encode("utf-8", errors="surrogateescape")
            ).hexdigest()
            snapshot_path = baseline_root / snapshot_name
            snapshot_path.write_bytes(payload)
            entries[relative_path] = {
                "state": "file",
                "size": size,
                "snapshot": snapshot_name,
            }
            captured_bytes += len(payload)
        return {
            "root": str(baseline_root),
            "entries": entries,
            "captured_bytes": captured_bytes,
            "file_limit": _REVIEW_BASELINE_FILE_LIMIT,
            "file_bytes_limit": _REVIEW_BASELINE_FILE_BYTES_LIMIT,
            "total_bytes_limit": _REVIEW_BASELINE_TOTAL_BYTES_LIMIT,
        }

    def _target_git_root(self) -> Path | None:
        try:
            completed = _run_bounded_process(
                ["git", "-C", str(self.target_repo), "rev-parse", "--show-toplevel"],
                timeout_seconds=_GIT_SNAPSHOT_TIMEOUT_SECONDS,
                output_limit_bytes=_GIT_COMMAND_OUTPUT_BYTES_LIMIT,
            )
        except OSError as exc:
            raise AlbertError(
                f"unable to inspect target Git repository: {exc}"
            ) from exc
        if completed.returncode != 0:
            reason = (
                self._subprocess_output_text(completed.stderr).strip()
                or self._subprocess_output_text(completed.stdout).strip()
                or f"Git exited {completed.returncode} without an explanation"
            )
            if _is_explicit_git_not_repository(completed):
                if not _git_metadata_exists(self.target_repo):
                    return None
                raise AlbertError(
                    "unable to inspect target Git repository: Git metadata exists but "
                    f"Git reported {reason}"
                )
            raise AlbertError(
                f"unable to inspect target Git repository: {reason}"
            )
        root = completed.stdout.strip()
        if not root:
            raise AlbertError(
                "unable to inspect target Git repository: Git returned an empty root"
            )
        return Path(root).resolve()

    def _validate_target_repository_boundary(self) -> None:
        git_root = self._target_git_root()
        if git_root is None:
            return
        if _runtime_identity_path(git_root) == _runtime_identity_path(self.target_repo):
            return
        raise LaunchBlockedError(
            "Selected workspace is a Git repository subdirectory: "
            f"{self.target_repo}. Select the repository root instead: {git_root}. "
            "Albert will not silently widen the Local Agent boundary."
        )

    def _repository_context(self, session: LocalAgentSession) -> str:
        repository_files = self._repository_files(session.worktree_path)
        tree_lines: list[str] = []
        tree_length = 0
        for relative_path in repository_files:
            line = f"- {relative_path}\n"
            if tree_length + len(line) > _REPOSITORY_TREE_LIMIT:
                tree_lines.append("- ... tracked file tree truncated ...\n")
                break
            tree_lines.append(line)
            tree_length += len(line)

        sources = self._rank_repository_sources(session, repository_files)
        source_sections: list[str] = []
        for relative_path in sources[:_REPOSITORY_SOURCE_COUNT]:
            content = self._read_repository_source(session.worktree_path, relative_path)
            if content is None:
                continue
            source_sections.append(f"### {relative_path}\n{content.rstrip()}\n")

        context = "\n".join(
            [
                "Repository context (bounded)",
                "Tracked file tree:",
                "".join(tree_lines).rstrip() or "(no tracked files discovered)",
                "",
                "Relevant repository instructions and sources:",
                "\n".join(source_sections).rstrip() or "(no small text sources selected)",
            ]
        )
        if len(context) > _REPOSITORY_CONTEXT_LIMIT:
            marker = "\n... repository context truncated ...\n"
            context = context[: _REPOSITORY_CONTEXT_LIMIT - len(marker)] + marker
        return context

    def _attach_selected_skill(self, session: LocalAgentSession) -> None:
        if session.task_packet.get("selected_skill"):
            return
        invocation_name = self._task_packet_skill_invocation(session.task_packet)
        if not invocation_name:
            return
        catalog = CapabilityCatalogService(
            workspace_root=self.target_repo,
            agent_registry=self.agent_registry,
        ).inspect()
        skill = next(
            (
                item
                for item in catalog.skills
                if item.name.casefold() == invocation_name.casefold()
            ),
            None,
        )
        if skill is None:
            raise LaunchBlockedError(
                f"Selected skill {invocation_name!r} is not installed in the capability catalog."
            )
        try:
            instructions = _read_bounded_utf8(
                Path(skill.source),
                _SKILL_INSTRUCTION_LIMIT,
                probe_for_truncation=True,
            )
        except (OSError, UnicodeError) as exc:
            raise LaunchBlockedError(
                f"Selected skill {skill.name!r} instructions are unavailable: {exc}"
            ) from exc
        truncated = len(instructions) > _SKILL_INSTRUCTION_LIMIT
        instructions = instructions[:_SKILL_INSTRUCTION_LIMIT]
        if truncated:
            instructions += "\n... skill instructions truncated ...\n"
        session.task_packet["selected_skill"] = {
            "name": skill.name,
            "description": skill.description,
            "invocation": skill.invocation,
            "instructions": instructions,
        }

    @staticmethod
    def _task_packet_skill_invocation(task_packet: dict[str, Any]) -> str:
        values: list[str] = []
        for field_name in ("goal", "prompt"):
            value = task_packet.get(field_name)
            if isinstance(value, str):
                values.append(value)
        criteria = task_packet.get("acceptance_criteria", [])
        if isinstance(criteria, (list, tuple)):
            values.extend(item for item in criteria if isinstance(item, str))
        combined = "\n".join(values)
        invocation = re.search(
            r"^\s*/use\s+([^\s]+)",
            combined,
            flags=re.IGNORECASE | re.MULTILINE,
        )
        if invocation is None:
            return ""
        name = invocation.group(1).removeprefix("$").casefold()
        if not SKILL_NAME_PATTERN.fullmatch(name):
            raise LaunchBlockedError(
                f"Invalid selected skill invocation: {invocation.group(1)!r}."
            )
        return name

    @classmethod
    def _bounded_git_paths(
        cls,
        root: Path,
        arguments: list[str],
        *,
        description: str,
        required: bool = False,
    ) -> list[str] | None:
        try:
            completed = _run_bounded_process(
                ["git", "-C", str(root), *arguments],
                timeout_seconds=_GIT_SNAPSHOT_TIMEOUT_SECONDS,
                output_limit_bytes=_GIT_PATH_OUTPUT_BYTES_LIMIT,
            )
        except OSError as exc:
            raise AlbertError(f"unable to inspect {description}: {exc}") from exc
        if (
            completed.returncode == _PROCESS_OUTPUT_LIMIT_EXIT_STATUS
            and "output exceeded" in completed.stderr.lower()
        ):
            raise AlbertError(
                f"{description} exceeds the "
                f"{_GIT_PATH_OUTPUT_BYTES_LIMIT}-byte capture limit; "
                "narrow the workspace or exclude generated dependency trees before launching"
            )
        if completed.returncode != 0:
            reason = _bounded_process_output(
                completed.stderr or completed.stdout,
                limit_bytes=_GIT_COMMAND_OUTPUT_BYTES_LIMIT,
            ).strip() or f"Git exited {completed.returncode} without an explanation"
            if (
                not required
                and _is_explicit_git_not_repository(completed)
                and not _git_metadata_exists(root)
            ):
                return None
            if _is_explicit_git_not_repository(completed) and _git_metadata_exists(
                root
            ):
                reason = f"Git metadata exists but Git reported {reason}"
            raise AlbertError(
                f"unable to inspect {description}: "
                f"{reason}"
            )
        paths: set[str] = set()
        for raw_path in completed.stdout.split("\0"):
            if not raw_path:
                continue
            normalized = raw_path.replace("\\", "/")
            path = PurePosixPath(normalized)
            if (
                "\\" in raw_path
                or not path.parts
                or path.is_absolute()
                or ".." in path.parts
            ):
                raise AlbertError(
                    f"{description} returned an unsafe path: {raw_path!r}"
                )
            paths.add(normalized)
            if len(paths) > _GIT_PATH_COUNT_LIMIT:
                raise AlbertError(
                    f"{description} exceeds the {_GIT_PATH_COUNT_LIMIT}-path limit; "
                    "narrow the workspace or exclude generated dependency trees before launching"
                )
        return sorted(paths)

    @classmethod
    def _bounded_filesystem_files(
        cls,
        root: Path,
        *,
        description: str,
    ) -> list[str]:
        files: list[str] = []
        pending = [root]
        scanned = 0
        while pending:
            directory = pending.pop()
            try:
                entries = os.scandir(directory)
            except OSError as exc:
                raise AlbertError(f"unable to inspect {description}: {exc}") from exc
            with entries:
                for entry in entries:
                    scanned += 1
                    if scanned > _FILESYSTEM_SCAN_ENTRY_LIMIT:
                        raise AlbertError(
                            f"{description} exceeds the "
                            f"{_FILESYSTEM_SCAN_ENTRY_LIMIT}-entry filesystem scan limit; "
                            "narrow the workspace or exclude generated dependency trees before launching"
                        )
                    path = Path(entry.path)
                    try:
                        relative_path = path.relative_to(root).as_posix()
                    except ValueError as exc:
                        raise AlbertError(
                            f"{description} escaped its workspace boundary"
                        ) from exc
                    # POSIX permits arbitrary non-NUL bytes in names. Convert
                    # surrogate-escaped bytes to an explicit replacement marker
                    # so they remain visible and can never collapse into an
                    # authorized UTF-8 spelling.
                    relative_path = relative_path.encode(
                        "utf-8", errors="surrogateescape"
                    ).decode("utf-8", errors="replace")
                    if entry.is_symlink():
                        continue
                    if entry.is_dir(follow_symlinks=False):
                        if ".git" not in PurePosixPath(relative_path).parts:
                            pending.append(path)
                        continue
                    if entry.is_file(follow_symlinks=False):
                        files.append(relative_path)
                        if len(files) > _GIT_PATH_COUNT_LIMIT:
                            raise AlbertError(
                                f"{description} exceeds the "
                                f"{_GIT_PATH_COUNT_LIMIT}-path limit; narrow the workspace"
                            )
        return sorted(files)

    def _repository_files(self, worktree_path: Path) -> list[str]:
        if not _git_metadata_exists(worktree_path):
            return self._bounded_filesystem_files(
                worktree_path,
                description="repository file listing",
            )
        git_files = self._bounded_git_paths(
            worktree_path,
            [
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
            ],
            description="repository file listing",
        )
        if git_files is not None:
            return git_files
        return self._bounded_filesystem_files(
            worktree_path,
            description="repository file listing",
        )

    def _rank_repository_sources(
        self,
        session: LocalAgentSession,
        repository_files: list[str],
    ) -> list[str]:
        task_text = json.dumps(session.task_packet, sort_keys=True).lower()
        task_tokens = {
            token
            for token in re.findall(r"[a-zA-Z][a-zA-Z0-9_-]{2,}", task_text)
            if token not in {"acceptance", "criteria", "agent", "assigned", "command", "issue"}
        }
        allowed_paths = [
            path.replace("\\", "/").strip("/")
            for path in session.task_packet.get("allowed_paths", [])
            if isinstance(path, str) and not Path(path).is_absolute()
        ]
        repair_overlay = session.repository_snapshot.get("repair_overlay", {})
        repair_files = {
            path
            for path in repair_overlay.get("applied_files", [])
            if isinstance(path, str)
        } if isinstance(repair_overlay, dict) else set()

        def priority(relative_path: str) -> tuple[int, int, str]:
            name = PurePosixPath(relative_path).name
            suffix = PurePosixPath(relative_path).suffix.lower()
            if relative_path == "AGENTS.md":
                return (0, 0, relative_path)
            if relative_path == "CONTEXT.md":
                return (0, 1, relative_path)
            if relative_path in repair_files:
                return (0, 2, relative_path)
            in_allowed_path = any(
                relative_path == allowed or relative_path.startswith(f"{allowed}/")
                for allowed in allowed_paths
                if allowed
            )
            token_hits = sum(token in relative_path.lower() for token in task_tokens)
            if in_allowed_path:
                category = 1
            elif token_hits:
                category = 2
            elif name in _TEXT_SOURCE_NAMES or name in {
                "Cargo.toml",
                "package.json",
                "pyproject.toml",
            }:
                category = 3
            elif suffix not in {".md", ".txt"}:
                category = 4
            else:
                category = 5
            return (category, -token_hits, relative_path)

        eligible = [
            relative_path
            for relative_path in repository_files
            if self._is_text_source(relative_path)
            and self.classify_file_for_frontier(relative_path) == "Normal"
        ]
        return sorted(eligible, key=priority)

    @staticmethod
    def _is_text_source(relative_path: str) -> bool:
        path = PurePosixPath(relative_path)
        if any(part in {".git", "node_modules", "target", "vendor"} for part in path.parts):
            return False
        return path.suffix.lower() in _TEXT_SOURCE_SUFFIXES or path.name in _TEXT_SOURCE_NAMES

    def _read_repository_source(self, worktree_path: Path, relative_path: str) -> str | None:
        try:
            path = self._safe_worktree_destination(worktree_path, relative_path)
        except AlbertError:
            return None
        if path.is_symlink() or not path.is_file():
            return None
        try:
            payload = _read_bounded_bytes(path, _REPOSITORY_SOURCE_LIMIT)
        except OSError:
            return None
        if b"\0" in payload:
            return None
        content = payload[:_REPOSITORY_SOURCE_LIMIT].decode("utf-8", errors="replace")
        if len(payload) > _REPOSITORY_SOURCE_LIMIT:
            content += "\n... source truncated ...\n"
        return content

    def _run_fake_agent(self, session: LocalAgentSession) -> None:
        self._raise_if_cancelled(session)
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        task_packet_path = artifact_dir / "task-packet.json"
        log_path = artifact_dir / "fake-agent.log"
        completion_path = artifact_dir / "completion.json"
        result_path = session.worktree_path / "FAKE_AGENT_RESULT.md"
        self._raise_if_cancelled(session)
        self._write(task_packet_path, json.dumps(session.task_packet, indent=2, sort_keys=True) + "\n")
        self._raise_if_cancelled(session)
        self._write(
            log_path,
            "\n".join(
                [
                    f"Fake Local Agent: {session.assigned_agent}",
                    f"Issue Slice: {session.issue_id}",
                    "Result: deterministic completion",
                    "",
                ]
            ),
        )
        self._raise_if_cancelled(session)
        self._write(
            completion_path,
            json.dumps(
                {
                    "session_id": session.session_id,
                    "issue_id": session.issue_id,
                    "status": "completed",
                    "changed_files": ["FAKE_AGENT_RESULT.md"],
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
        )
        self._raise_if_cancelled(session)
        self._write(
            result_path,
            "\n".join(
                [
                    f"# Fake Agent Result for {session.issue_id}",
                    "",
                    "This deterministic artifact proves the local runner path executed.",
                    "",
                ]
            ),
        )
        session.artifacts = {
            "task_packet": str(task_packet_path),
            "fake_log": str(log_path),
            "completion": str(completion_path),
        }
        session.evidence = EvidencePackage(
            changed_files=["FAKE_AGENT_RESULT.md"],
            diff_summary=f"Deterministic fake completion for {session.issue_id}.",
            commands_run=[f"fake-agent {session.assigned_agent}"],
            test_results="Not run: deterministic fake runner.",
            known_risks="Fake runner does not perform real code edits.",
            proposed_context_updates="None.",
            artifact_links=self._automated_review_artifact_links(session),
        )
        session.evidence_valid = True
        session.evidence_correlation_id = (
            f"evidence:{self.mission_id}:{session.session_id}"
        )
        session.status = "evidence-ready"
        self._record(f"{session.issue_id} fake runner produced evidence for {session.session_id}.")

    def _run_command_agent(self, session: LocalAgentSession, agent_config: AgentConfig) -> None:
        command = agent_config.command
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        task_packet_path = artifact_dir / "task-packet.json"
        stdout_path = artifact_dir / "stdout.log"
        stderr_path = artifact_dir / "stderr.log"
        result_path = artifact_dir / "runner-result.json"
        self._write(task_packet_path, json.dumps(session.task_packet, indent=2, sort_keys=True) + "\n")
        session.runner_started_at = _utc_now()
        env = os.environ.copy()
        env["ALBERT_TASK_PACKET"] = str(task_packet_path)
        env["ALBERT_SESSION_ID"] = session.session_id
        try:
            completed = self._run_cancellable_process(
                session,
                _command_invocation(command),
                env=env,
            )
            self._raise_if_cancelled(session)
            exit_status = completed.returncode
            stdout = completed.stdout
            stderr = completed.stderr
        except FileNotFoundError as exc:
            exit_status = 127
            stdout = ""
            stderr = f"Unable to start {command!r}: {exc}"
        session.runner_ended_at = _utc_now()
        self._write(stdout_path, stdout)
        self._write(stderr_path, stderr)
        self._write(
            result_path,
            json.dumps(
                {
                    "session_id": session.session_id,
                    "issue_id": session.issue_id,
                    "command": command,
                    "exit_status": exit_status,
                    "started_at": session.runner_started_at,
                    "ended_at": session.runner_ended_at,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
        )
        session.runner_exit_status = exit_status
        session.status = "completed" if exit_status == 0 else "failed"
        session.artifacts = {
            "task_packet": str(task_packet_path),
            "stdout": str(stdout_path),
            "stderr": str(stderr_path),
            "result": str(result_path),
        }
        self._collect_automated_evidence(session, agent_config)
        self._record(f"{session.issue_id} command runner exited {exit_status} for {session.session_id}.")

    def _run_ollama_agent(self, session: LocalAgentSession, agent_config: AgentConfig) -> None:
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        task_packet_path = artifact_dir / "task-packet.json"
        result_path = artifact_dir / "ollama-result.json"
        command = self._runner_command(agent_config)
        self._write(task_packet_path, json.dumps(session.task_packet, indent=2, sort_keys=True) + "\n")
        session.runner_started_at = _utc_now()
        session.artifacts = {
            "task_packet": str(task_packet_path),
            "result": str(result_path),
        }
        known_risk = ""
        planned_command_results: list[dict[str, Any]] = []
        round_results: list[dict[str, Any]] = []
        overall_exit_status = 1
        model_ended_at = ""
        feedback = ""
        recovered_iterations = 0
        for iteration in range(1, _MODEL_AGENT_ITERATION_LIMIT + 1):
            prompt = self._ollama_prompt(session, agent_config)
            if feedback:
                prompt += (
                    "\nIteration feedback from the governed command runner:\n"
                    f"{feedback[:_MODEL_FEEDBACK_LIMIT]}\n"
                    "Inspect the current repository state above, repair the failure, and return "
                    "a new JSON file/command plan.\n"
                )
            suffix = "" if iteration == 1 else f"-round-{iteration:02d}"
            prompt_path = artifact_dir / f"ollama-prompt{suffix}.txt"
            output_path = artifact_dir / f"ollama-output{suffix}.txt"
            stderr_path = artifact_dir / f"ollama-stderr{suffix}.log"
            self._write(prompt_path, prompt)
            try:
                completed = self._run_cancellable_process(
                    session,
                    _command_invocation(command),
                    input_text=prompt,
                    output_limit_bytes=_MODEL_PROCESS_OUTPUT_BYTES_LIMIT,
                )
                self._raise_if_cancelled(session)
                exit_status = completed.returncode
                stdout = completed.stdout
                stderr = completed.stderr
            except FileNotFoundError as exc:
                exit_status = 127
                stdout = ""
                stderr = f"Unable to start {command!r}: {exc}"
            model_ended_at = _utc_now()
            self._write(output_path, stdout)
            self._write(stderr_path, stderr)
            round_result: dict[str, Any] = {
                "iteration": iteration,
                "model_exit_status": exit_status,
                "prompt": str(prompt_path),
                "output": str(output_path),
                "stderr": str(stderr_path),
            }
            round_results.append(round_result)
            round_key = f"ollama_round_{iteration:02d}"
            session.artifacts.update(
                {
                    f"{round_key}_prompt": str(prompt_path),
                    f"{round_key}_output": str(output_path),
                    f"{round_key}_stderr": str(stderr_path),
                }
            )
            if iteration == 1:
                session.artifacts.update(
                    {
                        "ollama_prompt": str(prompt_path),
                        "ollama_output": str(output_path),
                        "stderr": str(stderr_path),
                    }
                )
            if exit_status != 0:
                session.status = "failed"
                overall_exit_status = exit_status or 1
                known_risk = (
                    f"Ollama command exited {exit_status}; inspect stderr artifact."
                )
                break
            try:
                plan = _parse_model_file_plan(stdout)
            except AlbertError as exc:
                session.status = "failed"
                overall_exit_status = 1
                known_risk = f"Malformed Ollama output: {exc}"
                break
            command_specs, rejected_command = self._preflight_model_commands(
                session,
                plan["commands"],
                index_offset=len(planned_command_results),
                iteration=iteration,
            )
            if rejected_command is not None:
                planned_command_results.append(rejected_command)
                session.status = "failed"
                overall_exit_status = 1
                known_risk = (
                    f"Ollama plan rejected: {rejected_command['failure_reason']}"
                )
                break
            try:
                for file_plan in plan["files"]:
                    self._write_model_file(
                        session,
                        file_plan["path"],
                        file_plan["content"],
                    )
            except AlbertError as exc:
                session.status = "failed"
                overall_exit_status = 1
                known_risk = f"Ollama plan rejected: {exc}"
                break
            iteration_results = self._execute_model_commands(
                session,
                command_specs,
                iteration=iteration,
            )
            planned_command_results.extend(iteration_results)
            failed_command = next(
                (
                    result
                    for result in iteration_results
                    if result["outcome"] != "passed"
                ),
                None,
            )
            if failed_command is None:
                session.status = "running"
                overall_exit_status = 0
                if recovered_iterations:
                    known_risk = (
                        f"Recovered after {recovered_iterations} failed agent iteration(s)."
                    )
                break
            session.status = "failed"
            overall_exit_status = failed_command["exit_status"] or 1
            known_risk = failed_command["failure_reason"]
            if iteration >= _MODEL_AGENT_ITERATION_LIMIT:
                break
            recovered_iterations += 1
            feedback = self._model_command_failure_feedback(
                session,
                failed_command,
            )
            session.status = "running"
        session.runner_ended_at = _utc_now()
        self._write(
            result_path,
            json.dumps(
                {
                    "session_id": session.session_id,
                    "issue_id": session.issue_id,
                    "command": command,
                    "model": agent_config.model,
                    "model_exit_status": round_results[-1]["model_exit_status"],
                    "exit_status": overall_exit_status,
                    "iterations": round_results,
                    "planned_commands": planned_command_results,
                    "started_at": session.runner_started_at,
                    "model_ended_at": model_ended_at,
                    "ended_at": session.runner_ended_at,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
        )
        session.runner_exit_status = overall_exit_status
        self._collect_automated_evidence(
            session,
            agent_config,
            known_risk_override=known_risk,
            planned_command_results=planned_command_results,
        )
        self._record(
            f"{session.issue_id} ollama runner exited {overall_exit_status} "
            f"for {session.session_id} after {len(round_results)} iteration(s)."
        )

    def _model_command_failure_feedback(
        self,
        session: LocalAgentSession,
        result: dict[str, Any],
    ) -> str:
        index = int(result["index"])
        artifact_key = f"planned_command_{index:02d}"
        stdout_path = session.artifacts.get(f"{artifact_key}_stdout", "")
        stderr_path = session.artifacts.get(f"{artifact_key}_stderr", "")

        def read_bounded(path_value: str) -> str:
            if not path_value:
                return ""
            try:
                return _read_bounded_utf8(
                    Path(path_value),
                    _MODEL_FEEDBACK_LIMIT,
                )
            except (OSError, UnicodeError):
                return ""

        return "\n".join(
            [
                result["failure_reason"],
                f"Command: {result['command']}",
                "stdout:",
                read_bounded(stdout_path),
                "stderr:",
                read_bounded(stderr_path),
            ]
        )[:_MODEL_FEEDBACK_LIMIT]

    def _preflight_model_commands(
        self,
        session: LocalAgentSession,
        commands: list[str],
        *,
        index_offset: int = 0,
        iteration: int = 1,
    ) -> tuple[list[tuple[int, str, list[str]]], dict[str, Any] | None]:
        specs: list[tuple[int, str, list[str]]] = []
        for index, command in enumerate(commands, start=index_offset + 1):
            policy = self.classify_command(command)
            try:
                argv = shlex.split(command)
            except ValueError as exc:
                reason = f"Planned command {index} has invalid argv syntax: {exc}"
                return [], self._record_model_command_artifacts(
                    session,
                    index=index,
                    command=command,
                    argv=[],
                    policy=policy,
                    executed=False,
                    outcome="rejected",
                    exit_status=None,
                    stdout="",
                    stderr=reason,
                    started_at="",
                    ended_at=_utc_now(),
                    failure_reason=reason,
                    iteration=iteration,
                )
            if not argv:
                reason = f"Planned command {index} has no argv tokens."
                return [], self._record_model_command_artifacts(
                    session,
                    index=index,
                    command=command,
                    argv=[],
                    policy=policy,
                    executed=False,
                    outcome="rejected",
                    exit_status=None,
                    stdout="",
                    stderr=reason,
                    started_at="",
                    ended_at=_utc_now(),
                    failure_reason=reason,
                    iteration=iteration,
                )
            if policy != "auto-allowed":
                reason = (
                    f"Planned command {index} policy is {policy}; "
                    "auto-allowed is required."
                )
                return [], self._record_model_command_artifacts(
                    session,
                    index=index,
                    command=command,
                    argv=argv,
                    policy=policy,
                    executed=False,
                    outcome="rejected",
                    exit_status=None,
                    stdout="",
                    stderr=reason,
                    started_at="",
                    ended_at=_utc_now(),
                    failure_reason=reason,
                    iteration=iteration,
                )
            specs.append((index, command, argv))
        return specs, None

    def _execute_model_commands(
        self,
        session: LocalAgentSession,
        specs: list[tuple[int, str, list[str]]],
        *,
        iteration: int = 1,
    ) -> list[dict[str, Any]]:
        results: list[dict[str, Any]] = []
        for index, command, argv in specs:
            started_at = _utc_now()
            outcome = "passed"
            failure_reason = ""
            try:
                completed = self._run_cancellable_process(
                    session,
                    argv,
                    timeout_seconds=_MODEL_COMMAND_TIMEOUT_SECONDS,
                )
                self._raise_if_cancelled(session)
                exit_status = completed.returncode
                stdout = completed.stdout
                stderr = completed.stderr
                if exit_status == 124:
                    outcome = "timed-out"
                    failure_reason = (
                        "Planned command timed out after "
                        f"{_MODEL_COMMAND_TIMEOUT_SECONDS} seconds: {command}"
                    )
                elif exit_status != 0:
                    outcome = "failed"
                    failure_reason = (
                        f"Planned command failed (exit {exit_status}): {command}"
                    )
            except OSError as exc:
                exit_status = 127
                outcome = "start-failed"
                stdout = ""
                failure_reason = f"Unable to start planned command {command!r}: {exc}"
                stderr = failure_reason
            result = self._record_model_command_artifacts(
                session,
                index=index,
                command=command,
                argv=argv,
                policy="auto-allowed",
                executed=True,
                outcome=outcome,
                exit_status=exit_status,
                stdout=stdout,
                stderr=stderr,
                started_at=started_at,
                ended_at=_utc_now(),
                failure_reason=failure_reason,
                iteration=iteration,
            )
            results.append(result)
            if outcome != "passed":
                break
        return results

    def _record_model_command_artifacts(
        self,
        session: LocalAgentSession,
        *,
        index: int,
        command: str,
        argv: list[str],
        policy: str,
        executed: bool,
        outcome: str,
        exit_status: int | None,
        stdout: str,
        stderr: str,
        started_at: str,
        ended_at: str,
        failure_reason: str,
        iteration: int = 1,
    ) -> dict[str, Any]:
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        prefix = f"planned-command-{index:02d}"
        stdout_path = artifact_dir / f"{prefix}-stdout.log"
        stderr_path = artifact_dir / f"{prefix}-stderr.log"
        result_path = artifact_dir / f"{prefix}-result.json"
        result: dict[str, Any] = {
            "index": index,
            "iteration": iteration,
            "command": command,
            "argv": argv,
            "policy": policy,
            "shell": False,
            "executed": executed,
            "outcome": outcome,
            "exit_status": exit_status,
            "timeout_seconds": _MODEL_COMMAND_TIMEOUT_SECONDS,
            "started_at": started_at,
            "ended_at": ended_at,
            "failure_reason": failure_reason,
        }
        self._write(stdout_path, stdout)
        self._write(stderr_path, stderr)
        self._write(result_path, json.dumps(result, indent=2, sort_keys=True) + "\n")
        artifact_key = f"planned_command_{index:02d}"
        session.artifacts.update(
            {
                f"{artifact_key}_stdout": str(stdout_path),
                f"{artifact_key}_stderr": str(stderr_path),
                f"{artifact_key}_result": str(result_path),
            }
        )
        return result

    @staticmethod
    def _subprocess_output_text(value: str | bytes | None) -> str:
        if value is None:
            return ""
        if isinstance(value, bytes):
            return value.decode("utf-8", errors="replace")
        return value

    def _run_cancellable_process(
        self,
        session: LocalAgentSession,
        argv: str | list[str],
        *,
        input_text: str | None = None,
        env: dict[str, str] | None = None,
        timeout_seconds: float = _RUNNER_COMMAND_TIMEOUT_SECONDS,
        output_limit_bytes: int = _PROCESS_OUTPUT_BYTES_LIMIT,
    ) -> subprocess.CompletedProcess[str]:
        self._raise_if_cancelled(session)
        process_env = sanitized_process_environment(env)
        if isinstance(argv, list) and argv and "/" not in argv[0]:
            if shutil.which(argv[0], path=process_env.get("PATH")) is None:
                return subprocess.CompletedProcess(
                    argv,
                    127,
                    "",
                    f"Unable to start {argv[0]!r}: command not found in governed PATH.",
                )
        readable_roots = tuple(
            Path(value)
            for name, value in (env or {}).items()
            if name == "ALBERT_TASK_PACKET" and value and Path(value).exists()
        )
        dependency_bindings: list[tuple[Path, Path]] = []
        dependency_parents = [self.target_repo]
        try:
            entries = os.scandir(self.target_repo)
        except OSError:
            entries = None
        if entries is not None:
            with entries:
                for _ in range(_DEPENDENCY_PARENT_SCAN_LIMIT):
                    try:
                        entry = next(entries)
                    except StopIteration:
                        break
                    try:
                        if (
                            not entry.name.startswith(".")
                            and not entry.is_symlink()
                            and entry.is_dir(follow_symlinks=False)
                        ):
                            dependency_parents.append(Path(entry.path))
                    except OSError:
                        continue
        for parent in dependency_parents:
            for dependency_name in ("node_modules", ".venv", "venv"):
                source = parent / dependency_name
                if source.is_symlink() or not source.is_dir():
                    continue
                try:
                    relative = source.relative_to(self.target_repo)
                    resolved_source = source.resolve()
                except ValueError:
                    continue
                if not resolved_source.is_relative_to(self.target_repo):
                    continue
                destination = session.worktree_path / relative
                if not destination.exists():
                    dependency_bindings.append((resolved_source, destination))
        governed_argv, sandboxed = sandboxed_process_argv(
            argv,
            working_directory=session.worktree_path,
            readable_roots=readable_roots,
            writable_roots=(session.worktree_path,),
            readonly_bindings=tuple(dependency_bindings),
        )
        if os.name == "posix" and isinstance(argv, list) and not sandboxed:
            return subprocess.CompletedProcess(
                argv,
                126,
                "",
                "Unable to start governed process: bubblewrap (bwrap) is required "
                "for the writable-worktree filesystem boundary.",
            )
        governed_session = session.runner_pid is not None
        process_started = False

        def record_process_start(
            process: subprocess.Popen[bytes],
            process_token: str,
        ) -> None:
            nonlocal process_started
            process_started = True
            session.runner_process_pid = process.pid
            session.runner_process_identity = _process_identity(process.pid)
            session.runner_process_token = process_token
            if governed_session:
                persisted = self._persist_session_update(session)
                if persisted.status == "cancelled":
                    raise SessionCancelledError(
                        f"{session.session_id} cancelled before runner process startup"
                    )

        try:
            return _run_bounded_process(
                governed_argv,
                input_text=input_text,
                cwd=session.worktree_path,
                env=process_env,
                timeout_seconds=timeout_seconds,
                output_limit_bytes=output_limit_bytes,
                process_binding_started=record_process_start,
                poll_callback=lambda: self._raise_if_cancelled(session),
                output_callback=lambda stream_name, payload: self._record_session_output(
                    session.session_id,
                    stream_name,
                    payload,
                ),
            )
        finally:
            # Retain the last exact process-group binding until the canonical
            # runner transition clears it. A late result or owner-loss probe
            # needs that durable identity to prove quiescence independently.
            if process_started and governed_session:
                persisted = self._persist_session_update(session)
                if persisted.status == "cancelled":
                    session.status = "cancelled"

    def _ollama_prompt(self, session: LocalAgentSession, agent_config: AgentConfig) -> str:
        selected_skill = session.task_packet.get("selected_skill")
        skill_instructions = (
            str(selected_skill.get("instructions", ""))
            if isinstance(selected_skill, dict)
            else ""
        )
        return "\n".join(
            [
                "You are Albert's local coding agent.",
                f"Model: {agent_config.model}",
                "Return only JSON with this schema:",
                '{"summary": "short text", "files": [{"path": "relative/path", "content": "file contents"}], "commands": ["optional test or verification command"]}',
                "Do not include markdown fences or commentary.",
                "Create or replace only files needed for the task. Keep every path relative to the session worktree and inside allowed_paths when they are present.",
                "Commands are optional. Request only focused test, lint, or build commands. Albert runs them as argv without a shell, only when its command policy classifies them as auto-allowed, and stops after the first failure or timeout.",
                f"You have up to {_MODEL_AGENT_ITERATION_LIMIT} governed iterations. When command feedback is present, diagnose it against the refreshed repository context and return a corrective plan.",
                "Skill instructions are guidance only. Never execute a skill-referenced script unless its exact command is separately returned in commands and passes Albert's normal command policy.",
                "",
                "Task packet:",
                json.dumps(session.task_packet, indent=2, sort_keys=True),
                "",
                "Selected skill instructions (bounded, catalog-resolved):",
                skill_instructions or "(none)",
                "",
                self._repository_context(session),
                "",
            ]
        )

    def _write_model_file(
        self,
        session: LocalAgentSession,
        relative_path: str,
        content: str,
    ) -> None:
        self._raise_if_cancelled(session)
        destination = self._safe_worktree_destination(
            session.worktree_path,
            relative_path,
        )
        if not self._model_path_is_allowed(session, destination):
            raise AlbertError(f"file path is outside allowed_paths: {relative_path!r}")
        self._write(destination, content)

    @staticmethod
    def _safe_worktree_destination(worktree_path: Path, relative_path: str) -> Path:
        if not isinstance(relative_path, str) or not relative_path.strip() or "\0" in relative_path:
            raise AlbertError(f"unsafe file path {relative_path!r}")
        normalized = relative_path.replace("\\", "/")
        if normalized.startswith("/") or normalized.startswith("//") or re.match(
            r"^[A-Za-z]:", normalized
        ):
            raise AlbertError(f"unsafe file path {relative_path!r}")
        path = PurePosixPath(normalized)
        if path.is_absolute() or ".." in path.parts or not path.parts or path.parts[0] == ".git":
            raise AlbertError(f"unsafe file path {relative_path!r}")
        root = worktree_path.resolve()
        destination = (root / Path(*path.parts)).resolve(strict=False)
        try:
            destination.relative_to(root)
        except ValueError as exc:
            raise AlbertError(f"unsafe file path {relative_path!r}") from exc
        if destination == root:
            raise AlbertError(f"unsafe file path {relative_path!r}")
        return destination

    def _model_path_is_allowed(
        self,
        session: LocalAgentSession,
        destination: Path,
    ) -> bool:
        if "\ufffd" in destination.as_posix():
            return False
        allowed_paths = session.task_packet.get("allowed_paths", [])
        if not allowed_paths:
            return True
        relative_destination = destination.relative_to(session.worktree_path.resolve())
        for allowed_path in allowed_paths:
            if not isinstance(allowed_path, str) or not allowed_path.strip():
                continue
            if allowed_path.replace("\\", "/").strip() in {".", "./"}:
                return True
            raw_allowed = Path(allowed_path)
            if raw_allowed.is_absolute():
                try:
                    relative_allowed = raw_allowed.resolve().relative_to(self.target_repo)
                except ValueError:
                    continue
            else:
                try:
                    relative_allowed = self._safe_worktree_destination(
                        session.worktree_path,
                        allowed_path,
                    ).relative_to(session.worktree_path.resolve())
                except AlbertError:
                    continue
            if relative_allowed == Path("."):
                return True
            if relative_destination == relative_allowed:
                return True
            try:
                relative_destination.relative_to(relative_allowed)
                return True
            except ValueError:
                continue
        return False

    def _collect_automated_evidence(
        self,
        session: LocalAgentSession,
        agent_config: AgentConfig,
        *,
        known_risk_override: str = "",
        planned_command_results: list[dict[str, Any]] | None = None,
    ) -> None:
        changed_files = self._worktree_changed_files(
            session.worktree_path,
            session.baseline_fingerprints,
        )
        unauthorized_changes = [
            relative_path
            for relative_path in changed_files
            if not self._model_path_is_allowed(
                session,
                self._safe_worktree_destination(
                    session.worktree_path,
                    relative_path,
                ),
            )
        ]
        if changed_files:
            diff_path, additions, deletions = self._write_review_diff_artifact(
                session,
                changed_files,
            )
            file_summary = ", ".join(changed_files[:5])
            if len(changed_files) > 5:
                file_summary += f", and {len(changed_files) - 5} more"
            diff_summary = (
                f"Reviewable patch for {len(changed_files)} file(s): "
                f"+{additions}/-{deletions} ({file_summary}). Artifact: {diff_path}"
            )
        else:
            diff_summary = "No agent-authored worktree file changes detected."
        planned_command_results = planned_command_results or []
        final_planned_command_failed = bool(
            planned_command_results
            and planned_command_results[-1]["outcome"] != "passed"
        )
        configured_test_results = self._collect_test_results(session, agent_config)
        planned_test_results = self._planned_command_test_results(planned_command_results)
        test_results = "\n".join(
            result
            for result in (planned_test_results, configured_test_results)
            if result
        )
        known_risks = known_risk_override or "None."
        requires_edit = self._session_requires_edit(session)
        if unauthorized_changes:
            session.status = "failed"
            known_risks = (
                "Runner changed files outside allowed_paths: "
                + ", ".join(unauthorized_changes[:10])
            )
        if requires_edit and not changed_files:
            session.status = "failed"
            if not known_risk_override:
                known_risks = (
                    "Edit-requiring work produced no agent-authored file changes."
                )
        if not known_risk_override and session.runner_exit_status and session.runner_exit_status != 0:
            known_risks = f"Runner exited {session.runner_exit_status}; inspect stderr artifact."
        elif not known_risk_override and final_planned_command_failed:
            known_risks = "Test command failed; inspect test stderr artifact."
        commands_run = [self._runner_command(agent_config)]
        commands_run.extend(
            result["command"]
            for result in planned_command_results
            if result["executed"]
        )
        session.evidence = EvidencePackage(
            changed_files=changed_files,
            diff_summary=diff_summary,
            commands_run=commands_run,
            test_results=test_results,
            known_risks=known_risks,
            proposed_context_updates="None.",
            artifact_links=self._automated_review_artifact_links(session),
        )
        session.evidence_valid = bool(
            not session.evidence.missing_fields() and not unauthorized_changes
        )
        session.evidence_correlation_id = (
            f"evidence:{self.mission_id}:{session.session_id}"
            if session.evidence_valid
            else ""
        )
        if (
            session.status != "failed"
            and session.runner_exit_status == 0
            and not final_planned_command_failed
            and "test command failed" not in configured_test_results.lower()
        ):
            session.status = "evidence-ready"

    def _write_review_diff_artifact(
        self,
        session: LocalAgentSession,
        changed_files: list[str],
    ) -> tuple[Path, int, int]:
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        diff_path = artifact_dir / "review.diff"
        chunks: list[str] = []
        additions = 0
        deletions = 0
        size = 0
        for relative_path in changed_files:
            visibility = self.classify_file_for_frontier(relative_path)
            if visibility != "Normal":
                chunk = (
                    f"--- a/{relative_path}\n"
                    f"+++ b/{relative_path}\n"
                    "@@ content redacted @@\n"
                    f"[{visibility} file content omitted from review artifact]\n"
                )
                encoded_size = len(chunk.encode("utf-8"))
                if size + encoded_size > _REVIEW_DIFF_BYTES_LIMIT:
                    chunks.append(
                        "\n... review diff truncated at "
                        f"{_REVIEW_DIFF_BYTES_LIMIT} bytes ...\n"
                    )
                    break
                chunks.append(chunk)
                size += encoded_size
                continue
            try:
                result_path = self._safe_worktree_destination(
                    session.worktree_path,
                    relative_path,
                )
            except AlbertError:
                continue
            before, baseline_size = self._review_baseline_text(
                session,
                relative_path,
            )
            after = self._bounded_diff_text(result_path)
            if before is None or after is None:
                chunk = (
                    f"Binary or oversized change: {relative_path} "
                    f"({baseline_size} -> {self._file_size(result_path)} bytes)\n"
                )
            else:
                lines = list(
                    difflib.unified_diff(
                        before.splitlines(keepends=True),
                        after.splitlines(keepends=True),
                        fromfile=f"a/{relative_path}",
                        tofile=f"b/{relative_path}",
                    )
                )
                additions += sum(
                    1
                    for line in lines
                    if line.startswith("+") and not line.startswith("+++")
                )
                deletions += sum(
                    1
                    for line in lines
                    if line.startswith("-") and not line.startswith("---")
                )
                chunk = "".join(lines)
            encoded_size = len(chunk.encode("utf-8", errors="replace"))
            if size + encoded_size > _REVIEW_DIFF_BYTES_LIMIT:
                chunks.append(
                    "\n... review diff truncated at "
                    f"{_REVIEW_DIFF_BYTES_LIMIT} bytes ...\n"
                )
                break
            chunks.append(chunk)
            size += encoded_size
        self._write(diff_path, "".join(chunks) or "No textual diff available.\n")
        session.artifacts["review_diff"] = str(diff_path)
        return diff_path, additions, deletions

    def _review_baseline_text(
        self,
        session: LocalAgentSession,
        relative_path: str,
    ) -> tuple[str | None, int]:
        baseline = session.repository_snapshot.get("review_baseline", {})
        entries = baseline.get("entries", {}) if isinstance(baseline, dict) else {}
        entry = entries.get(relative_path) if isinstance(entries, dict) else None
        if isinstance(entry, dict):
            state = entry.get("state")
            size = entry.get("size")
            baseline_size = size if isinstance(size, int) and size >= 0 else 0
            if state == "missing":
                return "", 0
            if state != "file":
                return None, baseline_size
            root_value = baseline.get("root")
            snapshot_name = entry.get("snapshot")
            if not isinstance(root_value, str) or not isinstance(snapshot_name, str):
                return None, baseline_size
            expected_root = (
                self.runtime_dir
                / "sessions"
                / session.session_id
                / "review-baseline"
            ).absolute()
            baseline_root = Path(root_value).absolute()
            if (
                _runtime_identity_path(baseline_root)
                != _runtime_identity_path(expected_root)
                or not re.fullmatch(r"[0-9a-f]{64}", snapshot_name)
            ):
                return None, baseline_size
            baseline_root = baseline_root.resolve()
            snapshot_path = (baseline_root / snapshot_name).resolve()
            if snapshot_path.parent != baseline_root:
                return None, baseline_size
            return self._bounded_diff_text(snapshot_path), baseline_size

        if relative_path in session.baseline_fingerprints:
            return None, 0
        if session.repository_snapshot.get("kind") != "git-worktree":
            return "", 0
        path = PurePosixPath(relative_path.replace("\\", "/"))
        if not path.parts or path.is_absolute() or ".." in path.parts:
            return None, 0
        object_name = f"HEAD:{path.as_posix()}"
        try:
            size_result = _run_bounded_process(
                [
                    "git",
                    "-C",
                    str(session.worktree_path),
                    "cat-file",
                    "-s",
                    object_name,
                ],
                timeout_seconds=_GIT_SNAPSHOT_TIMEOUT_SECONDS,
                output_limit_bytes=4_096,
            )
        except OSError:
            return None, 0
        if size_result.returncode != 0:
            return "", 0
        try:
            baseline_size = int(size_result.stdout.strip())
        except ValueError:
            return None, 0
        if baseline_size > _REVIEW_DIFF_BYTES_LIMIT:
            return None, baseline_size
        try:
            content_result = _run_bounded_process(
                [
                    "git",
                    "-C",
                    str(session.worktree_path),
                    "show",
                    object_name,
                ],
                timeout_seconds=_GIT_SNAPSHOT_TIMEOUT_SECONDS,
                output_limit_bytes=(
                    _REVIEW_DIFF_BYTES_LIMIT
                    + _PROCESS_OUTPUT_MESSAGE_RESERVE
                    + 1
                ),
            )
        except OSError:
            return None, baseline_size
        if content_result.returncode != 0 or "\0" in content_result.stdout:
            return None, baseline_size
        return content_result.stdout, baseline_size

    @staticmethod
    def _bounded_diff_text(path: Path) -> str | None:
        if not path.exists():
            return ""
        if path.is_symlink() or not path.is_file():
            return None
        try:
            payload = _read_bounded_bytes(path, _REVIEW_DIFF_BYTES_LIMIT)
        except OSError:
            return None
        if len(payload) > _REVIEW_DIFF_BYTES_LIMIT or b"\0" in payload:
            return None
        return payload.decode("utf-8", errors="replace")

    @staticmethod
    def _file_size(path: Path) -> int:
        try:
            return path.stat().st_size if path.exists() else 0
        except OSError:
            return 0

    @staticmethod
    def _session_requires_edit(session: LocalAgentSession) -> bool:
        explicit = session.task_packet.get("requires_edit")
        if isinstance(explicit, bool):
            return explicit
        return session.task_packet.get("work_kind") != "headless-review"

    @staticmethod
    def _planned_command_test_results(results: list[dict[str, Any]]) -> str:
        lines: list[str] = []
        for result in results:
            command = result["command"]
            outcome = result["outcome"]
            if outcome == "passed":
                lines.append(f"Planned command passed: {command}")
            elif outcome == "rejected":
                lines.append(
                    f"Planned command rejected ({result['policy']}): {command}"
                )
            elif outcome == "timed-out":
                lines.append(
                    "Planned command timed out after "
                    f"{result['timeout_seconds']} seconds: {command}"
                )
            elif outcome == "start-failed":
                lines.append(f"Planned command could not start: {command}")
            else:
                lines.append(
                    f"Planned command failed (exit {result['exit_status']}): {command}"
                )
        return "\n".join(lines)

    def _collect_test_results(self, session: LocalAgentSession, agent_config: AgentConfig) -> str:
        if not agent_config.test_command:
            return "Not applicable: no test command configured."
        policy = self.classify_command(agent_config.test_command)
        if policy != "auto-allowed":
            return f"Not applicable: test command policy is {policy}."
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        stdout_path = artifact_dir / "test-stdout.log"
        stderr_path = artifact_dir / "test-stderr.log"
        result_path = artifact_dir / "test-result.json"
        env = os.environ.copy()
        env["ALBERT_TASK_PACKET"] = session.artifacts.get("task_packet", "")
        env["ALBERT_SESSION_ID"] = session.session_id
        try:
            completed = self._run_cancellable_process(
                session,
                _command_invocation(agent_config.test_command),
                env=env,
            )
            self._raise_if_cancelled(session)
            exit_status = completed.returncode
            stdout = completed.stdout
            stderr = completed.stderr
        except FileNotFoundError as exc:
            exit_status = 127
            stdout = ""
            stderr = f"Unable to start {agent_config.test_command!r}: {exc}"
        self._write(stdout_path, stdout)
        self._write(stderr_path, stderr)
        self._write(
            result_path,
            json.dumps(
                {
                    "command": agent_config.test_command,
                    "exit_status": exit_status,
                    "session_id": session.session_id,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
        )
        session.artifacts.update(
            {
                "test_stdout": str(stdout_path),
                "test_stderr": str(stderr_path),
                "test_result": str(result_path),
            }
        )
        if exit_status == 0:
            return f"Test command passed: {agent_config.test_command}"
        session.status = "failed"
        return f"Test command failed (exit {exit_status}): {agent_config.test_command}"

    @classmethod
    def _worktree_changed_files(
        cls,
        worktree_path: Path,
        baseline_fingerprints: dict[str, str] | None = None,
    ) -> list[str]:
        if not worktree_path.exists():
            return []
        if _git_metadata_exists(worktree_path):
            changed = cls._bounded_git_paths(
                worktree_path,
                ["diff", "--name-only", "-z", "HEAD", "--"],
                description="worktree changed-file listing",
            )
            untracked = cls._bounded_git_paths(
                worktree_path,
                ["ls-files", "--others", "--exclude-standard", "-z", "--"],
                description="worktree untracked-file listing",
            )
            assert changed is not None and untracked is not None
            paths = set(changed) | set(untracked)
        else:
            paths = set(
                cls._bounded_filesystem_files(
                    worktree_path,
                    description="worktree changed-file listing",
                )
            )
        paths = {
            relative_path
            for relative_path in paths
            if cls._is_meaningful_evidence_path(relative_path)
        }
        if not baseline_fingerprints:
            return sorted(paths)
        for relative_path, baseline in baseline_fingerprints.items():
            if not cls._is_meaningful_evidence_path(relative_path):
                continue
            current = cls._worktree_file_fingerprint(worktree_path, relative_path)
            if current == baseline:
                paths.discard(relative_path)
            else:
                paths.add(relative_path)
        return sorted(paths)

    @staticmethod
    def _is_meaningful_evidence_path(relative_path: str) -> bool:
        path = PurePosixPath(relative_path.replace("\\", "/"))
        ignored_parts = {
            ".gradle",
            ".mypy_cache",
            ".pytest_cache",
            ".ruff_cache",
            ".tox",
            "__pycache__",
            "build",
            "coverage",
            "dist",
            "node_modules",
            "target",
        }
        ignored_names = {".coverage"}
        ignored_suffixes = {".class", ".o", ".pyc", ".pyo"}
        return (
            not any(part in ignored_parts for part in path.parts)
            and path.name not in ignored_names
            and path.suffix.casefold() not in ignored_suffixes
        )

    @staticmethod
    def _worktree_file_fingerprint(
        worktree_path: Path,
        relative_path: str,
    ) -> str:
        path = PurePosixPath(relative_path.replace("\\", "/"))
        if not path.parts or ".." in path.parts or path.is_absolute():
            return "unsafe"
        candidate = worktree_path.joinpath(*path.parts)
        try:
            mode = candidate.lstat().st_mode
            if candidate.is_symlink():
                return (
                    f"symlink:{mode:o}:"
                    + sha256(
                        os.readlink(candidate).encode(
                            "utf-8",
                            errors="surrogateescape",
                        )
                    ).hexdigest()
                )
            if not candidate.exists():
                return "missing"
            if not candidate.is_file():
                return f"non-file:{mode:o}"
            stat_result = candidate.stat()
            digest = sha256()
            with candidate.open("rb") as handle:
                if stat_result.st_size <= _FINGERPRINT_SAMPLE_BYTES_LIMIT:
                    digest.update(handle.read(_FINGERPRINT_SAMPLE_BYTES_LIMIT))
                else:
                    prefix_limit = _FINGERPRINT_SAMPLE_BYTES_LIMIT // 2
                    suffix_limit = _FINGERPRINT_SAMPLE_BYTES_LIMIT - prefix_limit
                    digest.update(handle.read(prefix_limit))
                    handle.seek(max(0, stat_result.st_size - suffix_limit))
                    digest.update(handle.read(suffix_limit))
            return (
                f"file:{mode:o}:{stat_result.st_size}:"
                f"{stat_result.st_mtime_ns}:{stat_result.st_ctime_ns}:"
                + digest.hexdigest()
            )
        except FileNotFoundError:
            return "missing"
        except OSError as exc:
            return f"unreadable:{type(exc).__name__}"

    def _blockers_satisfied(self, issue_id: str) -> bool:
        return all(self._lifecycle_satisfies_blocker(self.issues[blocker]) for blocker in self.issues[issue_id].blocked_by)

    @staticmethod
    def _lifecycle_satisfies_blocker(issue: IssueSlice) -> bool:
        # Approval authorizes a Local Agent launch; it is not evidence that the
        # dependency's work completed. Dependent work becomes eligible only
        # after the blocker has an accepted reviewed outcome.
        return issue.review_state in {"pr-ready", "complete"}

    def _next_actions_for_issue(self, issue: IssueSlice) -> list[str]:
        if issue.review_state == "complete":
            return ["reopen"]
        if issue.review_state in {"pr-ready"}:
            return ["prepare-pr", "reopen"]
        if issue.review_state in {"needs-repair", "rejected"}:
            return ["repair", "reopen", "record-review"]
        if issue.review_state == "approved":
            delegation = self.delegations.get(issue.id)
            if delegation and delegation.requires_approval and not delegation.approved:
                return ["approve-delegation", "reopen"]
            if not delegation and self._has_router_agent():
                return ["route", "launch", "reopen"]
            if self._blockers_satisfied(issue.id):
                return ["launch", "reopen"]
            return ["wait-for-blockers", "reopen"]
        if issue.review_state == "needs-human-review":
            return ["record-review", "reopen"]
        return ["approve"]

    def _write_tracker_status(self, issue: IssueSlice, status: str) -> None:
        path = Path(issue.source_path)
        text = path.read_text(encoding="utf-8")
        if re.search(r"^Status: .*$", text, flags=re.MULTILINE):
            text = re.sub(r"^Status: .*$", f"Status: {status}", text, count=1, flags=re.MULTILINE)
        else:
            text = f"Status: {status}\n{text}"
        path.write_text(text, encoding="utf-8")

    def _record(self, message: str) -> None:
        self.timeline.append(message)

    def _next_action_for_review(self, session_id: str, outcome: str, failure_type: str) -> str:
        if outcome in {"Approved", "Approved with limitations"}:
            if self._session(session_id).issue_id not in self.issues:
                return "complete"
            return "prepare-pr"
        if outcome == "Needs human review":
            return "user-review"
        if outcome == "Needs repair":
            return "same-local-agent-repair"
        if outcome != "Rejected":
            return "record-review"
        if failure_type in {"critical", "security", "merge-risk"}:
            return "user-escalation"
        prior_rejections = sum(1 for review in self.reviews if review.session_id == session_id and review.outcome == "Rejected")
        if failure_type == "architecture" or prior_rejections >= 2:
            return "frontier-architect-revision"
        if prior_rejections == 1:
            return "fresh-local-agent-repair"
        return "same-local-agent-repair"

    def _latest_review_for_session(self, session_id: str) -> ReviewDecision | None:
        for review in reversed(self.reviews):
            if review.session_id == session_id:
                return review
        return None

    def _next_action(self) -> str:
        for issue_id in self.ordered_issue_ids():
            issue = self.issues[issue_id]
            if issue.review_state != "approved" and issue.review_state != "pr-ready":
                return f"Review {issue_id}"
        for issue_id in self.ordered_issue_ids():
            issue = self.issues[issue_id]
            if issue.review_state == "approved":
                return f"Launch {issue_id}"
        return "Prepare PRs or wait for human merge"

    def _evidence_index_lines(self, issue_id: str | None = None) -> list[str]:
        lines: list[str] = []
        for session in self.sessions.values():
            if issue_id and session.issue_id != issue_id:
                continue
            if not session.evidence:
                continue
            links = self.review_artifact_links(session) or [
                f"app-local://{self.project_key}/{session.session_id}/evidence"
            ]
            for link in links:
                lines.append(f"- {session.issue_id} {session.session_id}: {link}")
        return lines

    @staticmethod
    def _write(path: Path, content: str) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def _metadata(text: str) -> dict[str, str]:
    metadata: dict[str, str] = {}
    for line in text.splitlines():
        if line.startswith("## "):
            break
        if ":" in line:
            key, value = line.split(":", 1)
            metadata[key.strip().lower()] = value.strip()
    return metadata


def _record_type(text: str) -> str:
    for line in text.splitlines():
        if line.startswith("## "):
            break
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        if key.strip().lower() == "type":
            return value.strip().lower()
    return ""


def _sections(text: str) -> dict[str, str]:
    sections: dict[str, list[str]] = {}
    current: str | None = None
    for line in text.splitlines():
        if line.startswith("## "):
            current = line.removeprefix("## ").strip()
            sections[current] = []
            continue
        if current:
            sections[current].append(line)
    return {heading: "\n".join(lines).strip() for heading, lines in sections.items()}


def _checklist_items(text: str) -> list[str]:
    items: list[str] = []
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("- [ ] "):
            items.append(stripped.removeprefix("- [ ] ").strip())
        elif stripped.startswith("- "):
            items.append(stripped.removeprefix("- ").strip())
    return items


def _issue_refs(text: str, issues_dir: Path | None = None) -> list[str]:
    if "None - can start immediately" in text:
        return []
    refs: list[str] = []
    for match in re.finditer(r"ISS-(\d+)", text, flags=re.IGNORECASE):
        refs.append(f"ISS-{int(match.group(1)):02d}")
    for match in re.finditer(
        r"(?:^|[/`\s])((\d+)-[^/`\s]+\.md)(?=`|\s|$)",
        text,
        flags=re.IGNORECASE | re.MULTILINE,
    ):
        if issues_dir is not None and not (issues_dir / match.group(1)).exists():
            continue
        refs.append(f"ISS-{int(match.group(2)):02d}")
    deduped: list[str] = []
    for ref in refs:
        if ref not in deduped:
            deduped.append(ref)
    return deduped


def _slug(value: str) -> str:
    value = value.removesuffix(".md").lower()
    value = re.sub(r"[^a-z0-9]+", "-", value).strip("-")
    return value or "issue"


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _normalize_review_outcome(outcome: str) -> str:
    normalized = outcome.strip().lower().replace("_", " ").replace("-", " ")
    known = {
        "approved": "Approved",
        "approved with limitations": "Approved with limitations",
        "needs repair": "Needs repair",
        "needs human review": "Needs human review",
        "rejected": "Rejected",
    }
    if normalized not in known:
        raise AlbertError(f"Unknown review outcome: {outcome}")
    return known[normalized]


_ANSI_SEQUENCE_RE = re.compile(
    r"\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]|\x1b[@-Z\\-_]"
)
_JSON_FENCE_RE = re.compile(r"```(?:json)?\s*(.*?)```", flags=re.IGNORECASE | re.DOTALL)


def _clean_model_output(output: str) -> str:
    text = output.replace("\r\n", "\n").replace("\r", "\n")
    text = _ANSI_SEQUENCE_RE.sub("", text)
    return re.sub(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]", "", text)


def _validate_model_file_plan(data: Any) -> dict[str, Any]:
    if not isinstance(data, dict):
        raise AlbertError("model output must be a JSON object")
    files = data.get("files")
    if not isinstance(files, list) or not files:
        raise AlbertError("model output must include a non-empty files list")
    if len(files) > _MODEL_FILE_COUNT_LIMIT:
        raise AlbertError(
            f"model output files exceeds the {_MODEL_FILE_COUNT_LIMIT}-file limit"
        )
    parsed_files: list[dict[str, str]] = []
    total_file_bytes = 0
    for index, item in enumerate(files, start=1):
        if not isinstance(item, dict):
            raise AlbertError(f"files[{index}] must be an object")
        path = str(item.get("path", "")).strip()
        content = item.get("content")
        if not path:
            raise AlbertError(f"files[{index}] is missing path")
        if not isinstance(content, str):
            raise AlbertError(f"files[{index}] is missing string content")
        content_bytes = len(content.encode("utf-8", errors="replace"))
        if content_bytes > _MODEL_FILE_BYTES_LIMIT:
            raise AlbertError(
                f"files[{index}] exceeds the {_MODEL_FILE_BYTES_LIMIT}-byte per-file limit"
            )
        total_file_bytes += content_bytes
        if total_file_bytes > _MODEL_FILE_TOTAL_BYTES_LIMIT:
            raise AlbertError(
                "model output files exceeds the "
                f"{_MODEL_FILE_TOTAL_BYTES_LIMIT}-byte total limit"
            )
        parsed_files.append({"path": path, "content": content})
    commands = data.get("commands", [])
    if not isinstance(commands, list):
        raise AlbertError("model output commands must be a list")
    if len(commands) > _MODEL_COMMAND_LIMIT:
        raise AlbertError(
            f"model output commands exceeds the {_MODEL_COMMAND_LIMIT}-command limit"
        )
    parsed_commands: list[str] = []
    for index, command in enumerate(commands, start=1):
        if not isinstance(command, str) or not command.strip():
            raise AlbertError(f"commands[{index}] must be a non-empty string")
        normalized = command.strip()
        if len(normalized) > _MODEL_COMMAND_LENGTH_LIMIT:
            raise AlbertError(
                f"commands[{index}] exceeds the {_MODEL_COMMAND_LENGTH_LIMIT}-character limit"
            )
        parsed_commands.append(normalized)
    return {
        "summary": str(data.get("summary", "")),
        "files": parsed_files,
        "commands": parsed_commands,
    }


def _balanced_json_candidates(text: str) -> list[str]:
    candidates: list[str] = []
    start: int | None = None
    depth = 0
    in_string = False
    escaped = False
    for index, char in enumerate(text):
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "{":
            if depth == 0:
                start = index
            depth += 1
        elif char == "}" and depth:
            depth -= 1
            if depth == 0 and start is not None:
                candidates.append(text[start : index + 1])
                start = None
    return candidates


def _validate_delegation_decision(data: Any) -> dict[str, Any]:
    if not isinstance(data, dict):
        raise AlbertError("delegation output must be a JSON object")
    complexity = str(data.get("complexity", "")).strip().lower()
    if complexity not in {"low", "medium", "high", "architectural"}:
        raise AlbertError("delegation output must include complexity: low, medium, high, or architectural")
    recommended_agent = str(data.get("recommended_agent", "")).strip()
    if not recommended_agent:
        raise AlbertError("delegation output is missing recommended_agent")
    reason = str(data.get("reason", "")).strip()
    if not reason:
        raise AlbertError("delegation output is missing reason")
    requires_approval = bool(data.get("requires_cloud_approval", data.get("requires_approval", False)))
    return {
        "complexity": complexity,
        "recommended_agent": recommended_agent,
        "requires_approval": requires_approval,
        "reason": reason,
    }


def _load_valid_delegation_decision(candidate: str) -> dict[str, Any] | None:
    try:
        data = json.loads(candidate.strip())
        return _validate_delegation_decision(data)
    except (json.JSONDecodeError, AlbertError):
        return None


def _parse_delegation_decision(output: str) -> dict[str, Any]:
    text = _clean_model_output(output).strip()
    if not text:
        raise AlbertError("router returned empty output")
    try:
        data = json.loads(text)
        return _validate_delegation_decision(data)
    except json.JSONDecodeError:
        pass

    for match in reversed(list(_JSON_FENCE_RE.finditer(text))):
        decision = _load_valid_delegation_decision(match.group(1))
        if decision:
            return decision

    for candidate in reversed(_balanced_json_candidates(text)):
        decision = _load_valid_delegation_decision(candidate)
        if decision:
            return decision

    if "{" not in text or "}" not in text:
        raise AlbertError("router output is not JSON")
    try:
        json.loads(text)
    except json.JSONDecodeError as exc:
        raise AlbertError(f"router output is not valid JSON: {exc}") from exc
    raise AlbertError("router output must include complexity, recommended_agent, and reason")


def _load_valid_model_file_plan(candidate: str) -> dict[str, Any] | None:
    try:
        data = json.loads(candidate.strip())
        return _validate_model_file_plan(data)
    except (json.JSONDecodeError, AlbertError):
        return None


def _parse_model_file_plan(output: str) -> dict[str, Any]:
    text = _clean_model_output(output).strip()
    if not text:
        raise AlbertError("model returned empty output")
    try:
        data = json.loads(text)
        return _validate_model_file_plan(data)
    except json.JSONDecodeError:
        pass

    for match in reversed(list(_JSON_FENCE_RE.finditer(text))):
        plan = _load_valid_model_file_plan(match.group(1))
        if plan:
            return plan

    for candidate in reversed(_balanced_json_candidates(text)):
        plan = _load_valid_model_file_plan(candidate)
        if plan:
            return plan

    if "{" not in text or "}" not in text:
        raise AlbertError("model output is not JSON")
    try:
        json.loads(text)
    except json.JSONDecodeError as exc:
        raise AlbertError(f"model output is not valid JSON: {exc}") from exc
    raise AlbertError("model output must include a non-empty files list")
