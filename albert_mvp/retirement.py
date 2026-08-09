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
        if not git_marker.exists():
            raise RetirementSnapshotError(
                "Retirement Snapshot requires an exact registered Git worktree."
            )
        git_state = self._capture_git_state(root, entries)
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
                    self.request.worktree_path.resolve(strict=True)
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
        untracked_output = self._git(
            worktree,
            ["ls-files", "--others", "--exclude-standard", "-z", "--"],
        )
        untracked: list[str] = []
        for value in untracked_output.split("\0"):
            if not value:
                continue
            relative = self._safe_relative(value)
            source = worktree / Path(*relative.parts)
            try:
                source.resolve(strict=True).relative_to(worktree.resolve(strict=True))
            except (OSError, ValueError) as exc:
                raise RetirementSnapshotError(
                    f"Retirement Snapshot untracked path escaped its worktree: {value!r}."
                ) from exc
            if source.is_symlink() or not source.is_file():
                raise RetirementSnapshotError(
                    f"Retirement Snapshot untracked path is unsupported: {value!r}."
                )
            destination = root / "untracked" / Path(*relative.parts)
            self._write_payload(destination, self._read_budgeted(source), root, entries)
            untracked.append(relative.as_posix())
        return {
            "kind": "git-worktree",
            "baseline_commit": baseline,
            "status_sha256": sha256(status.encode("utf-8")).hexdigest(),
            "status_bytes": len(status.encode("utf-8")),
            "untracked_paths": sorted(untracked),
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
        for relative_value in manifest["git_state"]["untracked_paths"]:
            relative = self._safe_relative(relative_value)
            source = untracked_root / Path(*relative.parts)
            destination = repository / Path(*relative.parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
        status = self._git(repository, ["status", "--porcelain=v2", "-z", "--"])
        if (
            sha256(status.encode("utf-8")).hexdigest()
            != manifest["git_state"]["status_sha256"]
        ):
            raise RetirementSnapshotError(
                "Retirement Snapshot clean-room Git reconstruction failed."
            )

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
            != str(self.request.worktree_path.resolve(strict=True))
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
            names[:] = sorted(
                name for name in names if not (Path(directory) / name).is_symlink()
            )
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
