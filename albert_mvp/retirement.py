from __future__ import annotations

import json
import os
import shutil
import tempfile
from collections.abc import Callable
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path, PurePosixPath
from typing import Any


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

    def materialize(self, record: dict[str, Any], destination_root: Path) -> Path:
        """Reconstruct verified preserved material below an explicit empty root."""

        self.verify(record)
        if (
            destination_root.is_symlink()
            or not destination_root.is_dir()
            or any(destination_root.iterdir())
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot reconstruction destination is not an empty directory."
            )
        manifest_path = self._contained_payload_file(
            self.payload_root,
            PurePosixPath("manifest.json"),
        )
        manifest = self._read_manifest(manifest_path)
        repository_kind = manifest["git_state"]["kind"]
        if repository_kind == "git-worktree":
            self._reconstruct_git(self.payload_root, manifest, destination_root)
        elif repository_kind == "managed-directory":
            self._reconstruct_directory(
                self.payload_root,
                manifest,
                destination_root,
            )
        elif repository_kind == "managed-absence":
            self._reconstruct_absence(destination_root)
        else:
            raise RetirementSnapshotError(
                "Retirement Snapshot reconstruction kind is unsupported."
            )
        return destination_root / "repository"

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

    @classmethod
    def _filesystem_files(cls, root: Path) -> list[str]:
        result: list[str] = []
        scanned = 0
        for directory, names, filenames in os.walk(root, followlinks=False):
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
