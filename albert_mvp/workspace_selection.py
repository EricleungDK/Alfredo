from __future__ import annotations

from contextlib import contextmanager
from dataclasses import asdict, dataclass, replace
import fcntl
from hashlib import sha256
import json
import os
from pathlib import Path
import re
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
    known_missions: tuple[dict[str, str], ...] = ()

    def to_dict(self) -> dict[str, object]:
        payload = asdict(self)
        payload["known_missions"] = [dict(item) for item in self.known_missions]
        return payload


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
                self._record_journey_state(
                    acknowledgement=acknowledgement,
                    selection_mode=selection_mode,
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
        known_missions = tuple(
            option.to_public_dict()
            for option in WorkspaceJourneyStore.discover_missions(coding_workspace)
        )
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
            known_missions=known_missions,
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
        self._record_journey_state(
            acknowledgement=acknowledgement,
            selection_mode=selection_mode,
        )
        return acknowledgement

    def _record_journey_state(
        self,
        *,
        acknowledgement: CodingWorkspaceAcknowledgement,
        selection_mode: SelectionMode,
    ) -> None:
        try:
            WorkspaceJourneyStore(self.runtime_root).record_selection(
                starting_location=self.starting_location,
                coding_workspace=Path(acknowledgement.coding_workspace),
                correlation_id=acknowledgement.correlation_id,
                selection_mode=selection_mode,
            )
        except MissionChoiceError as error:
            raise CodingWorkspaceSelectionError(
                error.code,
                str(error),
                recoverable=error.recoverable,
            ) from error

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
            normalized_value = dict(value)
            known_missions = normalized_value.get("known_missions", [])
            if not isinstance(known_missions, (list, tuple)) or any(
                not isinstance(item, dict)
                or not isinstance(item.get("id"), str)
                or not isinstance(item.get("title"), str)
                for item in known_missions
            ):
                raise ValueError("known missions must be objects")
            normalized_value["known_missions"] = tuple(
                {"id": item["id"], "title": item["title"]}
                for item in known_missions
            )
            acknowledgement = CodingWorkspaceAcknowledgement(**normalized_value)
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

MissionChoice = Literal["resume", "new"]
_STATE_SCHEMA_VERSION = 1
_MISSION_ID_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
_MISSION_TITLE_LIMIT = 240


class MissionChoiceError(Exception):
    """A structured failure while choosing or restoring a Mission."""

    def __init__(
        self,
        code: str,
        message: str,
        *,
        recoverable: bool = True,
        expected_revision: int | None = None,
        current_revision: int | None = None,
    ) -> None:
        self.code = code
        self.recoverable = recoverable
        self.expected_revision = expected_revision
        self.current_revision = current_revision
        super().__init__(message)


@dataclass(frozen=True)
class MissionOption:
    id: str
    title: str
    tracker_dir: str
    issues_dir: str

    def to_public_dict(self) -> dict[str, str]:
        return {"id": self.id, "title": self.title}

    def to_catalog_dict(self) -> dict[str, str]:
        return {
            "mission_id": self.id,
            "tracker_dir": self.tracker_dir,
            "issues_dir": self.issues_dir,
        }

    def to_dict(self) -> dict[str, str]:
        return {
            "id": self.id,
            "title": self.title,
            "tracker_dir": self.tracker_dir,
            "issues_dir": self.issues_dir,
        }

    @classmethod
    def from_dict(cls, value: object) -> "MissionOption":
        if not isinstance(value, dict):
            raise ValueError("Mission option must be an object")
        mission_id = value.get("id")
        title = value.get("title")
        tracker_dir = value.get("tracker_dir")
        issues_dir = value.get("issues_dir", "")
        if not all(isinstance(item, str) for item in (mission_id, title, tracker_dir, issues_dir)):
            raise ValueError("Mission option fields must be strings")
        if not mission_id or not title or not tracker_dir:
            raise ValueError("Mission option fields must not be empty")
        return cls(
            id=mission_id,
            title=title,
            tracker_dir=tracker_dir,
            issues_dir=issues_dir,
        )


def _canonical_directory(path: Path, *, label: str) -> str:
    try:
        resolved = path.resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise MissionChoiceError("workspace-session-invalid", f"{label} is unavailable: {path}") from error
    if not resolved.is_dir():
        raise MissionChoiceError("workspace-session-invalid", f"{label} is not a directory: {resolved}")
    return str(resolved)


class WorkspaceJourneyStore:
    """Persist the accepted Workspace/Mission journey independently of a process."""

    def __init__(self, runtime_root: Path) -> None:
        self.runtime_root = runtime_root.resolve(strict=False)
        self.state_path = self.runtime_root / "workspace-sessions.json"
        self.lock_path = self.runtime_root / "workspace-sessions.lock"

    @staticmethod
    def discover_missions(coding_workspace: Path) -> tuple[MissionOption, ...]:
        workspace = coding_workspace.resolve(strict=True)
        candidates: list[tuple[str, Path, str]] = []
        agent_issues = workspace / ".agent" / "issues"
        candidates.append(("agent-issues", agent_issues, "agent-issues"))

        for root_name in (".alfredo/missions", ".scratch"):
            root = workspace / root_name
            try:
                children = sorted(root.iterdir(), key=lambda path: path.name)
            except OSError:
                children = []
            for child in children:
                if child.is_dir():
                    prefix = "scratch-" if root_name == ".scratch" else ""
                    candidates.append((f"{prefix}{child.name}", child, child.name))

        direct_agent_prd = workspace / ".agent" / "PRD.md"
        if direct_agent_prd.exists():
            candidates.append(("agent", direct_agent_prd.parent, "agent"))

        discovered: list[MissionOption] = []
        seen_ids: set[str] = set()
        for mission_id, tracker_dir, fallback_title in candidates:
            prd_path = tracker_dir / "PRD.md"
            if not prd_path.is_file() or mission_id in seen_ids:
                continue
            title = _read_prd_title(prd_path, fallback=fallback_title)
            issues_dir = tracker_dir / "issues"
            if not issues_dir.is_dir() and tracker_dir.name == "issues":
                issues_dir = tracker_dir
            discovered.append(
                MissionOption(
                    id=mission_id,
                    title=title,
                    tracker_dir=str(tracker_dir.resolve(strict=True)),
                    issues_dir=str(issues_dir.resolve(strict=False)),
                )
            )
            seen_ids.add(mission_id)
        return tuple(discovered)

    def record_selection(
        self,
        *,
        starting_location: Path,
        coding_workspace: Path,
        correlation_id: str,
        selection_mode: str,
    ) -> dict[str, object]:
        starting = _canonical_directory(starting_location, label="Starting Location")
        workspace = _canonical_directory(coding_workspace, label="Coding Workspace")
        with self._locked():
            payload = self._load_payload_locked()
            sessions = payload["sessions"]
            session = self._find_session(sessions, starting_location=starting, coding_workspace=workspace)
            if session is None:
                options = list(self.discover_missions(Path(workspace)))
                session = {
                    "starting_location": starting,
                    "coding_workspace": workspace,
                    "revision": 1,
                    "active_mission": None,
                    "missions": [option.to_dict() for option in options],
                    "mission_catalog": str(self._catalog_path(starting, workspace)),
                    "selection": {
                        "correlation_id": correlation_id,
                        "selection_mode": selection_mode,
                    },
                    "receipts": {},
                }
                sessions.append(session)
            else:
                self._validate_session(session)
                session["selection"] = {
                    "correlation_id": correlation_id,
                    "selection_mode": selection_mode,
                }
                # Discovery may find a Mission added outside Alfredo between
                # restarts. Merge it, but never replace an accepted identity.
                known = [MissionOption.from_dict(item) for item in session["missions"]]
                known_ids = {option.id for option in known}
                for option in self.discover_missions(Path(workspace)):
                    if option.id not in known_ids:
                        known.append(option)
                        known_ids.add(option.id)
                session["missions"] = [option.to_dict() for option in known]
            self._write_catalog_locked(session)
            self._write_payload_locked(payload)
            return self._public_state(session)

    def mission_options(
        self,
        *,
        starting_location: Path,
        coding_workspace: Path,
    ) -> dict[str, object]:
        starting = _canonical_directory(starting_location, label="Starting Location")
        workspace = _canonical_directory(coding_workspace, label="Coding Workspace")
        with self._locked():
            payload = self._load_payload_locked()
            session = self._require_session(
                payload["sessions"], starting_location=starting, coding_workspace=workspace
            )
            return self._public_state(session)

    def workspace_context(self, *, starting_location: Path) -> dict[str, object]:
        starting = _canonical_directory(starting_location, label="Starting Location")
        with self._locked():
            payload = self._load_payload_locked()
            matches = [
                session
                for session in payload["sessions"]
                if session.get("starting_location") == starting
            ]
            if not matches:
                raise MissionChoiceError(
                    "workspace-session-not-found",
                    "No acknowledged Coding Workspace is persisted for this Starting Location.",
                )
            if len(matches) > 1:
                raise MissionChoiceError(
                    "workspace-session-ambiguous",
                    "More than one acknowledged Coding Workspace belongs to this Starting Location; select one explicitly.",
                )
            self._validate_session(matches[0])
            return self._public_state(matches[0])

    def choose_mission(
        self,
        *,
        starting_location: Path,
        coding_workspace: Path,
        correlation_id: str,
        expected_revision: int,
        choice: MissionChoice,
        mission_id: str,
        mission_title: str,
    ) -> dict[str, object]:
        if not correlation_id.strip():
            raise MissionChoiceError("contract-failure", "Mission choice requires a correlation id.")
        if choice not in ("resume", "new"):
            raise MissionChoiceError("contract-failure", f"Unsupported Mission choice: {choice}")
        if expected_revision < 1:
            raise MissionChoiceError("contract-failure", "Mission choice revision must be positive.")
        starting = _canonical_directory(starting_location, label="Starting Location")
        workspace = _canonical_directory(coding_workspace, label="Coding Workspace")
        request = {
            "starting_location": starting,
            "coding_workspace": workspace,
            "expected_revision": expected_revision,
            "choice": choice,
            "mission_id": mission_id,
            "mission_title": mission_title,
        }
        with self._locked():
            payload = self._load_payload_locked()
            session = self._require_session(
                payload["sessions"], starting_location=starting, coding_workspace=workspace
            )
            self._validate_session(session)
            receipts = session["receipts"]
            prior = receipts.get(correlation_id)
            if prior is not None:
                if prior.get("request") != request:
                    raise MissionChoiceError(
                        "correlation-conflict",
                        "The Mission choice correlation id was already used for a different request.",
                        recoverable=False,
                    )
                acknowledgement = dict(prior["acknowledgement"])
                acknowledgement["replayed"] = True
                return acknowledgement
            current_revision = int(session["revision"])
            if expected_revision != current_revision:
                raise MissionChoiceError(
                    "stale-action",
                    "The Mission choice was based on an older Workspace journey revision.",
                    expected_revision=expected_revision,
                    current_revision=current_revision,
                )
            options = [MissionOption.from_dict(item) for item in session["missions"]]
            if session["active_mission"] is not None:
                if choice == "new" and any(option.id == mission_id for option in options):
                    raise MissionChoiceError(
                        "mission-duplicate",
                        f"A Mission with this identity already exists: {mission_id}",
                    )
                raise MissionChoiceError(
                    "mission-already-selected",
                    "This Workspace already has an accepted Mission.",
                )
            if choice == "resume":
                selected = next((option for option in options if option.id == mission_id), None)
                if selected is None:
                    raise MissionChoiceError(
                        "mission-not-found",
                        f"The requested Mission is not known for this Coding Workspace: {mission_id}",
                    )
            else:
                selected = self._create_new_mission(
                    workspace=Path(workspace),
                    mission_id=mission_id,
                    mission_title=mission_title,
                    existing=options,
                )
                options.append(selected)

            session["missions"] = [option.to_dict() for option in options]
            session["active_mission"] = selected.id
            session["revision"] = current_revision + 1
            acknowledgement = {
                "schema_version": 1,
                "correlation_id": correlation_id,
                "outcome": "acknowledged",
                "coding_workspace": workspace,
                "choice": choice,
                "active_mission": selected.id,
                "revision": session["revision"],
                "replayed": False,
                "missions": [option.to_public_dict() for option in options],
                "message": (
                    "Existing Mission resumed."
                    if choice == "resume"
                    else "New Mission created and selected."
                ),
            }
            receipts[correlation_id] = {
                "request": request,
                "acknowledgement": acknowledgement,
            }
            self._write_catalog_locked(session)
            self._write_payload_locked(payload)
            return dict(acknowledgement)

    @contextmanager
    def _locked(self) -> Iterator[None]:
        try:
            self.runtime_root.mkdir(parents=True, exist_ok=True)
            with self.lock_path.open("a+b") as lock_file:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
                try:
                    yield
                finally:
                    fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
        except OSError as error:
            raise MissionChoiceError(
                "runtime-state-lock-failed",
                "The Workspace journey state could not be locked.",
            ) from error

    def _load_payload_locked(self) -> dict[str, object]:
        if not self.state_path.exists():
            return {"schema_version": _STATE_SCHEMA_VERSION, "sessions": []}
        try:
            value = json.loads(self.state_path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as error:
            raise MissionChoiceError(
                "runtime-state-invalid",
                "The persisted Workspace journey state is invalid.",
                recoverable=False,
            ) from error
        if (
            not isinstance(value, dict)
            or value.get("schema_version") != _STATE_SCHEMA_VERSION
            or not isinstance(value.get("sessions"), list)
        ):
            raise MissionChoiceError(
                "runtime-state-invalid",
                "The persisted Workspace journey state is invalid.",
                recoverable=False,
            )
        return value

    def _write_payload_locked(self, payload: dict[str, object]) -> None:
        _write_json_atomically(self.state_path, payload)

    def _write_catalog_locked(self, session: dict[str, object]) -> None:
        catalog_path = Path(str(session["mission_catalog"]))
        catalog = {
            "schema_version": 1,
            "missions": [
                option.to_catalog_dict()
                for option in (MissionOption.from_dict(item) for item in session["missions"])
            ],
        }
        _write_json_atomically(catalog_path, catalog)

    @staticmethod
    def _find_session(
        sessions: list[object], *, starting_location: str, coding_workspace: str
    ) -> dict[str, object] | None:
        for item in sessions:
            if not isinstance(item, dict):
                raise MissionChoiceError(
                    "runtime-state-invalid",
                    "The persisted Workspace journey contains an invalid session.",
                    recoverable=False,
                )
            if (
                item.get("starting_location") == starting_location
                and item.get("coding_workspace") == coding_workspace
            ):
                return item
        return None

    def _require_session(
        self,
        sessions: list[object],
        *,
        starting_location: str,
        coding_workspace: str,
    ) -> dict[str, object]:
        session = self._find_session(
            sessions, starting_location=starting_location, coding_workspace=coding_workspace
        )
        if session is None:
            raise MissionChoiceError(
                "workspace-session-not-found",
                "A Coding Workspace must be acknowledged before choosing a Mission.",
            )
        return session

    @staticmethod
    def _validate_session(session: dict[str, object]) -> None:
        if (
            not isinstance(session.get("starting_location"), str)
            or not isinstance(session.get("coding_workspace"), str)
            or not isinstance(session.get("revision"), int)
            or session.get("revision", 0) < 1
            or session.get("active_mission") is not None
            and not isinstance(session.get("active_mission"), str)
            or not isinstance(session.get("missions"), list)
            or not isinstance(session.get("mission_catalog"), str)
            or not isinstance(session.get("receipts"), dict)
        ):
            raise MissionChoiceError(
                "runtime-state-invalid",
                "The persisted Workspace journey session is invalid.",
                recoverable=False,
            )
        try:
            for item in session["missions"]:
                MissionOption.from_dict(item)
        except (TypeError, ValueError) as error:
            raise MissionChoiceError(
                "runtime-state-invalid",
                "The persisted Workspace journey Mission catalog is invalid.",
                recoverable=False,
            ) from error

    @staticmethod
    def _public_state(session: dict[str, object]) -> dict[str, object]:
        options = [MissionOption.from_dict(item) for item in session["missions"]]
        active_mission = session["active_mission"]
        return {
            "schema_version": 1,
            "revision": session["revision"],
            "starting_location": session["starting_location"],
            "coding_workspace": session["coding_workspace"],
            "phase": "workspace-ready" if active_mission else "mission-choice-required",
            "active_mission": active_mission,
            "missions": [option.to_public_dict() for option in options],
            "mission_catalog": session["mission_catalog"],
        }

    def _catalog_path(self, starting_location: str, coding_workspace: str) -> Path:
        digest = sha256(f"{starting_location}\n{coding_workspace}".encode("utf-8")).hexdigest()
        return self.runtime_root / "workspace-mission-catalogs" / f"{digest}.json"

    @staticmethod
    def _create_new_mission(
        *,
        workspace: Path,
        mission_id: str,
        mission_title: str,
        existing: list[MissionOption],
    ) -> MissionOption:
        if not _MISSION_ID_PATTERN.fullmatch(mission_id):
            raise MissionChoiceError(
                "contract-failure",
                "A new Mission id must be 1-64 characters using letters, numbers, '.', '_' or '-'.",
            )
        if any(option.id == mission_id for option in existing):
            raise MissionChoiceError(
                "mission-duplicate",
                f"A Mission with this identity already exists: {mission_id}",
            )
        title = mission_title.strip()
        if not title or len(title) > _MISSION_TITLE_LIMIT:
            raise MissionChoiceError(
                "contract-failure",
                "A new Mission requires a non-empty title of at most 240 characters.",
            )
        tracker_dir = workspace / ".alfredo" / "missions" / mission_id
        issues_dir = tracker_dir / "issues"
        try:
            if tracker_dir.exists():
                raise FileExistsError(tracker_dir)
            tracker_dir.mkdir(parents=True)
            issues_dir.mkdir()
            (tracker_dir / "PRD.md").write_text(f"# {title}\n", encoding="utf-8")
        except FileExistsError as error:
            raise MissionChoiceError(
                "mission-duplicate",
                f"A Mission with this identity already exists: {mission_id}",
            ) from error
        except OSError as error:
            raise MissionChoiceError(
                "runtime-state-write-failed",
                "The new Mission could not be created.",
            ) from error
        return MissionOption(
            id=mission_id,
            title=title,
            tracker_dir=str(tracker_dir.resolve(strict=True)),
            issues_dir=str(issues_dir.resolve(strict=True)),
        )


def _read_prd_title(path: Path, *, fallback: str) -> str:
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith("#"):
                title = line.lstrip("#").strip()
                if title:
                    return title
    except (OSError, UnicodeError):
        pass
    return fallback


def _write_json_atomically(path: Path, payload: dict[str, object]) -> None:
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
        raise MissionChoiceError(
            "runtime-state-write-failed",
            "The Workspace journey state could not be persisted.",
        ) from error
    finally:
        if temporary_path is not None:
            try:
                temporary_path.unlink(missing_ok=True)
            except OSError:
                pass
