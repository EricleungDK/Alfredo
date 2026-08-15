from __future__ import annotations

import json
import os
import re
import shutil
import stat
import tempfile
from collections.abc import Callable
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path, PurePosixPath
from typing import Any


_RETAINED_WORKTREE_RECOVERY_METADATA_BYTES_LIMIT = 32 * 1024 * 1024
_GIT_WORKTREE_POINTER_BYTES_LIMIT = 4096


class RetirementSnapshotError(RuntimeError):
    """Raised when preservation cannot be proven without guessing."""


@dataclass(frozen=True)
class SnapshotRequest:
    mission_id: str
    session_id: str
    session_revision: int
    worktree_path: Path
    worktree_identity: str
    runtime_dir: Path
    target_repo: Path
    repository_snapshot: dict[str, Any]
    baseline_fingerprints: dict[str, str]
    evidence_correlation_id: str
    evidence_valid: bool
    artifacts: dict[str, str]
    terminal_status: str
    reserved_bytes: int


GitRunner = Callable[[Path, list[str], str | None, int], tuple[int, str, str]]


class RetirementSnapshotStore:
    """Capture and prove one immutable, app-local Retirement Snapshot."""

    schema_version = 1
    manifest_bytes_limit = 8 * 1024 * 1024

    def __init__(self, request: SnapshotRequest, run_git: GitRunner):
        self.request = request
        self.run_git = run_git
        self.payload_root = (
            request.runtime_dir / "retirement" / "payloads" / request.session_id
        )

    def capture(self) -> dict[str, Any]:
        retirement_root = self.request.runtime_dir / "retirement"
        temporary_root = retirement_root / "temporary"
        temporary_root.mkdir(parents=True, exist_ok=True)
        if self.payload_root.exists() or self.payload_root.is_symlink():
            raise RetirementSnapshotError(
                "Retirement Snapshot payload already exists; duplicate publication is blocked."
            )
        with tempfile.TemporaryDirectory(
            dir=temporary_root,
            prefix=f"{self.request.session_id}.",
        ) as temporary_name:
            temporary = Path(temporary_name)
            manifest = self._capture_into(temporary)
            manifest_path = temporary / "manifest.json"
            self._write_json(manifest_path, manifest)
            readback = self._read_manifest(manifest_path)
            if readback != manifest:
                raise RetirementSnapshotError(
                    "Retirement Snapshot manifest readback did not match publication bytes."
                )
            self._verify_payload_integrity(manifest_path, readback)
            self._verify_clean_room(manifest_path, readback)
            manifest["verification"] = {
                "manifest_readback": True,
                "clean_room_reconstruction": True,
            }
            self._write_final_manifest(manifest_path, manifest)
            readback = self._read_manifest(manifest_path)
            self._verify_payload_integrity(manifest_path, readback)
            manifest_sha256 = self._file_digest(manifest_path)
            self.payload_root.parent.mkdir(parents=True, exist_ok=True)
            Path(temporary_name).replace(self.payload_root)

        published_manifest = self.payload_root / "manifest.json"
        if self._file_digest(published_manifest) != manifest_sha256:
            raise RetirementSnapshotError(
                "Published Retirement Snapshot manifest failed integrity readback."
            )
        return {
            "schema_version": self.schema_version,
            "manifest_path": str(published_manifest),
            "manifest_sha256": manifest_sha256,
            "payload_path": str(self.payload_root),
            "payload_bytes": int(manifest["sizes"]["payload_bytes"]),
            "manifest_bytes": int(manifest["sizes"]["manifest_bytes"]),
            "snapshot_bytes": int(manifest["sizes"]["snapshot_bytes"]),
            "worktree_identity": self.request.worktree_identity,
            "session_revision": self.request.session_revision,
            "verified": True,
        }

    def verify(self, record: dict[str, Any]) -> bool:
        manifest_path = self._contained_payload_file(
            self.payload_root,
            PurePosixPath("manifest.json"),
        )
        if (
            record.get("schema_version") != self.schema_version
            or record.get("manifest_path") != str(manifest_path)
            or record.get("payload_path") != str(self.payload_root)
            or record.get("worktree_identity") != self.request.worktree_identity
            or record.get("session_revision") != self.request.session_revision
            or record.get("payload_bytes") is None
            or record.get("manifest_bytes") is None
            or record.get("snapshot_bytes") is None
            or record.get("verified") is not True
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot record does not match its exact unit boundary."
            )
        expected_manifest_digest = record.get("manifest_sha256")
        if (
            not isinstance(expected_manifest_digest, str)
            or self._file_digest(manifest_path) != expected_manifest_digest
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot manifest failed integrity validation."
            )
        manifest = self._read_manifest(manifest_path)
        if record.get("payload_bytes") != manifest.get("sizes", {}).get(
            "payload_bytes"
        ) or record.get("manifest_bytes") != manifest.get("sizes", {}).get(
            "manifest_bytes"
        ) or record.get("snapshot_bytes") != manifest.get("sizes", {}).get(
            "snapshot_bytes"
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot record size does not match its manifest."
            )
        self._validate_manifest_authority(manifest)
        if manifest.get("verification") != {
            "manifest_readback": True,
            "clean_room_reconstruction": True,
        }:
            raise RetirementSnapshotError(
                "Retirement Snapshot verification receipts are invalid."
            )
        self._verify_payload_integrity(manifest_path, manifest)
        self._verify_clean_room(manifest_path, manifest)
        return True

    def _verified_payload_root_manifest(
        self,
        record: dict[str, Any],
        *,
        expected_root_device: int | None = None,
        expected_root_inode: int | None = None,
    ) -> tuple[tuple[int, int], dict[str, Any]]:
        """Verify one payload while its parent and root identities stay bound."""

        parent_fd = self._open_directory_path(
            self.payload_root.parent,
            error="Snapshot Payload reclamation parent boundary is invalid.",
        )
        root_fd: int | None = None
        try:
            root_fd = self._open_directory_at(
                parent_fd,
                self.payload_root.name,
                error="Snapshot Payload reclamation boundary is invalid.",
            )
            before_status = os.fstat(root_fd)
            identity = self._stat_identity(before_status)
            if expected_root_device is not None or expected_root_inode is not None:
                if (
                    expected_root_device is None
                    or expected_root_inode is None
                    or identity != (expected_root_device, expected_root_inode)
                ):
                    raise RetirementSnapshotError(
                        "Snapshot Payload reclamation boundary changed."
                    )
            before_manifest = self.retained_worktree_manifest_from_directory(
                root_fd,
                exclude_git_metadata=False,
            )
            self.verify(record)
            try:
                named_status = os.stat(
                    self.payload_root.name,
                    dir_fd=parent_fd,
                    follow_symlinks=False,
                )
            except OSError as exc:
                raise RetirementSnapshotError(
                    "Snapshot Payload reclamation boundary changed."
                ) from exc
            after_status = os.fstat(root_fd)
            if (
                self._stat_identity(named_status) != identity
                or self._stat_identity(after_status) != identity
            ):
                raise RetirementSnapshotError(
                    "Snapshot Payload reclamation boundary changed."
                )
            after_manifest = self.retained_worktree_manifest_from_directory(
                root_fd,
                exclude_git_metadata=False,
            )
            final_status = os.fstat(root_fd)
            if (
                self._stat_identity(final_status) != identity
                or before_manifest != after_manifest
            ):
                raise RetirementSnapshotError(
                    "Snapshot Payload changed during reclamation proof."
                )
            return identity, after_manifest
        finally:
            if root_fd is not None:
                os.close(root_fd)
            os.close(parent_fd)

    def verified_payload_root_identity(
        self,
        record: dict[str, Any],
    ) -> tuple[int, int]:
        """Return one exact payload-root identity after descriptor-bound proof."""

        identity, _manifest = self._verified_payload_root_manifest(record)
        return identity

    def reclaim_verified_payload(
        self,
        record: dict[str, Any],
        *,
        expected_root_device: int,
        expected_root_inode: int,
    ) -> None:
        """Delete only one verified payload whose durable root identity still matches."""

        identity, manifest = self._verified_payload_root_manifest(
            record,
            expected_root_device=expected_root_device,
            expected_root_inode=expected_root_inode,
        )
        self.remove_retained_worktree(
            self.payload_root,
            manifest,
            expected_root_device=identity[0],
            expected_root_inode=identity[1],
        )
        if self.payload_root.exists() or self.payload_root.is_symlink():
            raise RetirementSnapshotError(
                "Snapshot Payload remained after reclamation."
            )

    def materialize(self, record: dict[str, Any], destination_root: Path) -> Path:
        """Reconstruct verified preserved material below an explicit empty root."""

        destination_fd = self._open_directory_path(
            destination_root,
            error="Retirement Snapshot reconstruction destination is invalid.",
        )
        try:
            self.materialize_into_directory(record, destination_fd)
        finally:
            os.close(destination_fd)
        return destination_root / "repository"

    def materialize_into_directory(
        self,
        record: dict[str, Any],
        destination_fd: int,
    ) -> None:
        """Reconstruct one snapshot below an exact borrowed directory descriptor."""

        self.verify(record)
        root_fd = self._duplicate_directory_fd(
            destination_fd,
            error="Retirement Snapshot reconstruction destination is invalid.",
        )
        try:
            self._require_empty_directory_fd(
                root_fd,
                error=(
                    "Retirement Snapshot reconstruction destination is not an empty "
                    "directory."
                ),
            )
            manifest_path = self._contained_payload_file(
                self.payload_root,
                PurePosixPath("manifest.json"),
            )
            manifest = self._read_manifest(manifest_path)
            materialization_root = (
                self.request.runtime_dir / "retirement" / "export-materialization"
            )
            materialization_root.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(
                dir=materialization_root,
                prefix=f"{self.request.session_id}.",
            ) as temporary_name:
                temporary = Path(temporary_name)
                self._materialize_manifest_to_trusted_root(
                    manifest,
                    temporary,
                )
                source_repository_fd = self._open_directory_path(
                    temporary / "repository",
                    error="Retirement Snapshot private reconstruction is invalid.",
                )
                try:
                    self._copy_repository_into_directory(
                        source_repository_fd,
                        root_fd,
                    )
                finally:
                    os.close(source_repository_fd)
            if self._directory_names_fd(root_fd) != ("repository",):
                raise RetirementSnapshotError(
                    "Retirement Snapshot reconstruction destination changed."
                )
        finally:
            os.close(root_fd)

    def verify_materialized_repository(
        self,
        record: dict[str, Any],
        repository: Path,
    ) -> bool:
        """Prove a crash-left export is exactly the retained Snapshot Payload."""

        repository_fd = self._open_directory_path(
            repository,
            error="Retirement Snapshot exported repository boundary is invalid.",
        )
        try:
            return self.verify_materialized_repository_in_directory(
                record,
                repository_fd,
            )
        finally:
            os.close(repository_fd)

    def verify_materialized_repository_in_directory(
        self,
        record: dict[str, Any],
        repository_fd: int,
    ) -> bool:
        """Prove one exported repository through an exact borrowed descriptor."""

        self.verify(record)
        source_fd = self._duplicate_directory_fd(
            repository_fd,
            error="Retirement Snapshot exported repository boundary is invalid.",
        )
        try:
            manifest_path = self._contained_payload_file(
                self.payload_root,
                PurePosixPath("manifest.json"),
            )
            manifest = self._read_manifest(manifest_path)
            verification_root = (
                self.request.runtime_dir / "retirement" / "export-check"
            )
            verification_root.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(
                dir=verification_root,
                prefix=f"{self.request.session_id}.actual.",
            ) as actual_name:
                actual_root = Path(actual_name)
                actual_root_fd = self._open_directory_path(
                    actual_root,
                    error="Retirement Snapshot private verification is invalid.",
                )
                try:
                    self._copy_repository_into_directory(source_fd, actual_root_fd)
                finally:
                    os.close(actual_root_fd)
                actual = actual_root / "repository"
                self._verify_materialized_repository_path(
                    manifest,
                    actual,
                )
            return True
        finally:
            os.close(source_fd)

    @staticmethod
    def _is_git_worktree_pointer_content(content: bytes) -> bool:
        """Recognize only one bounded Git administration-pointer line."""

        if not content or len(content) > _GIT_WORKTREE_POINTER_BYTES_LIMIT:
            return False
        line = content[:-1] if content.endswith(b"\n") else content
        if line.endswith(b"\r"):
            line = line[:-1]
        prefix = b"gitdir: "
        return (
            line.startswith(prefix)
            and bool(line[len(prefix) :])
            and b"\0" not in line
            and b"\n" not in line
            and b"\r" not in line
        )

    @classmethod
    def _bounded_git_worktree_pointer(
        cls,
        path: Path,
    ) -> dict[str, Any] | None:
        """Read one stable no-follow Git pointer, or classify it as user data."""

        try:
            before = path.stat(follow_symlinks=False)
        except FileNotFoundError:
            return None
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree Git metadata could not be inspected."
            ) from exc
        if (
            not stat.S_ISREG(before.st_mode)
            or before.st_size > _GIT_WORKTREE_POINTER_BYTES_LIMIT
        ):
            return None
        try:
            descriptor = os.open(
                path,
                os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0),
            )
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree Git metadata could not be inspected."
            ) from exc
        try:
            after = os.fstat(descriptor)
            if (
                not stat.S_ISREG(after.st_mode)
                or (after.st_dev, after.st_ino, after.st_size)
                != (before.st_dev, before.st_ino, before.st_size)
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree Git metadata changed during inspection."
                )
            with os.fdopen(descriptor, "rb", closefd=False) as handle:
                content = handle.read(_GIT_WORKTREE_POINTER_BYTES_LIMIT + 1)
        finally:
            os.close(descriptor)
        if len(content) != before.st_size:
            raise RetirementSnapshotError(
                "Retained Worktree Git metadata changed during inspection."
            )
        if not cls._is_git_worktree_pointer_content(content):
            return None
        return {"mode": before.st_mode & 0o777, "content": content}

    @classmethod
    def _bounded_git_worktree_pointer_at(
        cls,
        parent_fd: int,
        name: str,
        initial: os.stat_result,
    ) -> dict[str, Any] | None:
        """Read one stable Git pointer relative to an exact parent descriptor."""

        if (
            not stat.S_ISREG(initial.st_mode)
            or initial.st_size > _GIT_WORKTREE_POINTER_BYTES_LIMIT
        ):
            return None
        flags = (
            os.O_RDONLY
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
            | getattr(os, "O_NONBLOCK", 0)
        )
        try:
            descriptor = os.open(name, flags, dir_fd=parent_fd)
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree Git metadata could not be inspected."
            ) from exc
        try:
            before = os.fstat(descriptor)
            if cls._stable_stat(before) != cls._stable_stat(initial):
                raise RetirementSnapshotError(
                    "Retained Worktree Git metadata changed during inspection."
                )
            content = bytearray()
            while len(content) <= _GIT_WORKTREE_POINTER_BYTES_LIMIT:
                chunk = os.read(descriptor, _GIT_WORKTREE_POINTER_BYTES_LIMIT + 1)
                if not chunk:
                    break
                content.extend(chunk)
            after = os.fstat(descriptor)
            if (
                cls._stable_stat(after) != cls._stable_stat(before)
                or len(content) != before.st_size
                or len(content) > _GIT_WORKTREE_POINTER_BYTES_LIMIT
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree Git metadata changed during inspection."
                )
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree Git metadata could not be inspected."
            ) from exc
        finally:
            os.close(descriptor)
        payload = bytes(content)
        if not cls._is_git_worktree_pointer_content(payload):
            return None
        return {"mode": before.st_mode & 0o777, "content": payload}

    @staticmethod
    def _retained_regular_file_record(path: Path) -> dict[str, Any]:
        """Fingerprint one stable no-follow regular-file descriptor."""

        try:
            initial = path.stat(follow_symlinks=False)
            if not stat.S_ISREG(initial.st_mode):
                raise RetirementSnapshotError(
                    "Retained Worktree export contains an unsupported entry."
                )
            descriptor = os.open(
                path,
                os.O_RDONLY
                | getattr(os, "O_NOFOLLOW", 0)
                | getattr(os, "O_NONBLOCK", 0),
            )
        except RetirementSnapshotError:
            raise
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree file changed during inspection."
            ) from exc
        try:
            before = os.fstat(descriptor)
            if not stat.S_ISREG(before.st_mode) or (
                initial.st_dev,
                initial.st_ino,
                initial.st_mode,
                initial.st_size,
                initial.st_mtime_ns,
                initial.st_ctime_ns,
            ) != (
                before.st_dev,
                before.st_ino,
                before.st_mode,
                before.st_size,
                before.st_mtime_ns,
                before.st_ctime_ns,
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree file changed during inspection."
                )
            digest = sha256()
            while chunk := os.read(descriptor, 1024 * 1024):
                digest.update(chunk)
            after = os.fstat(descriptor)
            if (
                before.st_dev,
                before.st_ino,
                before.st_mode,
                before.st_size,
                before.st_mtime_ns,
                before.st_ctime_ns,
            ) != (
                after.st_dev,
                after.st_ino,
                after.st_mode,
                after.st_size,
                after.st_mtime_ns,
                after.st_ctime_ns,
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree file changed during inspection."
                )
            return {
                "kind": "file",
                "mode": after.st_mode & 0o777,
                "size": after.st_size,
                "sha256": digest.hexdigest(),
            }
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree file changed during inspection."
            ) from exc
        finally:
            os.close(descriptor)

    @classmethod
    def _retained_regular_file_record_at(
        cls,
        parent_fd: int,
        name: str,
        initial: os.stat_result,
    ) -> dict[str, Any]:
        """Fingerprint one regular file relative to an exact parent descriptor."""

        flags = (
            os.O_RDONLY
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
            | getattr(os, "O_NONBLOCK", 0)
        )
        try:
            descriptor = os.open(name, flags, dir_fd=parent_fd)
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree file changed during inspection."
            ) from exc
        try:
            before = os.fstat(descriptor)
            if (
                not stat.S_ISREG(before.st_mode)
                or cls._stable_stat(before) != cls._stable_stat(initial)
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree file changed during inspection."
                )
            digest = cls._file_descriptor_digest(descriptor)
            after = os.fstat(descriptor)
            if cls._stable_stat(after) != cls._stable_stat(before):
                raise RetirementSnapshotError(
                    "Retained Worktree file changed during inspection."
                )
            return {
                "kind": "file",
                "mode": after.st_mode & 0o777,
                "size": after.st_size,
                "sha256": digest,
            }
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree file changed during inspection."
            ) from exc
        finally:
            os.close(descriptor)

    @classmethod
    def _retained_worktree_digest(
        cls,
        *,
        root_mode: int,
        entries: dict[str, dict[str, Any]],
    ) -> str:
        payload = {
            "schema_version": 2,
            "root_mode": root_mode,
            "entries": entries,
        }
        return sha256(
            json.dumps(
                payload,
                ensure_ascii=True,
                sort_keys=True,
                separators=(",", ":"),
            ).encode("ascii")
        ).hexdigest()

    @classmethod
    def _enforce_retained_worktree_manifest_bound(
        cls,
        manifest: dict[str, Any],
    ) -> None:
        encoded_bytes = 0
        encoder = json.JSONEncoder(
            ensure_ascii=True,
            sort_keys=True,
            separators=(",", ":"),
        )
        try:
            for chunk in encoder.iterencode(manifest):
                encoded_bytes += len(chunk.encode("ascii"))
                if encoded_bytes > _RETAINED_WORKTREE_RECOVERY_METADATA_BYTES_LIMIT:
                    raise RetirementSnapshotError(
                        "Retained Worktree recovery metadata exceeds 32 MiB."
                    )
        except (TypeError, ValueError, UnicodeError) as exc:
            raise RetirementSnapshotError(
                "Retained Worktree recovery manifest is invalid."
            ) from exc

    @classmethod
    def _build_retained_worktree_manifest(
        cls,
        *,
        root_mode: int,
        entries: dict[str, dict[str, Any]],
    ) -> dict[str, Any]:
        materialized_entries = {
            relative_value: record
            for relative_value, record in entries.items()
            if record.get("kind") != "git-pointer"
        }
        manifest = {
            "schema_version": 2,
            "root_mode": root_mode,
            "entries": entries,
            "tree_sha256": cls._retained_worktree_digest(
                root_mode=root_mode,
                entries=entries,
            ),
            "materialized_tree_sha256": cls._retained_worktree_digest(
                root_mode=root_mode,
                entries=materialized_entries,
            ),
        }
        cls._enforce_retained_worktree_manifest_bound(manifest)
        return manifest

    @staticmethod
    def _raise_incomplete_tree_walk(error: OSError) -> None:
        raise RetirementSnapshotError(
            "Retained Worktree recovery scan could not inspect the full tree."
        ) from error

    @classmethod
    def retained_worktree_manifest(
        cls,
        worktree: Path,
        *,
        exclude_git_metadata: bool,
    ) -> dict[str, Any]:
        """Fingerprint an exact retained filesystem tree without following links."""

        descriptor = cls._open_directory_path(
            worktree,
            error="Retained Worktree export source boundary is invalid.",
        )
        try:
            return cls.retained_worktree_manifest_from_directory(
                descriptor,
                exclude_git_metadata=exclude_git_metadata,
            )
        finally:
            os.close(descriptor)

    @classmethod
    def retained_worktree_manifest_from_directory(
        cls,
        worktree_fd: int,
        *,
        exclude_git_metadata: bool,
    ) -> dict[str, Any]:
        """Fingerprint one tree through an exact borrowed root descriptor."""

        root_fd = cls._duplicate_directory_fd(
            worktree_fd,
            error="Retained Worktree export source boundary is invalid.",
        )
        entries: dict[str, dict[str, Any]] = {}
        metadata_bytes = 0

        def add_entry(relative: str, record: dict[str, Any]) -> None:
            nonlocal metadata_bytes
            entries[relative] = record
            metadata_bytes += len(
                json.dumps(relative, ensure_ascii=True).encode("ascii")
            ) + len(
                json.dumps(
                    record,
                    ensure_ascii=True,
                    sort_keys=True,
                    separators=(",", ":"),
                ).encode("ascii")
            )
            if metadata_bytes > _RETAINED_WORKTREE_RECOVERY_METADATA_BYTES_LIMIT:
                raise RetirementSnapshotError(
                    "Retained Worktree recovery metadata exceeds 32 MiB."
                )

        def scan(directory_fd: int, parent_parts: tuple[str, ...]) -> None:
            before = os.fstat(directory_fd)
            if not stat.S_ISDIR(before.st_mode):
                raise RetirementSnapshotError(
                    "Retained Worktree export source boundary is invalid."
                )
            names = cls._directory_names_fd(directory_fd)
            for name in names:
                try:
                    initial = os.stat(
                        name,
                        dir_fd=directory_fd,
                        follow_symlinks=False,
                    )
                except OSError as exc:
                    raise RetirementSnapshotError(
                        "Retained Worktree recovery scan could not inspect the full tree."
                    ) from exc
                relative = PurePosixPath(*parent_parts, name).as_posix()
                if stat.S_ISLNK(initial.st_mode):
                    target = cls._read_stable_symlink_at(
                        directory_fd,
                        name,
                        initial,
                        error="Retained Worktree changed during inspection.",
                    )
                    add_entry(
                        relative,
                        {"kind": "symlink", "target": target.hex()},
                    )
                    continue
                if stat.S_ISDIR(initial.st_mode):
                    add_entry(
                        relative,
                        {"kind": "directory", "mode": initial.st_mode & 0o777},
                    )
                    child_fd = cls._open_directory_at(
                        directory_fd,
                        name,
                        error=(
                            "Retained Worktree recovery scan could not inspect the "
                            "full tree."
                        ),
                    )
                    try:
                        current = os.fstat(child_fd)
                        if cls._stat_identity(current) != cls._stat_identity(initial):
                            raise RetirementSnapshotError(
                                "Retained Worktree changed during inspection."
                            )
                        scan(child_fd, (*parent_parts, name))
                        after = os.fstat(child_fd)
                        if cls._stable_stat(after) != cls._stable_stat(current):
                            raise RetirementSnapshotError(
                                "Retained Worktree changed during inspection."
                            )
                    finally:
                        os.close(child_fd)
                    continue
                if not stat.S_ISREG(initial.st_mode):
                    raise RetirementSnapshotError(
                        "Retained Worktree export contains an unsupported entry."
                    )
                git_pointer = None
                if exclude_git_metadata and not parent_parts and name == ".git":
                    git_pointer = cls._bounded_git_worktree_pointer_at(
                        directory_fd,
                        name,
                        initial,
                    )
                if git_pointer is not None:
                    add_entry(
                        relative,
                        {
                            "kind": "git-pointer",
                            "mode": git_pointer["mode"],
                            "content_hex": git_pointer["content"].hex(),
                        },
                    )
                else:
                    add_entry(
                        relative,
                        cls._retained_regular_file_record_at(
                            directory_fd,
                            name,
                            initial,
                        ),
                    )
            if cls._directory_names_fd(directory_fd) != names:
                raise RetirementSnapshotError(
                    "Retained Worktree changed during inspection."
                )
            after = os.fstat(directory_fd)
            if cls._stable_stat(after) != cls._stable_stat(before):
                raise RetirementSnapshotError(
                    "Retained Worktree changed during inspection."
                )

        try:
            root_status = os.fstat(root_fd)
            scan(root_fd, ())
            return cls._build_retained_worktree_manifest(
                root_mode=root_status.st_mode & 0o777,
                entries=entries,
            )
        except RetirementSnapshotError:
            raise
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree recovery scan could not inspect the full tree."
            ) from exc
        finally:
            os.close(root_fd)

    @classmethod
    def validated_retained_worktree_manifest(
        cls,
        raw: Any,
    ) -> dict[str, Any]:
        """Validate one durable no-follow recovery manifest."""

        if (
            not isinstance(raw, dict)
            or set(raw)
            != {
                "schema_version",
                "root_mode",
                "entries",
                "tree_sha256",
                "materialized_tree_sha256",
            }
            or not isinstance(raw.get("schema_version"), int)
            or isinstance(raw.get("schema_version"), bool)
            or raw["schema_version"] != 2
            or not isinstance(raw.get("root_mode"), int)
            or isinstance(raw.get("root_mode"), bool)
            or not 0 <= raw["root_mode"] <= 0o777
            or not isinstance(raw.get("entries"), dict)
            or not isinstance(raw.get("tree_sha256"), str)
            or not re.fullmatch(r"[0-9a-f]{64}", raw["tree_sha256"])
            or not isinstance(raw.get("materialized_tree_sha256"), str)
            or not re.fullmatch(
                r"[0-9a-f]{64}", raw["materialized_tree_sha256"]
            )
        ):
            raise RetirementSnapshotError(
                "Retained Worktree recovery manifest is invalid."
            )
        entries = raw["entries"]
        metadata_bytes = 0
        for relative_value, record in entries.items():
            if not isinstance(relative_value, str) or not isinstance(record, dict):
                raise RetirementSnapshotError(
                    "Retained Worktree recovery manifest is invalid."
                )
            relative = cls._safe_retained_relative(relative_value)
            kind = record.get("kind")
            if kind == "directory":
                valid = (
                    set(record) == {"kind", "mode"}
                    and isinstance(record.get("mode"), int)
                    and not isinstance(record.get("mode"), bool)
                    and 0 <= record["mode"] <= 0o777
                )
            elif kind == "file":
                valid = (
                    set(record) == {"kind", "mode", "size", "sha256"}
                    and isinstance(record.get("mode"), int)
                    and not isinstance(record.get("mode"), bool)
                    and 0 <= record["mode"] <= 0o777
                    and isinstance(record.get("size"), int)
                    and not isinstance(record.get("size"), bool)
                    and record["size"] >= 0
                    and isinstance(record.get("sha256"), str)
                    and re.fullmatch(r"[0-9a-f]{64}", record["sha256"])
                    is not None
                )
            elif kind == "symlink":
                target = record.get("target")
                valid = (
                    set(record) == {"kind", "target"}
                    and isinstance(target, str)
                    and len(target) % 2 == 0
                    and re.fullmatch(r"[0-9a-f]*", target) is not None
                )
                if valid:
                    try:
                        decoded_target = bytes.fromhex(target)
                    except ValueError:
                        valid = False
                    else:
                        valid = b"\0" not in decoded_target
            elif kind == "git-pointer":
                content_hex = record.get("content_hex")
                valid = (
                    relative_value == ".git"
                    and set(record) == {"kind", "mode", "content_hex"}
                    and isinstance(record.get("mode"), int)
                    and not isinstance(record.get("mode"), bool)
                    and 0 <= record["mode"] <= 0o777
                    and isinstance(content_hex, str)
                    and len(content_hex) % 2 == 0
                    and re.fullmatch(r"[0-9a-f]*", content_hex) is not None
                )
                if valid:
                    try:
                        content = bytes.fromhex(content_hex)
                    except ValueError:
                        valid = False
                    else:
                        valid = cls._is_git_worktree_pointer_content(content)
            else:
                valid = False
            if not valid:
                raise RetirementSnapshotError(
                    "Retained Worktree recovery manifest is invalid."
                )
            metadata_bytes += len(
                json.dumps(relative_value, ensure_ascii=True).encode("ascii")
            ) + len(
                json.dumps(
                    record,
                    ensure_ascii=True,
                    sort_keys=True,
                    separators=(",", ":"),
                ).encode("ascii")
            )
            if (
                metadata_bytes
                > _RETAINED_WORKTREE_RECOVERY_METADATA_BYTES_LIMIT
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree recovery metadata exceeds 32 MiB."
                )
        for relative_value, record in entries.items():
            relative = PurePosixPath(relative_value)
            for parent in relative.parents:
                if parent == PurePosixPath("."):
                    continue
                parent_record = entries.get(parent.as_posix())
                if not isinstance(parent_record, dict) or parent_record.get(
                    "kind"
                ) != "directory":
                    raise RetirementSnapshotError(
                        "Retained Worktree recovery manifest is invalid."
                    )
        validated = cls._build_retained_worktree_manifest(
            root_mode=raw["root_mode"],
            entries={
                relative_value: dict(record)
                for relative_value, record in entries.items()
            },
        )
        if (
            validated["tree_sha256"] != raw["tree_sha256"]
            or validated["materialized_tree_sha256"]
            != raw["materialized_tree_sha256"]
        ):
            raise RetirementSnapshotError(
                "Retained Worktree recovery manifest is invalid."
            )
        return validated

    @classmethod
    def legacy_retained_worktree_manifest_projection(
        cls,
        manifest: dict[str, Any],
        *,
        exclude_git_metadata: bool,
    ) -> dict[str, Any]:
        """Project a schema-v2 tree into the exact pre-upgrade digest shape."""

        validated = cls.validated_retained_worktree_manifest(manifest)
        entries = {
            relative_value: dict(record)
            for relative_value, record in validated["entries"].items()
            if record.get("kind") != "git-pointer"
            and not (
                exclude_git_metadata
                and (
                    relative_value == ".git"
                    or relative_value.startswith(".git/")
                )
            )
        }
        tree_sha256 = sha256(
            json.dumps(
                entries,
                ensure_ascii=True,
                sort_keys=True,
                separators=(",", ":"),
            ).encode("ascii")
        ).hexdigest()
        return {
            "schema_version": 1,
            "entries": entries,
            "tree_sha256": tree_sha256,
        }

    @classmethod
    def materialize_retained_worktree(
        cls,
        worktree: Path,
        destination_root: Path,
        manifest: dict[str, Any],
        *,
        exclude_git_metadata: bool,
    ) -> Path:
        """Copy and re-prove one blocked retained tree below an empty root."""

        source_fd = cls._open_directory_path(
            worktree,
            error="Retained Worktree export source boundary is invalid.",
        )
        destination_fd: int | None = None
        try:
            destination_fd = cls._open_directory_path(
                destination_root,
                error="Retained Worktree export destination boundary is invalid.",
            )
            cls.materialize_retained_worktree_into_directory(
                source_fd,
                destination_fd,
                manifest,
                exclude_git_metadata=exclude_git_metadata,
            )
        finally:
            if destination_fd is not None:
                os.close(destination_fd)
            os.close(source_fd)
        return destination_root / "repository"

    @classmethod
    def materialize_retained_worktree_into_directory(
        cls,
        source_fd: int | Path,
        destination_fd: int,
        manifest: dict[str, Any],
        *,
        exclude_git_metadata: bool,
    ) -> None:
        """Copy one retained tree between exact borrowed directory descriptors.

        ``Path`` remains accepted for the source so callers of the historical path
        API can migrate independently.  It is opened once and all subsequent reads
        are relative to that bound descriptor.
        """

        manifest = cls.validated_retained_worktree_manifest(manifest)
        if isinstance(source_fd, Path):
            retained_fd = cls._open_directory_path(
                source_fd,
                error="Retained Worktree export source boundary is invalid.",
            )
        else:
            retained_fd = cls._duplicate_directory_fd(
                source_fd,
                error="Retained Worktree export source boundary is invalid.",
            )
        root_fd: int | None = None
        repository_fd: int | None = None
        try:
            root_fd = cls._duplicate_directory_fd(
                destination_fd,
                error="Retained Worktree export destination boundary is invalid.",
            )
            if (
                cls.retained_worktree_manifest_from_directory(
                    retained_fd,
                    exclude_git_metadata=exclude_git_metadata,
                )
                != manifest
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree changed before export materialization."
                )
            cls._require_empty_directory_fd(
                root_fd,
                error=(
                    "Retained Worktree export destination is not an empty directory."
                ),
            )
            os.mkdir("repository", mode=0o700, dir_fd=root_fd)
            repository_fd = cls._open_directory_at(
                root_fd,
                "repository",
                error="Retained Worktree export destination boundary changed.",
            )
            directories = sorted(
                (
                    (relative, record)
                    for relative, record in manifest["entries"].items()
                    if record.get("kind") == "directory"
                ),
                key=lambda item: (len(PurePosixPath(item[0]).parts), item[0]),
            )
            for relative_value, _record in directories:
                relative = cls._safe_retained_relative(relative_value)
                parent_fd, name = cls._open_relative_parent_at(
                    repository_fd,
                    relative,
                    error="Retained Worktree export destination boundary changed.",
                )
                try:
                    os.mkdir(name, mode=0o700, dir_fd=parent_fd)
                    child_fd = cls._open_directory_at(
                        parent_fd,
                        name,
                        error=(
                            "Retained Worktree export destination boundary changed."
                        ),
                    )
                    try:
                        os.fchmod(child_fd, 0o700)
                    finally:
                        os.close(child_fd)
                finally:
                    os.close(parent_fd)
            for relative_value, record in sorted(manifest["entries"].items()):
                kind = record["kind"]
                if kind in {"directory", "git-pointer"}:
                    continue
                relative = cls._safe_retained_relative(relative_value)
                source_parent_fd, source_name = cls._open_relative_parent_at(
                    retained_fd,
                    relative,
                    error="Retained Worktree changed during export materialization.",
                )
                destination_parent_fd, destination_name = (
                    cls._open_relative_parent_at(
                        repository_fd,
                        relative,
                        error=(
                            "Retained Worktree export destination boundary changed."
                        ),
                    )
                )
                try:
                    if kind == "symlink":
                        cls._copy_expected_symlink_at(
                            source_parent_fd,
                            source_name,
                            destination_parent_fd,
                            destination_name,
                            bytes.fromhex(record["target"]),
                        )
                    elif kind == "file":
                        cls._copy_expected_regular_at(
                            source_parent_fd,
                            source_name,
                            destination_parent_fd,
                            destination_name,
                            record,
                        )
                    else:
                        raise RetirementSnapshotError(
                            "Retained Worktree export manifest is invalid."
                        )
                finally:
                    os.close(destination_parent_fd)
                    os.close(source_parent_fd)
            for relative_value, record in reversed(directories):
                relative = cls._safe_retained_relative(relative_value)
                parent_fd, name = cls._open_relative_parent_at(
                    repository_fd,
                    relative,
                    error="Retained Worktree export destination boundary changed.",
                )
                try:
                    child_fd = cls._open_directory_at(
                        parent_fd,
                        name,
                        error=(
                            "Retained Worktree export destination boundary changed."
                        ),
                    )
                    try:
                        os.fchmod(child_fd, record["mode"])
                        os.fsync(child_fd)
                    finally:
                        os.close(child_fd)
                finally:
                    os.close(parent_fd)
            os.fchmod(repository_fd, manifest["root_mode"])
            os.fsync(repository_fd)
            if (
                cls.retained_worktree_manifest_from_directory(
                    retained_fd,
                    exclude_git_metadata=exclude_git_metadata,
                )
                != manifest
                or not cls.verify_retained_worktree_in_directory(
                    repository_fd,
                    manifest,
                )
                or cls._directory_names_fd(root_fd) != ("repository",)
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree export failed exact readback verification."
                )
        except OSError as exc:
            raise RetirementSnapshotError(
                f"Retained Worktree export materialization failed: {exc}"
            ) from exc
        finally:
            if repository_fd is not None:
                os.close(repository_fd)
            if root_fd is not None:
                os.close(root_fd)
            os.close(retained_fd)

    @classmethod
    def verify_retained_worktree_in_directory(
        cls,
        repository_fd: int,
        manifest: dict[str, Any],
    ) -> bool:
        """Verify a materialized retained tree through a borrowed root descriptor."""

        manifest = cls.validated_retained_worktree_manifest(manifest)
        actual = cls.retained_worktree_manifest_from_directory(
            repository_fd,
            exclude_git_metadata=False,
        )
        expected_entries = {
            relative_value: dict(record)
            for relative_value, record in manifest["entries"].items()
            if record.get("kind") != "git-pointer"
        }
        expected = cls._build_retained_worktree_manifest(
            root_mode=manifest["root_mode"],
            entries=expected_entries,
        )
        if actual != expected:
            raise RetirementSnapshotError(
                "Retained Worktree exported repository content is invalid."
            )
        return True

    @classmethod
    def prepare_retained_worktree_removal(
        cls,
        worktree: Path,
        manifest: dict[str, Any],
    ) -> None:
        """Make only claimed directories owner-writable for resumable deletion."""

        manifest = cls.validated_retained_worktree_manifest(manifest)
        directories: list[tuple[Path, int]] = [(worktree, manifest["root_mode"])]
        directories.extend(
            (
                worktree
                / Path(*cls._safe_retained_relative(relative_value).parts),
                record["mode"],
            )
            for relative_value, record in sorted(
                manifest["entries"].items(),
                key=lambda item: (
                    len(PurePosixPath(item[0]).parts),
                    item[0],
                ),
            )
            if record.get("kind") == "directory"
        )
        for directory, claimed_mode in directories:
            try:
                descriptor = os.open(
                    directory,
                    os.O_RDONLY
                    | getattr(os, "O_DIRECTORY", 0)
                    | getattr(os, "O_NOFOLLOW", 0),
                )
            except FileNotFoundError:
                if directory == worktree:
                    raise RetirementSnapshotError(
                        "Retained Worktree removal boundary disappeared."
                    )
                continue
            except OSError as exc:
                raise RetirementSnapshotError(
                    "Retained Worktree removal directory changed."
                ) from exc
            try:
                status = os.fstat(descriptor)
                if not stat.S_ISDIR(status.st_mode):
                    raise RetirementSnapshotError(
                        "Retained Worktree removal directory changed."
                    )
                current_mode = status.st_mode & 0o777
                prepared_mode = claimed_mode | 0o700
                if current_mode not in {claimed_mode, prepared_mode}:
                    raise RetirementSnapshotError(
                        "Retained Worktree removal directory mode changed."
                    )
                if hasattr(os, "fchmod"):
                    os.fchmod(descriptor, prepared_mode)
                else:
                    os.chmod(directory, prepared_mode, follow_symlinks=False)
            finally:
                os.close(descriptor)

    @classmethod
    def remove_retained_worktree(
        cls,
        worktree: Path,
        manifest: dict[str, Any],
        *,
        expected_root_device: int,
        expected_root_inode: int,
    ) -> None:
        """Remove only entries below one descriptor-bound retained-tree claim."""

        manifest = cls.validated_retained_worktree_manifest(manifest)
        if (
            not isinstance(expected_root_device, int)
            or isinstance(expected_root_device, bool)
            or expected_root_device < 0
            or not isinstance(expected_root_inode, int)
            or isinstance(expected_root_inode, bool)
            or expected_root_inode <= 0
        ):
            raise RetirementSnapshotError(
                "Retained Worktree removal boundary is invalid."
            )
        children: dict[tuple[str, ...], dict[str, dict[str, Any]]] = {}
        for relative_value, record in manifest["entries"].items():
            relative = cls._safe_retained_relative(relative_value)
            children.setdefault(tuple(relative.parts[:-1]), {})[
                relative.parts[-1]
            ] = record

        def stat_entry(parent_fd: int, name: str) -> os.stat_result:
            try:
                return os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
            except OSError as exc:
                raise RetirementSnapshotError(
                    "Retained Worktree removal content changed."
                ) from exc

        def validate_entry(
            parent_fd: int,
            name: str,
            record: dict[str, Any],
            initial: os.stat_result,
        ) -> None:
            kind = record["kind"]
            if kind == "directory":
                if not stat.S_ISDIR(initial.st_mode) or (
                    initial.st_mode & 0o777
                ) not in {record["mode"], record["mode"] | 0o700}:
                    raise RetirementSnapshotError(
                        "Retained Worktree removal content changed."
                    )
                return
            if kind == "symlink":
                target = cls._read_stable_symlink_at(
                    parent_fd,
                    name,
                    initial,
                    error="Retained Worktree removal content changed.",
                )
                if target.hex() != record["target"]:
                    raise RetirementSnapshotError(
                        "Retained Worktree removal content changed."
                    )
                return
            if kind == "git-pointer":
                pointer = cls._bounded_git_worktree_pointer_at(
                    parent_fd,
                    name,
                    initial,
                )
                if (
                    pointer is None
                    or pointer["mode"] != record["mode"]
                    or pointer["content"].hex() != record["content_hex"]
                ):
                    raise RetirementSnapshotError(
                        "Retained Worktree removal content changed."
                    )
                return
            if kind == "file":
                if cls._retained_regular_file_record_at(
                    parent_fd,
                    name,
                    initial,
                ) != record:
                    raise RetirementSnapshotError(
                        "Retained Worktree removal content changed."
                    )
                return
            raise RetirementSnapshotError(
                "Retained Worktree removal content changed."
            )

        def unlink_verified_entry(
            parent_fd: int,
            name: str,
            record: dict[str, Any],
            expected_identity: tuple[int, int],
        ) -> None:
            current = stat_entry(parent_fd, name)
            if cls._stat_identity(current) != expected_identity:
                raise RetirementSnapshotError(
                    "Retained Worktree removal content changed."
                )
            validate_entry(parent_fd, name, record, current)
            latest = stat_entry(parent_fd, name)
            if cls._stable_stat(latest) != cls._stable_stat(current):
                raise RetirementSnapshotError(
                    "Retained Worktree removal content changed."
                )
            try:
                os.unlink(name, dir_fd=parent_fd)
            except OSError as exc:
                raise RetirementSnapshotError(
                    "Retained Worktree removal content changed."
                ) from exc

        def remove_directory(
            directory_fd: int,
            relative_parts: tuple[str, ...],
            claimed_mode: int,
        ) -> None:
            directory_status = os.fstat(directory_fd)
            if not stat.S_ISDIR(directory_status.st_mode) or (
                directory_status.st_mode & 0o777
            ) not in {claimed_mode, claimed_mode | 0o700}:
                raise RetirementSnapshotError(
                    "Retained Worktree removal directory changed."
                )
            try:
                os.fchmod(directory_fd, claimed_mode | 0o700)
            except OSError as exc:
                raise RetirementSnapshotError(
                    "Retained Worktree removal directory changed."
                ) from exc
            expected = children.get(relative_parts, {})
            current_names = cls._directory_names_fd(directory_fd)
            for name in current_names:
                record = expected.get(name)
                if record is None:
                    raise RetirementSnapshotError(
                        "Retained Worktree removal found unauthorized content."
                    )
                initial = stat_entry(directory_fd, name)
                validate_entry(directory_fd, name, record, initial)
                if record["kind"] == "directory":
                    child_fd = cls._open_directory_at(
                        directory_fd,
                        name,
                        error="Retained Worktree removal content changed.",
                    )
                    try:
                        child_status = os.fstat(child_fd)
                        if cls._stat_identity(child_status) != cls._stat_identity(
                            initial
                        ):
                            raise RetirementSnapshotError(
                                "Retained Worktree removal content changed."
                            )
                        remove_directory(
                            child_fd,
                            (*relative_parts, name),
                            record["mode"],
                        )
                        named_child = stat_entry(directory_fd, name)
                        if (
                            cls._stat_identity(named_child)
                            != cls._stat_identity(child_status)
                            or cls._directory_names_fd(child_fd)
                        ):
                            raise RetirementSnapshotError(
                                "Retained Worktree removal content changed."
                            )
                        try:
                            os.rmdir(name, dir_fd=directory_fd)
                        except OSError as exc:
                            raise RetirementSnapshotError(
                                "Retained Worktree removal found unauthorized content."
                            ) from exc
                    finally:
                        os.close(child_fd)
                    continue
                unlink_verified_entry(
                    directory_fd,
                    name,
                    record,
                    cls._stat_identity(initial),
                )
            if cls._directory_names_fd(directory_fd):
                raise RetirementSnapshotError(
                    "Retained Worktree removal found unauthorized content."
                )

        parent_fd = cls._open_directory_path(
            worktree.parent,
            error="Retained Worktree removal boundary changed.",
        )
        root_fd: int | None = None
        try:
            root_fd = cls._open_directory_at(
                parent_fd,
                worktree.name,
                error="Retained Worktree removal boundary changed.",
            )
            root_status = os.fstat(root_fd)
            if cls._stat_identity(root_status) != (
                expected_root_device,
                expected_root_inode,
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree removal boundary changed."
                )
            remove_directory(root_fd, (), manifest["root_mode"])
            named_root = stat_entry(parent_fd, worktree.name)
            if (
                cls._stat_identity(named_root) != cls._stat_identity(root_status)
                or cls._directory_names_fd(root_fd)
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree removal boundary changed."
                )
            try:
                os.rmdir(worktree.name, dir_fd=parent_fd)
            except OSError as exc:
                raise RetirementSnapshotError(
                    "Retained Worktree removal found unauthorized content."
                ) from exc
        finally:
            if root_fd is not None:
                os.close(root_fd)
            os.close(parent_fd)

    def recover_published(self) -> dict[str, Any]:
        """Recover an exact publication whose canonical phase commit was interrupted."""

        manifest_path = self.payload_root / "manifest.json"
        manifest = self._read_manifest(manifest_path)
        record = {
            "schema_version": self.schema_version,
            "manifest_path": str(manifest_path),
            "manifest_sha256": self._file_digest(manifest_path),
            "payload_path": str(self.payload_root),
            "payload_bytes": int(manifest["sizes"]["payload_bytes"]),
            "manifest_bytes": int(manifest["sizes"]["manifest_bytes"]),
            "snapshot_bytes": int(manifest["sizes"]["snapshot_bytes"]),
            "worktree_identity": self.request.worktree_identity,
            "session_revision": self.request.session_revision,
            "verified": True,
        }
        self.verify(record)
        return record

    def prepare_git_non_force_removal(
        self,
        record: dict[str, Any],
        *,
        worktree_path: Path | None = None,
    ) -> None:
        """Restore only the verified Git-visible state before non-force removal."""

        self.verify(record)
        manifest = self._read_manifest(self.payload_root / "manifest.json")
        git_state = manifest["git_state"]
        if git_state["kind"] != "git-worktree":
            raise RetirementSnapshotError(
                "Non-force Git preparation requires a Git-backed Retirement Unit."
            )
        worktree = worktree_path or self.request.worktree_path
        if self._git(worktree, ["rev-parse", "HEAD"]).strip() != git_state[
            "baseline_commit"
        ]:
            raise RetirementSnapshotError(
                "Retirement Unit changed after verified preservation."
            )
        staged = self._git(
            worktree,
            ["diff", "--cached", "--binary", "--full-index", "HEAD", "--"],
            limit=self.request.reserved_bytes + 1,
        )
        unstaged = self._git(
            worktree,
            ["diff", "--binary", "--full-index", "--"],
            limit=self.request.reserved_bytes + 1,
        )
        expected_staged = self._contained_payload_file(
            self.payload_root,
            PurePosixPath("git/staged.patch"),
        ).read_text(encoding="utf-8")
        expected_unstaged = self._contained_payload_file(
            self.payload_root,
            PurePosixPath("git/unstaged.patch"),
        ).read_text(encoding="utf-8")
        if (
            staged not in {expected_staged, ""}
            or unstaged not in {expected_unstaged, ""}
        ) and not self._tracked_git_state_matches_preserved_or_baseline(
            manifest,
            worktree_path=worktree,
        ):
            raise RetirementSnapshotError(
                "Retirement Unit changed after verified preservation."
            )
        self._assert_git_untracked_subset_matches(
            manifest,
            worktree_path=worktree,
        )
        self._git(
            worktree,
            ["restore", "--source=HEAD", "--staged", "--worktree", "--", "."],
        )
        self._assert_git_untracked_subset_matches(
            manifest,
            worktree_path=worktree,
        )
        for relative_value in sorted(
            git_state["untracked_paths"],
            key=lambda value: (len(PurePosixPath(value).parts), value),
            reverse=True,
        ):
            relative = self._safe_relative(relative_value)
            candidate = worktree / Path(*relative.parts)
            if not candidate.exists() and not candidate.is_symlink():
                continue
            if candidate.is_symlink() or candidate.is_file():
                candidate.unlink()
            else:
                raise RetirementSnapshotError(
                    f"Verified untracked removal path is unavailable: {relative_value!r}."
                )
            parent = candidate.parent
            while parent != worktree:
                try:
                    parent.rmdir()
                except OSError:
                    break
                parent = parent.parent
        if (
            self._git(worktree, ["diff", "--cached", "--binary", "HEAD", "--"])
            or self._git(worktree, ["diff", "--binary", "--"])
            or self._git_untracked_paths(worktree)
        ):
            raise RetirementSnapshotError(
                "Retirement Unit remained dirty after verified cleanup."
            )

    def _assert_git_untracked_subset_matches(
        self,
        manifest: dict[str, Any],
        *,
        worktree_path: Path | None = None,
    ) -> None:
        worktree = worktree_path or self.request.worktree_path
        git_state = manifest["git_state"]
        current_untracked = set(self._git_untracked_paths(worktree))
        expected_untracked = set(git_state["untracked_paths"])
        if not current_untracked.issubset(expected_untracked):
            raise RetirementSnapshotError(
                "Retirement Unit changed after verified preservation."
            )
        untracked_entries = self._validated_untracked_entries(git_state)
        for relative_value in sorted(current_untracked):
            relative = self._safe_relative(relative_value)
            source = worktree / Path(*relative.parts)
            payload = self._contained_payload_file(
                self.payload_root,
                PurePosixPath("untracked", *relative.parts),
            ).read_bytes()
            metadata = untracked_entries[relative_value]
            if metadata["kind"] == "symlink":
                try:
                    current_payload = os.readlink(os.fsencode(source))
                except OSError as exc:
                    raise RetirementSnapshotError(
                        "Retirement Unit changed after verified preservation."
                    ) from exc
            else:
                if source.is_symlink() or not source.is_file():
                    raise RetirementSnapshotError(
                        "Retirement Unit changed after verified preservation."
                    )
                current_payload = self._read_budgeted(source)
                if (
                    source.stat(follow_symlinks=False).st_mode & 0o777
                    != metadata["mode"]
                ):
                    raise RetirementSnapshotError(
                        "Retirement Unit changed after verified preservation."
                    )
            if current_payload != payload:
                raise RetirementSnapshotError(
                    "Retirement Unit changed after verified preservation."
                )

    def _tracked_git_state_matches_preserved_or_baseline(
        self,
        manifest: dict[str, Any],
        *,
        worktree_path: Path | None = None,
    ) -> bool:
        worktree = worktree_path or self.request.worktree_path
        verification_root = self.request.runtime_dir / "retirement" / "verification"
        verification_root.mkdir(parents=True, exist_ok=True)
        with (
            tempfile.TemporaryDirectory(
                dir=verification_root,
                prefix="preserved-tracked.",
            ) as preserved_name,
            tempfile.TemporaryDirectory(
                dir=verification_root,
                prefix="baseline-tracked.",
            ) as baseline_name,
        ):
            preserved_root = Path(preserved_name)
            baseline_root = Path(baseline_name)
            self._reconstruct_git(self.payload_root, manifest, preserved_root)
            self._reconstruct_git(self.payload_root, manifest, baseline_root)
            preserved_repository = preserved_root / "repository"
            baseline_repository = baseline_root / "repository"
            self._git(
                baseline_repository,
                ["restore", "--source=HEAD", "--staged", "--worktree", "--", "."],
            )
            current_index = self._git_index_entries(worktree)
            preserved_index = self._git_index_entries(preserved_repository)
            baseline_index = self._git_index_entries(baseline_repository)
            paths = set(current_index) | set(preserved_index) | set(baseline_index)
            for relative_value in paths:
                current_entry = current_index.get(relative_value)
                if current_entry not in {
                    preserved_index.get(relative_value),
                    baseline_index.get(relative_value),
                }:
                    return False
                relative = self._safe_relative(relative_value)
                current_state = self._filesystem_entry_state(
                    worktree,
                    relative,
                )
                if current_state == ("absent",) and current_entry is not None:
                    # A crashed non-force removal may have unlinked a tracked
                    # working-tree entry while leaving the verified index and
                    # registration intact. Git restore reconstructs that exact
                    # indexed path before removal is retried.
                    continue
                if current_state not in {
                    self._filesystem_entry_state(preserved_repository, relative),
                    self._filesystem_entry_state(baseline_repository, relative),
                }:
                    return False
        return True

    def _git_index_entries(self, repository: Path) -> dict[str, str]:
        output = self._git(repository, ["ls-files", "--stage", "-z", "--"])
        entries: dict[str, str] = {}
        for record in output.split("\0"):
            if not record:
                continue
            try:
                metadata, relative_value = record.split("\t", 1)
            except ValueError as exc:
                raise RetirementSnapshotError(
                    "Retirement Snapshot Git index state is invalid."
                ) from exc
            self._safe_relative(relative_value)
            if relative_value in entries:
                raise RetirementSnapshotError(
                    "Retirement Snapshot cannot clean an unmerged Git index."
                )
            entries[relative_value] = metadata
        return entries

    def _filesystem_entry_state(
        self,
        root: Path,
        relative: PurePosixPath,
    ) -> tuple[Any, ...]:
        path = root / Path(*relative.parts)
        if path.is_symlink():
            return ("symlink", os.readlink(os.fsencode(path)))
        if not path.exists():
            return ("absent",)
        if not path.is_file():
            return ("unsupported",)
        return (
            "file",
            bool(path.stat(follow_symlinks=False).st_mode & 0o111),
            path.stat().st_size,
            self._file_digest(path),
        )

    def prepare_managed_directory_removal(
        self,
        record: dict[str, Any],
        *,
        worktree_path: Path | None = None,
    ) -> None:
        """Prove the managed directory still equals its verified snapshot."""

        self.verify(record)
        manifest = self._read_manifest(self.payload_root / "manifest.json")
        directory_state = manifest["git_state"]
        if directory_state["kind"] != "managed-directory":
            raise RetirementSnapshotError(
                "Managed directory preparation requires a directory-backed Retirement Unit."
            )
        worktree = worktree_path or self.request.worktree_path
        expected_paths = set(directory_state["paths"])
        current_paths = set(self._filesystem_files(worktree))
        if not current_paths.issubset(expected_paths):
            raise RetirementSnapshotError(
                "Retirement Unit changed after verified preservation."
            )
        current: dict[str, dict[str, Any]] = {}
        for relative_value in sorted(current_paths):
            relative = self._safe_relative(relative_value)
            source = worktree / Path(*relative.parts)
            if source.stat().st_mode & 0o777 != directory_state["modes"][relative_value]:
                raise RetirementSnapshotError(
                    "Retirement Unit changed after verified preservation."
                )
            current[f"directory/{relative.as_posix()}"] = {
                "size": source.stat().st_size,
                "sha256": self._file_digest(source),
            }
            if current[f"directory/{relative.as_posix()}"] != manifest["files"][
                f"directory/{relative.as_posix()}"
            ]:
                raise RetirementSnapshotError(
                    "Retirement Unit changed after verified preservation."
                )

    def quarantine_publication(self, record: dict[str, Any]) -> None:
        """Move an exact unpublished payload aside so its unit can fail closed."""

        if (
            record.get("payload_path") != str(self.payload_root)
            or not self.payload_root.is_dir()
            or self.payload_root.is_symlink()
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot publication boundary cannot be quarantined."
            )
        quarantine_root = self.request.runtime_dir / "retirement" / "quarantine"
        quarantine_root.mkdir(parents=True, exist_ok=True)
        digest = str(record.get("manifest_sha256", "invalid"))[:24]
        destination = quarantine_root / f"{self.request.session_id}.{digest}"
        if destination.exists() or destination.is_symlink():
            raise RetirementSnapshotError(
                "Retirement Snapshot quarantine destination already exists."
            )
        self.payload_root.replace(destination)

    def _capture_into(self, root: Path) -> dict[str, Any]:
        worktree = self.request.worktree_path
        git_marker = worktree / ".git"
        entries: dict[str, dict[str, Any]] = {}
        git_state: dict[str, Any]
        if git_marker.exists():
            git_state = self._capture_git_state(root, entries)
        elif self.request.worktree_identity.startswith("managed-directory:"):
            git_state = self._capture_directory_state(root, entries)
        elif (
            self.request.worktree_identity.startswith("managed-absence:")
            and not worktree.exists()
            and not worktree.is_symlink()
        ):
            git_state = {"kind": "managed-absence"}
        else:
            raise RetirementSnapshotError(
                "Retirement Snapshot requires an exact managed worktree."
            )
        evidence = self._capture_evidence(root, entries)
        payload_bytes = sum(item["size"] for item in entries.values())
        if payload_bytes > self.request.reserved_bytes:
            raise RetirementSnapshotError(
                "Retirement Snapshot exceeds the bound Preservation Budget: "
                f"{payload_bytes} > {self.request.reserved_bytes} bytes."
            )
        baseline_payload = json.dumps(
            {
                "repository_snapshot": self.request.repository_snapshot,
                "baseline_fingerprints": self.request.baseline_fingerprints,
            },
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        return {
            "schema_version": self.schema_version,
            "authority": {
                "mission_id": self.request.mission_id,
                "session_id": self.request.session_id,
                "session_revision": self.request.session_revision,
                "terminal_status": self.request.terminal_status,
                "evidence_correlation_id": self.request.evidence_correlation_id,
                "evidence_valid": self.request.evidence_valid,
            },
            "identity": {
                "stored_session_path": str(self.request.worktree_path),
                "canonical_worktree_path": str(
                    self.request.worktree_path.resolve(strict=False)
                ),
                "worktree_identity": self.request.worktree_identity,
                "baseline_sha256": sha256(baseline_payload).hexdigest(),
                "baseline": {
                    "repository_snapshot": self.request.repository_snapshot,
                    "baseline_fingerprints": self.request.baseline_fingerprints,
                },
            },
            "git_state": git_state,
            "evidence": evidence,
            "files": entries,
            "sizes": {
                "payload_bytes": payload_bytes,
                "manifest_bytes": 0,
                "snapshot_bytes": payload_bytes,
                "reserved_bytes": self.request.reserved_bytes,
                "file_count": len(entries),
            },
            "verification": {
                "manifest_readback": False,
                "clean_room_reconstruction": False,
            },
        }

    def _capture_git_state(
        self,
        root: Path,
        entries: dict[str, dict[str, Any]],
    ) -> dict[str, Any]:
        worktree = self.request.worktree_path
        baseline = self._git(worktree, ["rev-parse", "HEAD"]).strip()
        if len(baseline) != 40 or any(
            character not in "0123456789abcdef" for character in baseline
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot baseline Git identity is invalid."
            )
        bundle_path = root / "git" / "baseline.bundle"
        bundle_path.parent.mkdir(parents=True, exist_ok=True)
        self._git(
            worktree,
            ["bundle", "create", str(bundle_path), "HEAD"],
        )
        self._record_payload_file(bundle_path, root, entries)
        status = self._git(worktree, ["status", "--porcelain=v2", "-z", "--"])
        staged = self._git(
            worktree,
            ["diff", "--cached", "--binary", "--full-index", "HEAD", "--"],
            limit=self.request.reserved_bytes + 1,
        )
        unstaged = self._git(
            worktree,
            ["diff", "--binary", "--full-index", "--"],
            limit=self.request.reserved_bytes + 1,
        )
        git_root = root / "git"
        self._write_payload(
            git_root / "staged.patch", staged.encode("utf-8"), root, entries
        )
        self._write_payload(
            git_root / "unstaged.patch", unstaged.encode("utf-8"), root, entries
        )
        self._write_payload(
            git_root / "status.porcelain-v2", status.encode("utf-8"), root, entries
        )
        untracked: list[str] = []
        untracked_entries: dict[str, dict[str, Any]] = {}
        for value in self._git_untracked_paths(worktree):
            relative = self._safe_relative(value)
            source = worktree / Path(*relative.parts)
            destination = root / "untracked" / Path(*relative.parts)
            relative_value = relative.as_posix()
            if source.is_symlink():
                try:
                    payload = os.readlink(os.fsencode(source))
                except OSError as exc:
                    raise RetirementSnapshotError(
                        f"Retirement Snapshot could not read untracked symlink: {value!r}."
                    ) from exc
                metadata: dict[str, Any] = {"kind": "symlink"}
            else:
                try:
                    source.resolve(strict=True).relative_to(
                        worktree.resolve(strict=True)
                    )
                except (OSError, ValueError) as exc:
                    raise RetirementSnapshotError(
                        f"Retirement Snapshot untracked path escaped its worktree: {value!r}."
                    ) from exc
                if not source.is_file():
                    raise RetirementSnapshotError(
                        f"Retirement Snapshot untracked path is unsupported: {value!r}."
                    )
                payload = self._read_budgeted(source)
                metadata = {
                    "kind": "file",
                    "mode": source.stat(follow_symlinks=False).st_mode & 0o777,
                }
            self._write_payload(destination, payload, root, entries)
            untracked.append(relative_value)
            untracked_entries[relative_value] = metadata
        return {
            "kind": "git-worktree",
            "baseline_commit": baseline,
            "status_sha256": sha256(status.encode("utf-8")).hexdigest(),
            "status_bytes": len(status.encode("utf-8")),
            "untracked_paths": sorted(untracked),
            "untracked_entries": {
                path: untracked_entries[path] for path in sorted(untracked_entries)
            },
        }

    def _git_untracked_paths(self, worktree: Path) -> list[str]:
        visible = self._git(
            worktree,
            ["ls-files", "--others", "--exclude-standard", "-z", "--"],
        )
        ignored = self._git(
            worktree,
            [
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z",
                "--",
            ],
        )
        return sorted(
            {
                value
                for output in (visible, ignored)
                for value in output.split("\0")
                if value
            }
        )

    def _capture_directory_state(
        self,
        root: Path,
        entries: dict[str, dict[str, Any]],
    ) -> dict[str, Any]:
        paths = self._filesystem_files(self.request.worktree_path)
        modes: dict[str, int] = {}
        for relative_value in paths:
            relative = self._safe_relative(relative_value)
            source = self.request.worktree_path / Path(*relative.parts)
            self._write_payload(
                root / "directory" / Path(*relative.parts),
                self._read_budgeted(source),
                root,
                entries,
            )
            modes[relative.as_posix()] = source.stat().st_mode & 0o777
        return {
            "kind": "managed-directory",
            "paths": paths,
            "modes": modes,
            "tree_sha256": self._entries_digest(entries, prefix="directory/"),
        }

    def _capture_evidence(
        self,
        root: Path,
        entries: dict[str, dict[str, Any]],
    ) -> dict[str, dict[str, Any]]:
        artifact_root = (
            self.request.runtime_dir / "sessions" / self.request.session_id
        ).resolve(strict=False)
        evidence: dict[str, dict[str, Any]] = {}
        if len(self.request.artifacts) > 256:
            raise RetirementSnapshotError(
                "Retirement Snapshot exceeds the 256-artifact evidence limit."
            )
        for artifact_id, raw_path in sorted(self.request.artifacts.items()):
            if (
                not isinstance(artifact_id, str)
                or not artifact_id
                or len(artifact_id.encode("utf-8")) > 4_096
                or not isinstance(raw_path, str)
                or not raw_path
                or len(raw_path.encode("utf-8")) > 4_096
            ):
                raise RetirementSnapshotError(
                    "Retirement Snapshot contains an invalid evidence registration."
                )
            source = Path(raw_path)
            try:
                canonical = source.resolve(strict=True)
                canonical.relative_to(artifact_root)
            except (OSError, ValueError) as exc:
                raise RetirementSnapshotError(
                    f"Registered evidence {artifact_id!r} is outside its app-local session boundary."
                ) from exc
            if source.is_symlink() or not canonical.is_file():
                raise RetirementSnapshotError(
                    f"Registered evidence {artifact_id!r} is unavailable or unsupported."
                )
            safe_id = sha256(artifact_id.encode("utf-8")).hexdigest()
            destination = root / "evidence" / safe_id
            payload = self._read_budgeted(canonical)
            self._write_payload(destination, payload, root, entries)
            evidence[artifact_id] = {
                "payload": destination.relative_to(root).as_posix(),
                "size": len(payload),
                "sha256": sha256(payload).hexdigest(),
            }
        return evidence

    def _verify_payload_integrity(
        self,
        manifest_path: Path,
        manifest: dict[str, Any],
    ) -> None:
        self._validate_manifest_authority(manifest)
        root = manifest_path.parent
        files = manifest.get("files")
        if not isinstance(files, dict):
            raise RetirementSnapshotError(
                "Retirement Snapshot manifest files are invalid."
            )
        total = 0
        for relative_value, record in files.items():
            relative = self._safe_relative(relative_value)
            path = self._contained_payload_file(root, relative)
            if not isinstance(record, dict):
                raise RetirementSnapshotError(
                    f"Retirement Snapshot payload integrity failed for {relative_value!r}."
                )
            size = record.get("size")
            digest = record.get("sha256")
            if (
                not isinstance(size, int)
                or isinstance(size, bool)
                or size < 0
                or not isinstance(digest, str)
                or path.stat().st_size != size
                or self._file_digest(path) != digest
            ):
                raise RetirementSnapshotError(
                    f"Retirement Snapshot payload integrity failed for {relative_value!r}."
                )
            total += size
        sizes = manifest.get("sizes")
        verification = manifest.get("verification")
        verified = verification == {
            "manifest_readback": True,
            "clean_room_reconstruction": True,
        }
        manifest_bytes = manifest_path.stat().st_size if verified else 0
        if (
            not isinstance(sizes, dict)
            or sizes.get("payload_bytes") != total
            or not isinstance(sizes.get("manifest_bytes"), int)
            or not isinstance(sizes.get("snapshot_bytes"), int)
            or sizes.get("file_count") != len(files)
            or (
                verified
                and (
                    sizes.get("manifest_bytes") != manifest_bytes
                    or sizes.get("snapshot_bytes") != total + manifest_bytes
                )
            )
            or sizes.get("snapshot_bytes") > self.request.reserved_bytes
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot size or Preservation Budget integrity failed."
            )

    def _verify_clean_room(self, manifest_path: Path, manifest: dict[str, Any]) -> None:
        git_state = manifest["git_state"]
        clean_parent = self.request.runtime_dir / "retirement" / "clean-room"
        clean_parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            dir=clean_parent,
            prefix=f"{self.request.session_id}.",
        ) as clean_name:
            clean = Path(clean_name)
            if git_state["kind"] == "git-worktree":
                self._reconstruct_git(manifest_path.parent, manifest, clean)
            elif git_state["kind"] == "managed-directory":
                self._reconstruct_directory(manifest_path.parent, manifest, clean)
            elif git_state["kind"] == "managed-absence":
                self._reconstruct_absence(clean)
            else:
                raise RetirementSnapshotError(
                    "Retirement Snapshot manifest has an unsupported repository kind."
                )

    def _reconstruct_git(
        self,
        payload_root: Path,
        manifest: dict[str, Any],
        clean: Path,
    ) -> None:
        repository = clean / "repository"
        self._git(
            clean,
            [
                "clone",
                "--quiet",
                "--no-checkout",
                str(payload_root / "git" / "baseline.bundle"),
                str(repository),
            ],
        )
        self._git(
            repository,
            [
                "checkout",
                "--quiet",
                "--detach",
                manifest["git_state"]["baseline_commit"],
            ],
        )
        staged = (payload_root / "git" / "staged.patch").read_text(encoding="utf-8")
        unstaged = (payload_root / "git" / "unstaged.patch").read_text(encoding="utf-8")
        if staged:
            self._git(
                repository, ["apply", "--index", "--binary", "-"], input_text=staged
            )
        if unstaged:
            self._git(repository, ["apply", "--binary", "-"], input_text=unstaged)
        untracked_root = payload_root / "untracked"
        untracked_entries = self._validated_untracked_entries(manifest["git_state"])
        for relative_value, metadata in untracked_entries.items():
            relative = self._safe_relative(relative_value)
            source = untracked_root / Path(*relative.parts)
            destination = repository / Path(*relative.parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            if metadata["kind"] == "symlink":
                try:
                    target = source.read_bytes()
                    os.symlink(target, os.fsencode(destination))
                except OSError as exc:
                    raise RetirementSnapshotError(
                        f"Retirement Snapshot could not reconstruct untracked symlink: {relative_value!r}."
                    ) from exc
                if (
                    not destination.is_symlink()
                    or os.readlink(os.fsencode(destination)) != target
                ):
                    raise RetirementSnapshotError(
                        f"Retirement Snapshot untracked symlink reconstruction failed: {relative_value!r}."
                    )
            else:
                shutil.copyfile(source, destination)
                mode = metadata.get("mode")
                if mode is not None:
                    destination.chmod(mode)
                    if destination.stat(follow_symlinks=False).st_mode & 0o777 != mode:
                        raise RetirementSnapshotError(
                            f"Retirement Snapshot untracked file mode reconstruction failed: {relative_value!r}."
                        )
        status = self._git(repository, ["status", "--porcelain=v2", "-z", "--"])
        if (
            sha256(status.encode("utf-8")).hexdigest()
            != manifest["git_state"]["status_sha256"]
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot clean-room Git reconstruction failed."
            )

    def _reconstruct_directory(
        self,
        payload_root: Path,
        manifest: dict[str, Any],
        clean: Path,
    ) -> None:
        repository = clean / "repository"
        repository.mkdir()
        directory_state = manifest["git_state"]
        paths = directory_state.get("paths")
        modes = directory_state.get("modes")
        if (
            not isinstance(paths, list)
            or not all(isinstance(path, str) and path for path in paths)
            or len(paths) != len(set(paths))
            or not isinstance(modes, dict)
            or set(modes) != set(paths)
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot managed-directory state is invalid."
            )
        reconstructed: dict[str, dict[str, Any]] = {}
        for relative_value in paths:
            relative = self._safe_relative(relative_value)
            mode = modes[relative_value]
            if (
                not isinstance(mode, int)
                or isinstance(mode, bool)
                or mode < 0
                or mode > 0o777
            ):
                raise RetirementSnapshotError(
                    "Retirement Snapshot managed-directory mode is invalid."
                )
            source = self._contained_payload_file(
                payload_root,
                PurePosixPath("directory", *relative.parts),
            )
            destination = repository / Path(*relative.parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
            destination.chmod(mode)
            reconstructed[f"directory/{relative.as_posix()}"] = {
                "size": destination.stat().st_size,
                "sha256": self._file_digest(destination),
            }
        if (
            self._filesystem_files(repository) != sorted(paths)
            or self._entries_digest(reconstructed, prefix="directory/")
            != directory_state.get("tree_sha256")
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot clean-room directory reconstruction failed."
            )

    @staticmethod
    def _reconstruct_absence(clean: Path) -> None:
        repository = clean / "repository"
        repository.mkdir()
        if any(repository.iterdir()):
            raise RetirementSnapshotError(
                "Retirement Snapshot clean-room absence reconstruction failed."
            )

    def _materialize_manifest_to_trusted_root(
        self,
        manifest: dict[str, Any],
        root: Path,
    ) -> None:
        """Reconstruct one verified manifest only inside an app-private root."""

        repository_kind = manifest["git_state"]["kind"]
        if repository_kind == "git-worktree":
            self._reconstruct_git(self.payload_root, manifest, root)
        elif repository_kind == "managed-directory":
            self._reconstruct_directory(self.payload_root, manifest, root)
        elif repository_kind == "managed-absence":
            self._reconstruct_absence(root)
        else:
            raise RetirementSnapshotError(
                "Retirement Snapshot reconstruction kind is unsupported."
            )

    def _verify_materialized_repository_path(
        self,
        manifest: dict[str, Any],
        repository: Path,
    ) -> None:
        """Semantically verify a repository copied into app-private storage."""

        git_state = manifest["git_state"]
        if git_state["kind"] == "git-worktree":
            head = self._git(repository, ["rev-parse", "HEAD"]).strip()
            status = self._git(repository, ["status", "--porcelain=v2", "-z", "--"])
            if (
                head != git_state["baseline_commit"]
                or sha256(status.encode("utf-8")).hexdigest()
                != git_state["status_sha256"]
            ):
                raise RetirementSnapshotError(
                    "Retirement Snapshot exported Git state is invalid."
                )

        verification_root = self.request.runtime_dir / "retirement" / "export-check"
        verification_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            dir=verification_root,
            prefix=f"{self.request.session_id}.expected.",
        ) as expected_name:
            expected_root = Path(expected_name)
            self._materialize_manifest_to_trusted_root(manifest, expected_root)
            expected = expected_root / "repository"
            exclude_git_metadata = git_state["kind"] == "git-worktree"
            if self._repository_tree_entries(
                repository,
                exclude_git_metadata=exclude_git_metadata,
            ) != self._repository_tree_entries(
                expected,
                exclude_git_metadata=exclude_git_metadata,
            ):
                raise RetirementSnapshotError(
                    "Retirement Snapshot exported repository content is invalid."
                )

    def _validated_untracked_entries(
        self,
        git_state: dict[str, Any],
    ) -> dict[str, dict[str, Any]]:
        paths = git_state.get("untracked_paths")
        if (
            not isinstance(paths, list)
            or not all(isinstance(path, str) and path for path in paths)
            or len(paths) != len(set(paths))
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot untracked path manifest is invalid."
            )
        for path in paths:
            self._safe_relative(path)
        raw_entries = git_state.get("untracked_entries")
        if raw_entries is None:
            return {path: {"kind": "file", "mode": None} for path in sorted(paths)}
        if not isinstance(raw_entries, dict) or set(raw_entries) != set(paths):
            raise RetirementSnapshotError(
                "Retirement Snapshot untracked entry manifest is invalid."
            )
        validated: dict[str, dict[str, Any]] = {}
        symlink_paths: set[PurePosixPath] = set()
        for path in sorted(paths):
            record = raw_entries.get(path)
            if not isinstance(record, dict):
                raise RetirementSnapshotError(
                    "Retirement Snapshot untracked entry manifest is invalid."
                )
            kind = record.get("kind")
            if kind == "symlink" and set(record) == {"kind"}:
                symlink_paths.add(self._safe_relative(path))
                validated[path] = {"kind": "symlink"}
                continue
            mode = record.get("mode")
            if (
                kind != "file"
                or set(record) != {"kind", "mode"}
                or not isinstance(mode, int)
                or isinstance(mode, bool)
                or mode < 0
                or mode > 0o777
            ):
                raise RetirementSnapshotError(
                    "Retirement Snapshot untracked entry manifest is invalid."
                )
            validated[path] = {"kind": "file", "mode": mode}
        for path in (self._safe_relative(value) for value in paths):
            if any(parent in symlink_paths for parent in path.parents):
                raise RetirementSnapshotError(
                    "Retirement Snapshot untracked entries cannot descend through a symlink."
                )
        return validated

    def _validate_manifest_authority(self, manifest: dict[str, Any]) -> None:
        authority = manifest.get("authority")
        identity = manifest.get("identity")
        baseline_payload = json.dumps(
            {
                "repository_snapshot": self.request.repository_snapshot,
                "baseline_fingerprints": self.request.baseline_fingerprints,
            },
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        expected_evidence_ids = set(self.request.artifacts)
        evidence = manifest.get("evidence")
        sizes = manifest.get("sizes")
        verification = manifest.get("verification")
        if (
            not isinstance(manifest, dict)
            or manifest.get("schema_version") != self.schema_version
            or not isinstance(authority, dict)
            or authority.get("mission_id") != self.request.mission_id
            or authority.get("session_id") != self.request.session_id
            or authority.get("session_revision") != self.request.session_revision
            or authority.get("terminal_status") != self.request.terminal_status
            or authority.get("evidence_correlation_id")
            != self.request.evidence_correlation_id
            or authority.get("evidence_valid") is not self.request.evidence_valid
            or not isinstance(identity, dict)
            or identity.get("worktree_identity") != self.request.worktree_identity
            or identity.get("stored_session_path") != str(self.request.worktree_path)
            or identity.get("canonical_worktree_path")
            != str(self.request.worktree_path.resolve(strict=False))
            or identity.get("baseline_sha256") != sha256(baseline_payload).hexdigest()
            or identity.get("baseline")
            != {
                "repository_snapshot": self.request.repository_snapshot,
                "baseline_fingerprints": self.request.baseline_fingerprints,
            }
            or not isinstance(evidence, dict)
            or set(evidence) != expected_evidence_ids
            or not isinstance(sizes, dict)
            or sizes.get("reserved_bytes") != self.request.reserved_bytes
            or not isinstance(verification, dict)
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot manifest authority is invalid."
            )

    def _git(
        self,
        cwd: Path,
        arguments: list[str],
        *,
        input_text: str | None = None,
        limit: int = 1_000_000,
    ) -> str:
        returncode, stdout, stderr = self.run_git(cwd, arguments, input_text, limit)
        if returncode != 0:
            raise RetirementSnapshotError(
                "Retirement Snapshot Git operation failed: "
                + (stderr.strip() or stdout.strip() or f"exit {returncode}")
            )
        return stdout

    @staticmethod
    def _stat_identity(status: os.stat_result) -> tuple[int, int]:
        return status.st_dev, status.st_ino

    @staticmethod
    def _stable_stat(status: os.stat_result) -> tuple[int, int, int, int, int, int]:
        return (
            status.st_dev,
            status.st_ino,
            status.st_mode,
            status.st_size,
            status.st_mtime_ns,
            status.st_ctime_ns,
        )

    @staticmethod
    def _open_directory_path(path: Path, *, error: str) -> int:
        flags = (
            os.O_RDONLY
            | getattr(os, "O_DIRECTORY", 0)
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
        )
        descriptor: int | None = None
        try:
            descriptor = os.open(path, flags)
            status = os.fstat(descriptor)
            if not stat.S_ISDIR(status.st_mode):
                raise RetirementSnapshotError(error)
            return descriptor
        except RetirementSnapshotError:
            if descriptor is not None:
                os.close(descriptor)
            raise
        except OSError as exc:
            if descriptor is not None:
                try:
                    os.close(descriptor)
                except OSError:
                    pass
            raise RetirementSnapshotError(error) from exc

    @staticmethod
    def _duplicate_directory_fd(descriptor: int, *, error: str) -> int:
        duplicate: int | None = None
        try:
            duplicate = os.dup(descriptor)
            status = os.fstat(duplicate)
            if not stat.S_ISDIR(status.st_mode):
                raise RetirementSnapshotError(error)
            return duplicate
        except RetirementSnapshotError:
            if duplicate is not None:
                os.close(duplicate)
            raise
        except OSError as exc:
            if duplicate is not None:
                try:
                    os.close(duplicate)
                except OSError:
                    pass
            raise RetirementSnapshotError(error) from exc

    @staticmethod
    def _open_directory_at(parent_fd: int, name: str, *, error: str) -> int:
        if not name or "/" in name or "\0" in name or name in {".", ".."}:
            raise RetirementSnapshotError(error)
        flags = (
            os.O_RDONLY
            | getattr(os, "O_DIRECTORY", 0)
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
        )
        descriptor: int | None = None
        try:
            descriptor = os.open(name, flags, dir_fd=parent_fd)
            status = os.fstat(descriptor)
            if not stat.S_ISDIR(status.st_mode):
                raise RetirementSnapshotError(error)
            return descriptor
        except RetirementSnapshotError:
            if descriptor is not None:
                os.close(descriptor)
            raise
        except OSError as exc:
            if descriptor is not None:
                try:
                    os.close(descriptor)
                except OSError:
                    pass
            raise RetirementSnapshotError(error) from exc

    @staticmethod
    def _directory_names_fd(descriptor: int) -> tuple[str, ...]:
        try:
            names = tuple(os.listdir(descriptor))
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retirement Snapshot directory could not be inspected."
            ) from exc
        if not all(
            isinstance(name, str)
            and name
            and "/" not in name
            and "\0" not in name
            and name not in {".", ".."}
            for name in names
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot directory contains an invalid entry name."
            )
        return tuple(sorted(names, key=os.fsencode))

    @classmethod
    def _require_empty_directory_fd(cls, descriptor: int, *, error: str) -> None:
        try:
            status = os.fstat(descriptor)
        except OSError as exc:
            raise RetirementSnapshotError(error) from exc
        if not stat.S_ISDIR(status.st_mode) or cls._directory_names_fd(descriptor):
            raise RetirementSnapshotError(error)

    @classmethod
    def _open_relative_parent_at(
        cls,
        root_fd: int,
        relative: PurePosixPath,
        *,
        error: str,
    ) -> tuple[int, str]:
        if not relative.parts:
            raise RetirementSnapshotError(error)
        current = cls._duplicate_directory_fd(root_fd, error=error)
        try:
            for part in relative.parts[:-1]:
                child = cls._open_directory_at(current, part, error=error)
                os.close(current)
                current = child
            return current, relative.parts[-1]
        except BaseException:
            os.close(current)
            raise

    @staticmethod
    def _file_descriptor_digest(descriptor: int) -> str:
        digest = sha256()
        try:
            position = os.lseek(descriptor, 0, os.SEEK_CUR)
            os.lseek(descriptor, 0, os.SEEK_SET)
            while chunk := os.read(descriptor, 1024 * 1024):
                digest.update(chunk)
            os.lseek(descriptor, position, os.SEEK_SET)
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retirement Snapshot descriptor integrity read failed."
            ) from exc
        return digest.hexdigest()

    @classmethod
    def _read_stable_symlink_at(
        cls,
        parent_fd: int,
        name: str,
        initial: os.stat_result,
        *,
        error: str,
    ) -> bytes:
        if not stat.S_ISLNK(initial.st_mode):
            raise RetirementSnapshotError(error)
        try:
            target = os.readlink(os.fsencode(name), dir_fd=parent_fd)
            after = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
        except OSError as exc:
            raise RetirementSnapshotError(error) from exc
        if (
            not isinstance(target, bytes)
            or b"\0" in target
            or cls._stable_stat(after) != cls._stable_stat(initial)
        ):
            raise RetirementSnapshotError(error)
        return target

    @classmethod
    def _copy_expected_symlink_at(
        cls,
        source_parent_fd: int,
        source_name: str,
        destination_parent_fd: int,
        destination_name: str,
        expected_target: bytes,
    ) -> None:
        error = "Retained Worktree changed during export materialization."
        try:
            source_before = os.stat(
                source_name,
                dir_fd=source_parent_fd,
                follow_symlinks=False,
            )
            if (
                cls._read_stable_symlink_at(
                    source_parent_fd,
                    source_name,
                    source_before,
                    error=error,
                )
                != expected_target
            ):
                raise RetirementSnapshotError(error)
            os.symlink(
                expected_target,
                os.fsencode(destination_name),
                dir_fd=destination_parent_fd,
            )
            destination_status = os.stat(
                destination_name,
                dir_fd=destination_parent_fd,
                follow_symlinks=False,
            )
            if (
                cls._read_stable_symlink_at(
                    destination_parent_fd,
                    destination_name,
                    destination_status,
                    error="Retained Worktree export destination boundary changed.",
                )
                != expected_target
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree export destination boundary changed."
                )
            source_after = os.stat(
                source_name,
                dir_fd=source_parent_fd,
                follow_symlinks=False,
            )
            if cls._stable_stat(source_after) != cls._stable_stat(source_before):
                raise RetirementSnapshotError(error)
        except RetirementSnapshotError:
            raise
        except OSError as exc:
            raise RetirementSnapshotError(error) from exc

    @classmethod
    def _copy_expected_regular_at(
        cls,
        source_parent_fd: int,
        source_name: str,
        destination_parent_fd: int,
        destination_name: str,
        expected: dict[str, Any],
    ) -> None:
        source_flags = (
            os.O_RDONLY
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
            | getattr(os, "O_NONBLOCK", 0)
        )
        destination_flags = (
            os.O_RDWR
            | os.O_CREAT
            | os.O_EXCL
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
        )
        source_descriptor: int | None = None
        destination_descriptor: int | None = None
        try:
            initial = os.stat(
                source_name,
                dir_fd=source_parent_fd,
                follow_symlinks=False,
            )
            source_descriptor = os.open(
                source_name,
                source_flags,
                dir_fd=source_parent_fd,
            )
            before = os.fstat(source_descriptor)
            if (
                not stat.S_ISREG(before.st_mode)
                or cls._stable_stat(before) != cls._stable_stat(initial)
                or before.st_mode & 0o777 != expected["mode"]
                or before.st_size != expected["size"]
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree changed during export materialization."
                )
            destination_descriptor = os.open(
                destination_name,
                destination_flags,
                expected["mode"],
                dir_fd=destination_parent_fd,
            )
            digest = sha256()
            copied = 0
            while chunk := os.read(source_descriptor, 1024 * 1024):
                digest.update(chunk)
                copied += len(chunk)
                written = 0
                while written < len(chunk):
                    count = os.write(destination_descriptor, chunk[written:])
                    if count <= 0:
                        raise OSError("Retained Worktree export write did not progress.")
                    written += count
            after = os.fstat(source_descriptor)
            if (
                cls._stable_stat(after) != cls._stable_stat(before)
                or copied != expected["size"]
                or digest.hexdigest() != expected["sha256"]
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree changed during export materialization."
                )
            os.fchmod(destination_descriptor, expected["mode"])
            os.fsync(destination_descriptor)
            destination_status = os.fstat(destination_descriptor)
            named_destination = os.stat(
                destination_name,
                dir_fd=destination_parent_fd,
                follow_symlinks=False,
            )
            if (
                not stat.S_ISREG(destination_status.st_mode)
                or cls._stat_identity(destination_status)
                != cls._stat_identity(named_destination)
                or destination_status.st_mode & 0o777 != expected["mode"]
                or destination_status.st_size != expected["size"]
                or cls._file_descriptor_digest(destination_descriptor)
                != expected["sha256"]
            ):
                raise RetirementSnapshotError(
                    "Retained Worktree export destination boundary changed."
                )
        except RetirementSnapshotError:
            raise
        except OSError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree changed during export materialization."
            ) from exc
        finally:
            if destination_descriptor is not None:
                os.close(destination_descriptor)
            if source_descriptor is not None:
                os.close(source_descriptor)

    @classmethod
    def _copy_repository_into_directory(
        cls,
        source_repository_fd: int,
        destination_root_fd: int,
    ) -> None:
        """Copy an exact repository into an empty borrowed destination root."""

        source_fd = cls._duplicate_directory_fd(
            source_repository_fd,
            error="Retirement Snapshot private reconstruction is invalid.",
        )
        root_fd: int | None = None
        repository_fd: int | None = None
        try:
            root_fd = cls._duplicate_directory_fd(
                destination_root_fd,
                error="Retirement Snapshot reconstruction destination is invalid.",
            )
            cls._require_empty_directory_fd(
                root_fd,
                error=(
                    "Retirement Snapshot reconstruction destination is not an empty "
                    "directory."
                ),
            )
            source_manifest = cls.retained_worktree_manifest_from_directory(
                source_fd,
                exclude_git_metadata=False,
            )
            os.mkdir("repository", mode=0o700, dir_fd=root_fd)
            repository_fd = cls._open_directory_at(
                root_fd,
                "repository",
                error="Retirement Snapshot reconstruction destination changed.",
            )
            cls._copy_directory_contents_at(source_fd, repository_fd)
            os.fchmod(repository_fd, source_manifest["root_mode"])
            os.fsync(repository_fd)
            if (
                cls.retained_worktree_manifest_from_directory(
                    source_fd,
                    exclude_git_metadata=False,
                )
                != source_manifest
                or cls.retained_worktree_manifest_from_directory(
                    repository_fd,
                    exclude_git_metadata=False,
                )
                != source_manifest
                or cls._directory_names_fd(root_fd) != ("repository",)
            ):
                raise RetirementSnapshotError(
                    "Retirement Snapshot reconstruction destination changed."
                )
        except RetirementSnapshotError:
            raise
        except OSError as exc:
            raise RetirementSnapshotError(
                f"Retirement Snapshot reconstruction failed: {exc}"
            ) from exc
        finally:
            if repository_fd is not None:
                os.close(repository_fd)
            if root_fd is not None:
                os.close(root_fd)
            os.close(source_fd)

    @classmethod
    def _copy_directory_contents_at(
        cls,
        source_fd: int,
        destination_fd: int,
    ) -> None:
        source_before = os.fstat(source_fd)
        if (
            not stat.S_ISDIR(source_before.st_mode)
            or cls._directory_names_fd(destination_fd)
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot reconstruction directory is invalid."
            )
        names = cls._directory_names_fd(source_fd)
        for name in names:
            initial = os.stat(name, dir_fd=source_fd, follow_symlinks=False)
            if stat.S_ISLNK(initial.st_mode):
                target = cls._read_stable_symlink_at(
                    source_fd,
                    name,
                    initial,
                    error="Retirement Snapshot private reconstruction changed.",
                )
                cls._copy_expected_symlink_at(
                    source_fd,
                    name,
                    destination_fd,
                    name,
                    target,
                )
                continue
            if stat.S_ISREG(initial.st_mode):
                record = cls._retained_regular_file_record_at(
                    source_fd,
                    name,
                    initial,
                )
                cls._copy_expected_regular_at(
                    source_fd,
                    name,
                    destination_fd,
                    name,
                    record,
                )
                continue
            if not stat.S_ISDIR(initial.st_mode):
                raise RetirementSnapshotError(
                    "Retirement Snapshot private reconstruction contains an "
                    "unsupported entry."
                )
            source_child_fd = cls._open_directory_at(
                source_fd,
                name,
                error="Retirement Snapshot private reconstruction changed.",
            )
            destination_child_fd: int | None = None
            try:
                if cls._stat_identity(os.fstat(source_child_fd)) != cls._stat_identity(
                    initial
                ):
                    raise RetirementSnapshotError(
                        "Retirement Snapshot private reconstruction changed."
                    )
                os.mkdir(name, mode=0o700, dir_fd=destination_fd)
                destination_child_fd = cls._open_directory_at(
                    destination_fd,
                    name,
                    error="Retirement Snapshot reconstruction destination changed.",
                )
                cls._copy_directory_contents_at(
                    source_child_fd,
                    destination_child_fd,
                )
                os.fchmod(destination_child_fd, initial.st_mode & 0o777)
                os.fsync(destination_child_fd)
                if cls._stable_stat(os.fstat(source_child_fd)) != cls._stable_stat(
                    initial
                ):
                    raise RetirementSnapshotError(
                        "Retirement Snapshot private reconstruction changed."
                    )
            finally:
                if destination_child_fd is not None:
                    os.close(destination_child_fd)
                os.close(source_child_fd)
        if (
            cls._directory_names_fd(source_fd) != names
            or cls._directory_names_fd(destination_fd) != names
            or cls._stable_stat(os.fstat(source_fd)) != cls._stable_stat(source_before)
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot reconstruction directory changed."
            )

    def _write_payload(
        self,
        path: Path,
        payload: bytes,
        root: Path,
        entries: dict[str, dict[str, Any]],
    ) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
        relative = path.relative_to(root).as_posix()
        entries[relative] = {
            "size": len(payload),
            "sha256": sha256(payload).hexdigest(),
        }
        if sum(item["size"] for item in entries.values()) > self.request.reserved_bytes:
            raise RetirementSnapshotError(
                "Retirement Snapshot exceeds the bound Preservation Budget."
            )

    def _record_payload_file(
        self,
        path: Path,
        root: Path,
        entries: dict[str, dict[str, Any]],
    ) -> None:
        size = path.stat().st_size
        relative = path.relative_to(root).as_posix()
        entries[relative] = {"size": size, "sha256": self._file_digest(path)}
        if sum(item["size"] for item in entries.values()) > self.request.reserved_bytes:
            raise RetirementSnapshotError(
                "Retirement Snapshot exceeds the bound Preservation Budget."
            )

    def _read_budgeted(self, path: Path) -> bytes:
        size = path.stat().st_size
        if size > self.request.reserved_bytes:
            raise RetirementSnapshotError(
                f"Retirement Snapshot file exceeds the bound Preservation Budget: {path.name}."
            )
        with path.open("rb") as handle:
            payload = handle.read(self.request.reserved_bytes + 1)
        if len(payload) > self.request.reserved_bytes:
            raise RetirementSnapshotError(
                f"Retirement Snapshot file exceeds the bound Preservation Budget: {path.name}."
            )
        return payload

    @staticmethod
    def _safe_relative(value: str) -> PurePosixPath:
        if not isinstance(value, str) or not value or "\\" in value or "\0" in value:
            raise RetirementSnapshotError(
                f"Retirement Snapshot contains an unsafe relative path: {value!r}."
            )
        path = PurePosixPath(value)
        if path.is_absolute() or not path.parts or ".." in path.parts:
            raise RetirementSnapshotError(
                f"Retirement Snapshot contains an unsafe relative path: {value!r}."
            )
        return path

    @staticmethod
    def _safe_retained_relative(value: str) -> PurePosixPath:
        """Validate one filesystem-representable retained POSIX path."""

        if (
            not isinstance(value, str)
            or not value
            or "\0" in value
            or (os.name == "nt" and "\\" in value)
        ):
            raise RetirementSnapshotError(
                "Retained Worktree recovery manifest is invalid."
            )
        try:
            if os.fsdecode(os.fsencode(value)) != value:
                raise RetirementSnapshotError(
                    "Retained Worktree recovery manifest is invalid."
                )
        except UnicodeError as exc:
            raise RetirementSnapshotError(
                "Retained Worktree recovery manifest is invalid."
            ) from exc
        path = PurePosixPath(value)
        if (
            path.is_absolute()
            or not path.parts
            or ".." in path.parts
            or path.as_posix() != value
        ):
            raise RetirementSnapshotError(
                "Retained Worktree recovery manifest is invalid."
            )
        return path

    @classmethod
    def _filesystem_files(cls, root: Path) -> list[str]:
        result: list[str] = []
        scanned = 0
        for directory, names, filenames in os.walk(
            root,
            followlinks=False,
            onerror=cls._raise_incomplete_tree_walk,
        ):
            for name in names:
                if (Path(directory) / name).is_symlink():
                    raise RetirementSnapshotError(
                        f"Retirement Snapshot contains an unsupported path: {name!r}."
                    )
            names[:] = sorted(names)
            for name in sorted(filenames):
                scanned += 1
                if scanned > 10_000:
                    raise RetirementSnapshotError(
                        "Retirement Snapshot exceeds the 10000-file preservation limit."
                    )
                path = Path(directory) / name
                if path.is_symlink() or not path.is_file():
                    raise RetirementSnapshotError(
                        f"Retirement Snapshot contains an unsupported path: {path.name!r}."
                    )
                result.append(path.relative_to(root).as_posix())
        return sorted(result)

    def _repository_tree_entries(
        self,
        root: Path,
        *,
        exclude_git_metadata: bool,
    ) -> dict[str, dict[str, Any]]:
        entries: dict[str, dict[str, Any]] = {}
        scanned = 0
        for directory, names, filenames in os.walk(
            root,
            followlinks=False,
            onerror=self._raise_incomplete_tree_walk,
        ):
            directory_path = Path(directory)
            if exclude_git_metadata and directory_path == root and ".git" in names:
                names.remove(".git")
            for name in sorted(tuple(names)):
                path = directory_path / name
                if not path.is_symlink():
                    continue
                names.remove(name)
                scanned += 1
                entries[path.relative_to(root).as_posix()] = {
                    "kind": "symlink",
                    "target": os.readlink(os.fsencode(path)).hex(),
                }
                if scanned > 10_000:
                    raise RetirementSnapshotError(
                        "Retirement Snapshot export exceeds the 10000-entry limit."
                    )
            names[:] = sorted(names)
            for name in sorted(filenames):
                path = directory_path / name
                if exclude_git_metadata and directory_path == root and name == ".git":
                    continue
                scanned += 1
                relative = path.relative_to(root).as_posix()
                if path.is_symlink():
                    entries[relative] = {
                        "kind": "symlink",
                        "target": os.readlink(os.fsencode(path)).hex(),
                    }
                elif path.is_file():
                    stat_result = path.stat(follow_symlinks=False)
                    entries[relative] = {
                        "kind": "file",
                        "mode": stat_result.st_mode & 0o777,
                        "size": stat_result.st_size,
                        "sha256": self._file_digest(path),
                    }
                else:
                    raise RetirementSnapshotError(
                        "Retirement Snapshot export contains an unsupported entry."
                    )
                if scanned > 10_000:
                    raise RetirementSnapshotError(
                        "Retirement Snapshot export exceeds the 10000-entry limit."
                    )
        return entries

    @staticmethod
    def _entries_digest(entries: dict[str, dict[str, Any]], *, prefix: str) -> str:
        selected = {
            path: entries[path] for path in sorted(entries) if path.startswith(prefix)
        }
        return sha256(
            json.dumps(selected, sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).hexdigest()

    @staticmethod
    def _contained_payload_file(root: Path, relative: PurePosixPath) -> Path:
        try:
            if root.is_symlink():
                raise RetirementSnapshotError(
                    "Retirement Snapshot payload root is a symlink."
                )
            canonical_root = root.resolve(strict=True)
            candidate = root / Path(*relative.parts)
            current = candidate
            while current != root:
                if current.is_symlink():
                    raise RetirementSnapshotError(
                        f"Retirement Snapshot payload contains a symlink: {relative.as_posix()!r}."
                    )
                current = current.parent
            canonical = candidate.resolve(strict=True)
            canonical.relative_to(canonical_root)
        except (OSError, ValueError) as exc:
            raise RetirementSnapshotError(
                f"Retirement Snapshot payload escaped its boundary: {relative.as_posix()!r}."
            ) from exc
        if not canonical.is_file():
            raise RetirementSnapshotError(
                f"Retirement Snapshot payload is not a regular file: {relative.as_posix()!r}."
            )
        return canonical

    @staticmethod
    def _write_json(path: Path, value: dict[str, Any]) -> None:
        path.write_text(
            json.dumps(value, indent=2, sort_keys=True),
            encoding="utf-8",
        )

    def _write_final_manifest(self, path: Path, manifest: dict[str, Any]) -> None:
        for _ in range(8):
            self._write_json(path, manifest)
            manifest_bytes = path.stat().st_size
            snapshot_bytes = manifest["sizes"]["payload_bytes"] + manifest_bytes
            if snapshot_bytes > self.request.reserved_bytes:
                raise RetirementSnapshotError(
                    "Retirement Snapshot exceeds the bound Preservation Budget."
                )
            if (
                manifest["sizes"].get("manifest_bytes") == manifest_bytes
                and manifest["sizes"].get("snapshot_bytes") == snapshot_bytes
            ):
                return
            manifest["sizes"]["manifest_bytes"] = manifest_bytes
            manifest["sizes"]["snapshot_bytes"] = snapshot_bytes
        raise RetirementSnapshotError(
            "Retirement Snapshot manifest size did not converge."
        )

    @staticmethod
    def _read_manifest(path: Path) -> dict[str, Any]:
        try:
            if path.stat().st_size > RetirementSnapshotStore.manifest_bytes_limit:
                raise RetirementSnapshotError(
                    "Retirement Snapshot manifest exceeds its readback limit."
                )
            with path.open("rb") as handle:
                payload = handle.read(RetirementSnapshotStore.manifest_bytes_limit + 1)
            if len(payload) > RetirementSnapshotStore.manifest_bytes_limit:
                raise RetirementSnapshotError(
                    "Retirement Snapshot manifest exceeds its readback limit."
                )
            value = json.loads(payload.decode("utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as exc:
            raise RetirementSnapshotError(
                f"Retirement Snapshot manifest readback failed: {exc}"
            ) from exc
        if not isinstance(value, dict):
            raise RetirementSnapshotError(
                "Retirement Snapshot manifest readback is not an object."
            )
        return value

    @staticmethod
    def _file_digest(path: Path) -> str:
        digest = sha256()
        try:
            with path.open("rb") as handle:
                for chunk in iter(lambda: handle.read(128 * 1024), b""):
                    digest.update(chunk)
        except OSError as exc:
            raise RetirementSnapshotError(
                f"Retirement Snapshot integrity read failed: {exc}"
            ) from exc
        return digest.hexdigest()
