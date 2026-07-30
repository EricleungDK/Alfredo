from __future__ import annotations

from contextlib import contextmanager
from dataclasses import asdict, dataclass, replace
import fcntl
from hashlib import sha256
import json
import os
from pathlib import Path
import shutil
import secrets
import subprocess
import tempfile
from typing import Iterator, Literal


SelectionMode = Literal["existing", "create"]
_RECEIPT_SCHEMA_VERSION = 1
_RECEIPT_SIZE_LIMIT = 64_000
_CREATE_MARKER_NAME = ".alfredo-create-pending"
_GIT_PROCESS_ENV_ALLOWLIST = {
    "CI",
    "LANG",
    "LC_ALL",
    "NO_COLOR",
    "PATH",
    "TERM",
}


def sanitized_process_environment() -> dict[str, str]:
    """Return a bounded Git environment without inherited repository controls."""
    environment = {
        key: value
        for key, value in os.environ.items()
        if key in _GIT_PROCESS_ENV_ALLOWLIST
    }
    environment.setdefault("PATH", os.defpath)
    environment["HOME"] = "/tmp"
    environment["TMPDIR"] = "/tmp"
    return environment


class CodingWorkspaceSelectionError(Exception):
    """A structured failure before a Coding Workspace becomes accepted state."""

    def __init__(
        self,
        code: str,
        message: str,
        *,
        recoverable: bool = True,
    ) -> None:
        self.code = code
        self.recoverable = recoverable
        super().__init__(message)


@dataclass(frozen=True)
class CodingWorkspaceAcknowledgement:
    schema_version: int
    correlation_id: str
    outcome: Literal["acknowledged"]
    starting_location: str
    coding_workspace: str
    selection_mode: SelectionMode
    active_mission: None
    replayed: bool
    message: str

    def to_dict(self) -> dict[str, object]:
        return asdict(self)


class CodingWorkspaceSelectionService:
    """Validate the exact repository boundary before acknowledging selection."""

    def __init__(
        self,
        *,
        starting_location: Path,
        runtime_root: Path,
        forbidden_roots: tuple[Path, ...] = (),
    ) -> None:
        self.starting_location = self._existing_directory(
            starting_location, label="Starting Location"
        )
        self.runtime_root = runtime_root.resolve(strict=False)
        self.forbidden_roots = tuple(
            root.resolve(strict=False) for root in forbidden_roots
        ) + (self.runtime_root,)

    def select(
        self,
        *,
        correlation_id: str,
        workspace_path: Path,
        selection_mode: SelectionMode,
    ) -> CodingWorkspaceAcknowledgement:
        if not correlation_id.strip():
            raise CodingWorkspaceSelectionError(
                "contract-failure", "Coding Workspace selection requires a correlation id."
            )
        if selection_mode not in ("existing", "create"):
            raise CodingWorkspaceSelectionError(
                "contract-failure",
                f"Unsupported Coding Workspace selection mode: {selection_mode}",
            )
        normalized_workspace = workspace_path.resolve(strict=False)
        request_boundary = {
            "starting_location": str(self.starting_location),
            "workspace_path": str(normalized_workspace),
            "selection_mode": selection_mode,
        }
        with self._correlation_lock(correlation_id):
            return self._select_locked(
                correlation_id=correlation_id,
                normalized_workspace=normalized_workspace,
                selection_mode=selection_mode,
                request_boundary=request_boundary,
            )

    def _select_locked(
        self,
        *,
        correlation_id: str,
        normalized_workspace: Path,
        selection_mode: SelectionMode,
        request_boundary: dict[str, object],
    ) -> CodingWorkspaceAcknowledgement:
        receipt = self._load_receipt(correlation_id)
        if receipt is not None:
            if receipt.get("request") != request_boundary:
                raise CodingWorkspaceSelectionError(
                    "correlation-conflict",
                    "The Coding Workspace correlation id was already used for a different boundary.",
                    recoverable=False,
                )
            stored_acknowledgement = receipt.get("acknowledgement")
            if stored_acknowledgement is not None:
                acknowledgement = self._acknowledgement_from_receipt(
                    stored_acknowledgement,
                    correlation_id=correlation_id,
                    request_boundary=request_boundary,
                )
                coding_workspace = self._existing_directory(
                    Path(acknowledgement.coding_workspace),
                    label="Coding Workspace",
                )
                self._reject_forbidden_root(coding_workspace)
                self._validate_exact_git_repository(coding_workspace)
                if str(coding_workspace) != acknowledgement.coding_workspace:
                    raise CodingWorkspaceSelectionError(
                        "runtime-state-invalid",
                        "The persisted Coding Workspace boundary is no longer canonical.",
                        recoverable=False,
                    )
                completed_create_token = receipt.get("create_token")
                if completed_create_token is not None:
                    if (
                        not isinstance(completed_create_token, str)
                        or len(completed_create_token) != 64
                    ):
                        raise CodingWorkspaceSelectionError(
                            "runtime-state-invalid",
                            "The completed Coding Workspace receipt has an invalid cleanup token.",
                            recoverable=False,
                        )
                    self._reconcile_completed_create_marker(
                        coding_workspace,
                        completed_create_token,
                    )
                    self._write_receipt(
                        correlation_id,
                        request_boundary=request_boundary,
                        acknowledgement=acknowledgement,
                        create_token=None,
                    )
                return replace(acknowledgement, replayed=True)

        create_token: str | None = None
        if selection_mode == "existing":
            coding_workspace = self._existing_directory(
                normalized_workspace, label="Coding Workspace"
            )
        elif normalized_workspace.exists() and receipt is not None:
            create_token = self._receipt_create_token(receipt)
            self._verify_create_marker(normalized_workspace, create_token)
            coding_workspace = self._existing_directory(
                normalized_workspace, label="Coding Workspace"
            )
        else:
            coding_workspace = self._new_repository_target(normalized_workspace)
            if receipt is None:
                create_token = secrets.token_hex(32)
                self._write_receipt(
                    correlation_id,
                    request_boundary=request_boundary,
                    acknowledgement=None,
                    create_token=create_token,
                )
            else:
                create_token = self._receipt_create_token(receipt)
            coding_workspace = self._create_repository(
                coding_workspace,
                create_token=create_token,
            )

        self._reject_forbidden_root(coding_workspace)
        self._validate_exact_git_repository(coding_workspace)
        acknowledgement = CodingWorkspaceAcknowledgement(
            schema_version=1,
            correlation_id=correlation_id,
            outcome="acknowledged",
            starting_location=str(self.starting_location),
            coding_workspace=str(coding_workspace),
            selection_mode=selection_mode,
            active_mission=None,
            replayed=False,
            message=(
                "Coding Workspace acknowledged by the Orchestrator; "
                "no Mission has been selected."
            ),
        )
        self._write_receipt(
            correlation_id,
            request_boundary=request_boundary,
            acknowledgement=acknowledgement,
            create_token=create_token,
        )
        if create_token is not None:
            self._clear_create_marker(coding_workspace)
            self._write_receipt(
                correlation_id,
                request_boundary=request_boundary,
                acknowledgement=acknowledgement,
                create_token=None,
            )
        return acknowledgement

    @contextmanager
    def _correlation_lock(self, correlation_id: str) -> Iterator[None]:
        lock_path = self._receipt_path(correlation_id).with_suffix(".lock")
        try:
            lock_path.parent.mkdir(parents=True, exist_ok=True)
            with lock_path.open("a+b") as lock_file:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
                try:
                    yield
                finally:
                    fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
        except OSError as error:
            raise CodingWorkspaceSelectionError(
                "runtime-state-lock-failed",
                "The Coding Workspace selection correlation could not be locked.",
            ) from error

    def _new_repository_target(self, workspace_path: Path) -> Path:
        if workspace_path.exists() or workspace_path.is_symlink():
            raise CodingWorkspaceSelectionError(
                "workspace-invalid",
                "A new Coding Workspace target must not already exist.",
            )
        try:
            parent = workspace_path.parent.resolve(strict=True)
        except (OSError, RuntimeError) as error:
            raise CodingWorkspaceSelectionError(
                "workspace-invalid",
                "The parent directory for the new Coding Workspace is unavailable.",
            ) from error
        if parent != self.starting_location and self.starting_location not in parent.parents:
            raise CodingWorkspaceSelectionError(
                "workspace-unsafe",
                "A new repository must be created below the Starting Location.",
            )
        if not workspace_path.name or workspace_path.name in (".", ".."):
            raise CodingWorkspaceSelectionError(
                "workspace-invalid", "The new Coding Workspace requires a directory name."
            )
        coding_workspace = parent / workspace_path.name
        self._reject_forbidden_root(coding_workspace)
        return coding_workspace

    def _create_repository(
        self,
        coding_workspace: Path,
        *,
        create_token: str | None,
    ) -> Path:
        created_directory = False
        try:
            coding_workspace.mkdir()
            created_directory = True
            if not create_token:
                raise CodingWorkspaceSelectionError(
                    "runtime-state-invalid",
                    "The pending Coding Workspace create receipt has no ownership token.",
                    recoverable=False,
                )
            marker = coding_workspace / _CREATE_MARKER_NAME
            with marker.open("x", encoding="utf-8") as marker_file:
                marker_file.write(create_token)
            result = subprocess.run(
                ["git", "init", "--quiet", str(coding_workspace)],
                capture_output=True,
                check=False,
                env=sanitized_process_environment(),
                text=True,
                timeout=10,
            )
            if result.returncode != 0:
                raise CodingWorkspaceSelectionError(
                    "workspace-create-failed",
                    "Git could not initialize the new Coding Workspace.",
                )
        except (OSError, subprocess.TimeoutExpired) as error:
            if created_directory:
                self._cleanup_owned_create(coding_workspace, create_token)
            raise CodingWorkspaceSelectionError(
                "workspace-create-failed",
                "The new Coding Workspace could not be created.",
            ) from error
        except CodingWorkspaceSelectionError:
            if created_directory:
                self._cleanup_owned_create(coding_workspace, create_token)
            raise
        return coding_workspace.resolve(strict=True)

    @staticmethod
    def _receipt_create_token(receipt: dict[str, object]) -> str:
        create_token = receipt.get("create_token")
        if not isinstance(create_token, str) or len(create_token) != 64:
            raise CodingWorkspaceSelectionError(
                "runtime-state-invalid",
                "The pending Coding Workspace create receipt has no valid ownership token.",
                recoverable=False,
            )
        return create_token

    @staticmethod
    def _verify_create_marker(coding_workspace: Path, create_token: str) -> None:
        marker = coding_workspace / _CREATE_MARKER_NAME
        try:
            if marker.is_symlink() or marker.read_text(encoding="utf-8") != create_token:
                raise ValueError("create ownership marker mismatch")
        except (OSError, UnicodeError, ValueError) as error:
            raise CodingWorkspaceSelectionError(
                "workspace-create-conflict",
                "The pending Coding Workspace path was created outside this Alfredo request.",
                recoverable=False,
            ) from error

    @staticmethod
    def _clear_create_marker(coding_workspace: Path) -> None:
        try:
            (coding_workspace / _CREATE_MARKER_NAME).unlink(missing_ok=True)
        except OSError as error:
            raise CodingWorkspaceSelectionError(
                "workspace-create-cleanup-failed",
                "The Coding Workspace was created, but its ownership marker could not be cleared.",
            ) from error

    @staticmethod
    def _reconcile_completed_create_marker(
        coding_workspace: Path,
        create_token: str,
    ) -> None:
        marker = coding_workspace / _CREATE_MARKER_NAME
        try:
            if not marker.exists():
                return
            if marker.is_symlink() or marker.read_text(encoding="utf-8") != create_token:
                raise ValueError("completed create ownership marker mismatch")
            marker.unlink()
        except (OSError, UnicodeError, ValueError) as error:
            raise CodingWorkspaceSelectionError(
                "workspace-create-cleanup-failed",
                "The completed Coding Workspace ownership marker could not be reconciled.",
            ) from error

    @staticmethod
    def _cleanup_owned_create(coding_workspace: Path, create_token: str | None) -> None:
        if not create_token:
            return
        marker = coding_workspace / _CREATE_MARKER_NAME
        try:
            if (
                not marker.is_symlink()
                and marker.read_text(encoding="utf-8") == create_token
            ):
                shutil.rmtree(coding_workspace)
        except (OSError, UnicodeError):
            pass

    def _receipt_path(self, correlation_id: str) -> Path:
        digest = sha256(correlation_id.encode("utf-8")).hexdigest()
        return self.runtime_root / "workspace-selection-receipts" / f"{digest}.json"

    def _load_receipt(self, correlation_id: str) -> dict[str, object] | None:
        path = self._receipt_path(correlation_id)
        try:
            if not path.exists():
                return None
            if path.stat().st_size > _RECEIPT_SIZE_LIMIT:
                raise ValueError("receipt exceeds the size limit")
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as error:
            raise CodingWorkspaceSelectionError(
                "runtime-state-invalid",
                "The persisted Coding Workspace selection receipt is invalid.",
                recoverable=False,
            ) from error
        if (
            not isinstance(value, dict)
            or value.get("schema_version") != _RECEIPT_SCHEMA_VERSION
            or value.get("correlation_id") != correlation_id
            or not isinstance(value.get("request"), dict)
        ):
            raise CodingWorkspaceSelectionError(
                "runtime-state-invalid",
                "The persisted Coding Workspace selection receipt is invalid.",
                recoverable=False,
            )
        return value

    def _write_receipt(
        self,
        correlation_id: str,
        *,
        request_boundary: dict[str, object],
        acknowledgement: CodingWorkspaceAcknowledgement | None,
        create_token: str | None,
    ) -> None:
        path = self._receipt_path(correlation_id)
        payload = {
            "schema_version": _RECEIPT_SCHEMA_VERSION,
            "correlation_id": correlation_id,
            "request": request_boundary,
            "acknowledgement": (
                acknowledgement.to_dict() if acknowledgement is not None else None
            ),
            "create_token": create_token,
        }
        temporary_path: Path | None = None
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            descriptor, temporary_name = tempfile.mkstemp(
                dir=path.parent,
                prefix=f".{path.stem}-",
                suffix=".tmp",
            )
            temporary_path = Path(temporary_name)
            with os.fdopen(descriptor, "w", encoding="utf-8") as destination:
                json.dump(payload, destination, sort_keys=True)
                destination.flush()
                os.fsync(destination.fileno())
            os.replace(temporary_path, path)
        except OSError as error:
            raise CodingWorkspaceSelectionError(
                "runtime-state-write-failed",
                "The Coding Workspace selection receipt could not be persisted.",
            ) from error
        finally:
            if temporary_path is not None:
                try:
                    temporary_path.unlink(missing_ok=True)
                except OSError:
                    pass

    @staticmethod
    def _acknowledgement_from_receipt(
        value: object,
        *,
        correlation_id: str,
        request_boundary: dict[str, object],
    ) -> CodingWorkspaceAcknowledgement:
        if not isinstance(value, dict):
            raise CodingWorkspaceSelectionError(
                "runtime-state-invalid",
                "The persisted Coding Workspace selection acknowledgement is invalid.",
                recoverable=False,
            )
        try:
            acknowledgement = CodingWorkspaceAcknowledgement(**value)
        except (TypeError, ValueError) as error:
            raise CodingWorkspaceSelectionError(
                "runtime-state-invalid",
                "The persisted Coding Workspace selection acknowledgement is invalid.",
                recoverable=False,
            ) from error
        if (
            acknowledgement.schema_version != 1
            or acknowledgement.correlation_id != correlation_id
            or acknowledgement.outcome != "acknowledged"
            or acknowledgement.starting_location
            != request_boundary["starting_location"]
            or acknowledgement.coding_workspace != request_boundary["workspace_path"]
            or acknowledgement.selection_mode != request_boundary["selection_mode"]
            or acknowledgement.active_mission is not None
        ):
            raise CodingWorkspaceSelectionError(
                "runtime-state-invalid",
                "The persisted Coding Workspace selection acknowledgement is invalid.",
                recoverable=False,
            )
        return acknowledgement

    @staticmethod
    def _existing_directory(path: Path, *, label: str) -> Path:
        try:
            resolved = path.resolve(strict=True)
        except (OSError, RuntimeError) as error:
            raise CodingWorkspaceSelectionError(
                "workspace-invalid", f"{label} is unavailable: {path}"
            ) from error
        if not resolved.is_dir():
            raise CodingWorkspaceSelectionError(
                "workspace-invalid", f"{label} is not a directory: {resolved}"
            )
        if not os.access(resolved, os.R_OK | os.W_OK):
            raise CodingWorkspaceSelectionError(
                "workspace-unsafe",
                f"{label} must be readable and writable: {resolved}",
            )
        return resolved

    def _reject_forbidden_root(self, coding_workspace: Path) -> None:
        for forbidden in self.forbidden_roots:
            if (
                coding_workspace == forbidden
                or forbidden in coding_workspace.parents
                or coding_workspace in forbidden.parents
            ):
                raise CodingWorkspaceSelectionError(
                    "workspace-unsafe",
                    "The selected repository overlaps an Alfredo install, backend, or runtime root.",
                )

    @staticmethod
    def _validate_exact_git_repository(coding_workspace: Path) -> None:
        try:
            result = subprocess.run(
                [
                    "git",
                    "-C",
                    str(coding_workspace),
                    "rev-parse",
                    "--show-toplevel",
                ],
                capture_output=True,
                check=False,
                env=sanitized_process_environment(),
                text=True,
                timeout=5,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise CodingWorkspaceSelectionError(
                "workspace-validation-failed",
                "Git could not validate the selected Coding Workspace.",
            ) from error
        if result.returncode != 0:
            raise CodingWorkspaceSelectionError(
                "workspace-invalid",
                "The selected Coding Workspace is not a Git repository.",
            )
        try:
            repository_root = Path(result.stdout.strip()).resolve(strict=True)
        except (OSError, RuntimeError) as error:
            raise CodingWorkspaceSelectionError(
                "workspace-validation-failed",
                "Git returned an unavailable repository root.",
            ) from error
        if repository_root != coding_workspace:
            raise CodingWorkspaceSelectionError(
                "workspace-invalid",
                "Select the exact Git repository root, not one of its subdirectories.",
            )
