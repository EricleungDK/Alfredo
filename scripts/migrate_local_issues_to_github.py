#!/usr/bin/env python3
"""Migrate the canonical .scratch tracker to GitHub Issues.

The default command is a local-only dry run. ``--apply`` is idempotent:
stable source markers recover already-created issues after interruption, and
the manifest is updated atomically after each remote creation.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from typing import Any, Callable, Iterable, Sequence


SCHEMA_VERSION = 1
SOURCE_MARKER_RE = re.compile(
    r"<!-- local-issue-source: (?P<path>[^ ]+) sha256:(?P<sha>[0-9a-f]{64}) -->"
)
COMMENT_MARKER_RE = re.compile(
    r"<!-- local-issue-comment: (?P<path>[^ ]+) index:(?P<index>\d+) "
    r"sha256:(?P<sha>[0-9a-f]{64}) -->"
)
H2_RE = re.compile(r"^##\s+(.+?)\s*$")
H3_RE = re.compile(r"^###\s+(.+?)\s*$")
SEQUENCE_RE = re.compile(r"^(?P<sequence>\d+)-")

LABELS: dict[str, tuple[str, str]] = {
    "needs-triage": ("d876e3", "Maintainer needs to evaluate this issue"),
    "needs-info": ("fbca04", "Waiting on the reporter for more information"),
    "ready-for-agent": ("0e8a16", "Fully specified and ready for an AFK agent"),
    "ready-for-human": ("1d76db", "Requires human implementation or review"),
    "wontfix": ("ffffff", "Will not be actioned"),
    "type:prd": ("5319e7", "Parent issue for a product requirements document"),
    "type:afk": ("0052cc", "AFK Issue Slice"),
    "type:hitl": ("d93f0b", "Human-in-the-loop Issue Slice"),
    "migration:local-markdown": (
        "ededed",
        "Imported from the repository's local Markdown tracker",
    ),
}
TRIAGE_LABELS = {
    "needs-triage",
    "needs-info",
    "ready-for-agent",
    "ready-for-human",
    "wontfix",
}
COMPLETE_STATUSES = {"complete", "completed"}
PRD_SUFFIXES = (
    " product requirements document",
)


class MigrationError(RuntimeError):
    """Raised when source or remote state cannot be migrated safely."""


@dataclasses.dataclass(frozen=True)
class ImportedComment:
    index: int
    original: str
    sha256: str


@dataclasses.dataclass(frozen=True)
class LocalIssue:
    source_path: str
    feature: str
    kind: str
    sequence: str | None
    title: str
    github_title: str
    status: str
    issue_type: str
    sha256: str
    body: str
    comments: tuple[ImportedComment, ...]
    parent_source: str | None
    blockers: tuple[str, ...]
    aliases: tuple[str, ...]

    @property
    def labels(self) -> tuple[str, ...]:
        values = ["migration:local-markdown"]
        if self.kind == "prd":
            values.append("type:prd")
        elif self.issue_type == "hitl":
            values.append("type:hitl")
        else:
            values.append("type:afk")
        if self.status in TRIAGE_LABELS:
            values.append(self.status)
        return tuple(values)

    @property
    def expected_state(self) -> tuple[str, str | None]:
        if self.status in COMPLETE_STATUSES:
            return ("closed", "completed")
        if self.status == "wontfix":
            return ("closed", "not_planned")
        return ("open", None)


@dataclasses.dataclass(frozen=True)
class Inventory:
    issues: tuple[LocalIssue, ...]
    aliases: dict[str, str]

    @property
    def by_source(self) -> dict[str, LocalIssue]:
        return {issue.source_path: issue for issue in self.issues}


def _sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _sha256_text(value: str) -> str:
    return _sha256_bytes(value.encode("utf-8"))


def _normalize_status(value: str) -> str:
    return value.strip().lower()


def _smart_title(slug: str) -> str:
    acronyms = {
        "afk": "AFK",
        "api": "API",
        "cli": "CLI",
        "github": "GitHub",
        "hitl": "HITL",
        "mvp": "MVP",
        "npm": "npm",
        "ollama": "Ollama",
        "pr": "PR",
        "prd": "PRD",
        "tui": "TUI",
        "ui": "UI",
    }
    lowercase_words = {
        "and",
        "as",
        "for",
        "from",
        "in",
        "into",
        "of",
        "or",
        "the",
        "through",
        "to",
        "while",
        "with",
    }
    words = []
    for index, word in enumerate(slug.replace("_", "-").split("-")):
        lowered = word.lower()
        if lowered in acronyms:
            words.append(acronyms[lowered])
        elif index > 0 and lowered in lowercase_words:
            words.append(lowered)
        else:
            words.append(word.capitalize())
    return " ".join(words)


def _short_prd_title(title: str) -> str:
    shortened = title.strip()
    if shortened.lower().startswith("prd:"):
        shortened = shortened[4:].strip()
    lowered = shortened.lower()
    for suffix in PRD_SUFFIXES:
        if lowered.endswith(suffix):
            shortened = shortened[: -len(suffix)].rstrip()
            break
    return shortened


def _h2_sections(lines: Sequence[str]) -> dict[str, tuple[int, int]]:
    starts: list[tuple[str, int]] = []
    for index, line in enumerate(lines):
        match = H2_RE.match(line)
        if match:
            starts.append((match.group(1).strip().lower(), index))
    sections: dict[str, tuple[int, int]] = {}
    for position, (name, start) in enumerate(starts):
        end = starts[position + 1][1] if position + 1 < len(starts) else len(lines)
        sections[name] = (start, end)
    return sections


def _section_content(
    lines: Sequence[str], sections: dict[str, tuple[int, int]], name: str
) -> list[str]:
    span = sections.get(name.lower())
    if span is None:
        return []
    return list(lines[span[0] + 1 : span[1]])


def _first_content_line(lines: Sequence[str]) -> str | None:
    for line in lines:
        stripped = line.strip()
        if stripped:
            return stripped
    return None


def _clean_reference(raw: str) -> str:
    cleaned = raw.strip()
    if cleaned.startswith("- "):
        cleaned = cleaned[2:].strip()
    return cleaned.strip("`").strip()


def _resolve_reference(
    root: Path, source_path: str, raw: str, known_paths: set[str]
) -> str | None:
    cleaned = _clean_reference(raw)
    if not cleaned or cleaned.lower().startswith("none"):
        return None
    candidate = Path(cleaned)
    if candidate.is_absolute():
        try:
            resolved = candidate.resolve().relative_to(root.resolve()).as_posix()
        except ValueError as exc:
            raise MigrationError(
                f"{source_path}: reference leaves repository: {cleaned}"
            ) from exc
    elif cleaned.startswith((".scratch/", ".agent/")):
        resolved = candidate.as_posix()
    else:
        resolved = (Path(source_path).parent / candidate).as_posix()
    if resolved not in known_paths:
        raise MigrationError(f"{source_path}: unresolved tracker reference: {cleaned}")
    return resolved


def _extract_comment_entries(lines: Sequence[str]) -> tuple[ImportedComment, ...]:
    content = list(lines)
    while content and not content[0].strip():
        content.pop(0)
    while content and not content[-1].strip():
        content.pop()
    if not content:
        return ()

    h3_positions = [
        index for index, line in enumerate(content) if H3_RE.match(line)
    ]
    entries: list[str] = []
    if h3_positions:
        if any(line.strip() for line in content[: h3_positions[0]]):
            entries.append("\n".join(content[: h3_positions[0]]).strip())
        for position, start in enumerate(h3_positions):
            end = (
                h3_positions[position + 1]
                if position + 1 < len(h3_positions)
                else len(content)
            )
            heading = H3_RE.match(content[start])
            assert heading is not None
            body = "\n".join(content[start + 1 : end]).strip()
            entry = f"**{heading.group(1).strip()}**"
            if body:
                entry = f"{entry}\n\n{body}"
            entries.append(entry)
    else:
        current: list[str] = []
        for line in content:
            if line.startswith("- ") and current:
                entries.append("\n".join(current).strip())
                current = [line[2:]]
            elif line.startswith("- "):
                current = [line[2:]]
            else:
                current.append(line)
        if current:
            entries.append("\n".join(current).strip())

    return tuple(
        ImportedComment(index=index, original=entry, sha256=_sha256_text(entry))
        for index, entry in enumerate((entry for entry in entries if entry), start=1)
    )


def _body_without_tracker_sections(
    lines: Sequence[str], sections: dict[str, tuple[int, int]]
) -> str:
    excluded: set[int] = set()
    for section_name in ("parent", "blocked by", "comments"):
        span = sections.get(section_name)
        if span is not None:
            excluded.update(range(span[0], span[1]))

    kept: list[str] = []
    h1_removed = False
    for index, line in enumerate(lines):
        if index in excluded:
            continue
        stripped = line.strip()
        if re.match(r"^(Status|Type):\s*", stripped, re.IGNORECASE):
            continue
        if not h1_removed and re.match(r"^#\s+", line):
            h1_removed = True
            continue
        kept.append(line.rstrip())
    while kept and not kept[0].strip():
        kept.pop(0)
    while kept and not kept[-1].strip():
        kept.pop()
    return "\n".join(kept)


def _parse_unresolved(
    root: Path, path: Path
) -> dict[str, Any]:
    relative = path.relative_to(root).as_posix()
    raw_bytes = path.read_bytes()
    text = raw_bytes.decode("utf-8-sig").replace("\r\n", "\n").replace("\r", "\n")
    lines = text.splitlines()
    sections = _h2_sections(lines)

    status_match = next(
        (
            re.match(r"^Status:\s*(.+?)\s*$", line, re.IGNORECASE)
            for line in lines
            if re.match(r"^Status:\s*", line, re.IGNORECASE)
        ),
        None,
    )
    if status_match is None:
        raise MigrationError(f"{relative}: missing Status")
    status = _normalize_status(status_match.group(1))

    type_match = next(
        (
            re.match(r"^Type:\s*(.+?)\s*$", line, re.IGNORECASE)
            for line in lines
            if re.match(r"^Type:\s*", line, re.IGNORECASE)
        ),
        None,
    )
    is_prd = (
        (type_match is not None and type_match.group(1).strip().lower() == "prd")
        or path.name.lower() == "prd.md"
        or ("prd" in path.stem.lower() and "issues" not in path.parts)
    )
    issue_type = (
        "prd"
        if is_prd
        else (type_match.group(1).strip().lower() if type_match else "afk")
    )

    h1 = next(
        (re.sub(r"^#\s+", "", line).strip() for line in lines if line.startswith("# ")),
        None,
    )
    sequence_match = SEQUENCE_RE.match(path.name)
    sequence = sequence_match.group("sequence") if sequence_match and not is_prd else None
    fallback_slug = path.stem
    if sequence_match:
        fallback_slug = fallback_slug[len(sequence_match.group(0)) :]
    title = h1 or _smart_title(fallback_slug)
    github_title = (
        f"[PRD] {_short_prd_title(title)}"
        if is_prd
        else (f"{sequence} — {title}" if sequence else title)
    )

    parent_line = _first_content_line(_section_content(lines, sections, "parent"))
    blocker_lines = [
        line
        for line in _section_content(lines, sections, "blocked by")
        if line.strip().startswith("- ")
    ]
    comments = _extract_comment_entries(_section_content(lines, sections, "comments"))

    return {
        "source_path": relative,
        "feature": Path(relative).parts[1],
        "kind": "prd" if is_prd else "issue",
        "sequence": sequence,
        "title": title,
        "github_title": github_title,
        "status": status,
        "issue_type": issue_type,
        "sha256": _sha256_bytes(raw_bytes),
        "body": _body_without_tracker_sections(lines, sections),
        "comments": comments,
        "raw_parent": parent_line,
        "raw_blockers": blocker_lines,
    }


def discover_inventory(root: Path) -> Inventory:
    scratch = root / ".scratch"
    if not scratch.is_dir():
        raise MigrationError(f"missing canonical tracker directory: {scratch}")

    paths = sorted(
        path
        for path in scratch.rglob("*.md")
        if path.name.lower() != "readme.md"
    )
    unresolved = [_parse_unresolved(root, path) for path in paths]
    known_paths = {item["source_path"] for item in unresolved}

    aliases: dict[str, str] = {}
    for item in unresolved:
        canonical = item["source_path"]
        basename = Path(canonical).name
        legacy = root / ".agent" / "issues" / basename
        if legacy.is_file() and legacy.read_bytes() == (root / canonical).read_bytes():
            alias = legacy.relative_to(root).as_posix()
            aliases[alias] = canonical
            known_paths.add(alias)

    feature_prds: dict[str, str] = {}
    for item in unresolved:
        if item["kind"] == "prd":
            feature = item["feature"]
            if feature in feature_prds:
                raise MigrationError(f"{feature}: multiple canonical PRDs")
            feature_prds[feature] = item["source_path"]

    issues: list[LocalIssue] = []
    for item in unresolved:
        source_path = item["source_path"]
        raw_parent = item.pop("raw_parent")
        raw_blockers = item.pop("raw_blockers")
        explicit_parent = (
            _resolve_reference(root, source_path, raw_parent, known_paths)
            if raw_parent
            else None
        )
        if explicit_parent in aliases:
            explicit_parent = aliases[explicit_parent]
        parent_source = (
            explicit_parent
            if explicit_parent is not None
            else (
                feature_prds[item["feature"]]
                if item["kind"] == "issue"
                else None
            )
        )
        blockers: list[str] = []
        for raw_blocker in raw_blockers:
            blocker = _resolve_reference(
                root, source_path, raw_blocker, known_paths
            )
            if blocker is None:
                continue
            blockers.append(aliases.get(blocker, blocker))
        issue_aliases = tuple(
            sorted(alias for alias, canonical in aliases.items() if canonical == source_path)
        )
        issues.append(
            LocalIssue(
                **item,
                parent_source=parent_source,
                blockers=tuple(blockers),
                aliases=issue_aliases,
            )
        )

    by_source = {issue.source_path: issue for issue in issues}
    for issue in issues:
        for reference in (issue.parent_source, *issue.blockers):
            if reference is not None and reference not in by_source:
                raise MigrationError(
                    f"{issue.source_path}: reference is not canonical: {reference}"
                )

    def issue_sort_key(issue: LocalIssue) -> tuple[str, int, int, str]:
        kind_order = 0 if issue.kind == "prd" else 1
        sequence = int(issue.sequence) if issue.sequence is not None else 9999
        return (issue.feature, kind_order, sequence, issue.source_path)

    return Inventory(issues=tuple(sorted(issues, key=issue_sort_key)), aliases=aliases)


def _manifest_item(issue: LocalIssue, existing: dict[str, Any] | None) -> dict[str, Any]:
    github = dict((existing or {}).get("github") or {})
    if existing and existing.get("sha256") != issue.sha256 and github.get("number"):
        raise MigrationError(
            f"{issue.source_path}: source changed after GitHub mapping; "
            "reconcile the issue deliberately before continuing"
        )
    return {
        "feature": issue.feature,
        "kind": issue.kind,
        "sequence": issue.sequence,
        "title": issue.github_title,
        "status": issue.status,
        "issue_type": issue.issue_type,
        "sha256": issue.sha256,
        "parent_source": issue.parent_source,
        "blockers": list(issue.blockers),
        "comment_count": len(issue.comments),
        "aliases": list(issue.aliases),
        "github": github,
    }


def build_manifest(
    inventory: Inventory,
    repo: str,
    existing: dict[str, Any] | None = None,
) -> dict[str, Any]:
    old_items = dict((existing or {}).get("items") or {})
    return {
        "schema_version": SCHEMA_VERSION,
        "repo": repo,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "source": ".scratch",
        "policy": {
            "external_prs_are_triage_surface": False,
            "legacy_agent_issues_included": False,
            "local_archive_retained": True,
        },
        "items": {
            issue.source_path: _manifest_item(
                issue, old_items.get(issue.source_path)
            )
            for issue in inventory.issues
        },
    }


def read_manifest(path: Path) -> dict[str, Any] | None:
    if not path.exists():
        return None
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != SCHEMA_VERSION:
        raise MigrationError(f"{path}: unsupported manifest schema")
    return data


def write_manifest(path: Path, manifest: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=path.parent, text=True
    )
    try:
        with os.fdopen(handle, "w", encoding="utf-8") as stream:
            json.dump(manifest, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_name, path)
    finally:
        if os.path.exists(temporary_name):
            os.unlink(temporary_name)


def render_body(
    issue: LocalIssue,
    number_by_source: dict[str, int],
    aliases: dict[str, str],
) -> str:
    body = issue.body
    replacements: dict[str, int] = dict(number_by_source)
    for alias, canonical in aliases.items():
        if canonical in number_by_source:
            replacements[alias] = number_by_source[canonical]
    for source, number in sorted(replacements.items(), key=lambda item: -len(item[0])):
        body = body.replace(f"`{source}`", f"#{number}")
        body = body.replace(source, f"#{number}")
    provenance = (
        f"<!-- local-issue-source: {issue.source_path} sha256:{issue.sha256} -->\n"
        f"> Migrated from `{issue.source_path}`. Original status: "
        f"`{issue.status}`; original type: `{issue.issue_type}`; "
        f"source SHA-256: `{issue.sha256}`."
    )
    return f"{provenance}\n\n{body}".rstrip() + "\n"


def render_comment(
    issue: LocalIssue,
    comment: ImportedComment,
    number_by_source: dict[str, int],
    aliases: dict[str, str],
) -> str:
    body = comment.original
    replacements: dict[str, int] = dict(number_by_source)
    for alias, canonical in aliases.items():
        if canonical in number_by_source:
            replacements[alias] = number_by_source[canonical]
    for source, number in sorted(replacements.items(), key=lambda item: -len(item[0])):
        body = body.replace(f"`{source}`", f"#{number}")
        body = body.replace(source, f"#{number}")
    marker = (
        f"<!-- local-issue-comment: {issue.source_path} index:{comment.index} "
        f"sha256:{comment.sha256} -->"
    )
    return (
        f"{marker}\n> Imported tracker comment {comment.index} from "
        f"`{issue.source_path}`.\n\n{body}".rstrip()
        + "\n"
    )


RunCommand = Callable[[Sequence[str], bool], subprocess.CompletedProcess[str]]


def _default_run(
    command: Sequence[str], check: bool = True
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        list(command),
        check=check,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


class GitHubClient:
    def __init__(
        self,
        repo: str,
        run_command: RunCommand = _default_run,
    ) -> None:
        self.repo = repo
        self._run_command = run_command
        try:
            self.owner, self.name = repo.split("/", 1)
        except ValueError as exc:
            raise MigrationError(f"invalid GitHub repository: {repo}") from exc

    def _run(
        self, command: Sequence[str], check: bool = True
    ) -> subprocess.CompletedProcess[str]:
        result = self._run_command(command, check)
        if check and result.returncode != 0:
            raise MigrationError(result.stderr.strip() or "GitHub CLI command failed")
        return result

    def api(
        self,
        endpoint: str,
        *,
        method: str = "GET",
        fields: Iterable[tuple[str, str | int | bool]] = (),
        check: bool = True,
    ) -> Any:
        command = [
            "gh",
            "api",
            "--method",
            method,
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2026-03-10",
            endpoint,
        ]
        for name, value in fields:
            command.extend(["-F", f"{name}={str(value).lower() if isinstance(value, bool) else value}"])
        result = self._run(command, check=check)
        if result.returncode != 0:
            return None
        output = result.stdout.strip()
        return json.loads(output) if output else None

    def verify_access(self) -> dict[str, Any]:
        auth = self._run(["gh", "auth", "status"], check=False)
        if auth.returncode != 0:
            raise MigrationError(
                "GitHub authentication is unavailable; run `gh auth login -h github.com`"
            )
        repository = self.api(f"repos/{self.repo}")
        if not repository.get("has_issues"):
            raise MigrationError(f"GitHub Issues are disabled for {self.repo}")
        return repository

    def list_labels(self) -> dict[str, dict[str, Any]]:
        labels = self.api(f"repos/{self.repo}/labels?per_page=100")
        return {label["name"]: label for label in labels}

    def ensure_labels(self) -> None:
        existing = self.list_labels()
        for name, (color, description) in LABELS.items():
            if name in existing:
                continue
            self.api(
                f"repos/{self.repo}/labels",
                method="POST",
                fields=(
                    ("name", name),
                    ("color", color),
                    ("description", description),
                ),
            )

    def list_issues(self) -> list[dict[str, Any]]:
        issues = self.api(f"repos/{self.repo}/issues?state=all&per_page=100")
        return [issue for issue in issues if "pull_request" not in issue]

    def create_issue(
        self, *, title: str, body: str, labels: Sequence[str]
    ) -> dict[str, Any]:
        fields: list[tuple[str, str]] = [("title", title), ("body", body)]
        fields.extend(("labels[]", label) for label in labels)
        return self.api(f"repos/{self.repo}/issues", method="POST", fields=fields)

    def update_issue(
        self,
        number: int,
        *,
        title: str,
        body: str,
        labels: Sequence[str],
        state: str,
        state_reason: str | None,
    ) -> dict[str, Any]:
        fields: list[tuple[str, str]] = [
            ("title", title),
            ("body", body),
            ("state", state),
        ]
        fields.extend(("labels[]", label) for label in labels)
        if state_reason is not None:
            fields.append(("state_reason", state_reason))
        return self.api(
            f"repos/{self.repo}/issues/{number}", method="PATCH", fields=fields
        )

    def get_issue(self, number: int) -> dict[str, Any]:
        return self.api(f"repos/{self.repo}/issues/{number}")

    def list_comments(self, number: int) -> list[dict[str, Any]]:
        return self.api(f"repos/{self.repo}/issues/{number}/comments?per_page=100")

    def create_comment(self, number: int, body: str) -> dict[str, Any]:
        return self.api(
            f"repos/{self.repo}/issues/{number}/comments",
            method="POST",
            fields=(("body", body),),
        )

    def get_parent(self, number: int) -> dict[str, Any] | None:
        return self.api(
            f"repos/{self.repo}/issues/{number}/parent", check=False
        )

    def add_sub_issue(
        self, parent_number: int, child_database_id: int
    ) -> dict[str, Any]:
        return self.api(
            f"repos/{self.repo}/issues/{parent_number}/sub_issues",
            method="POST",
            fields=(("sub_issue_id", child_database_id),),
        )

    def list_sub_issues(self, parent_number: int) -> list[dict[str, Any]]:
        return self.api(
            f"repos/{self.repo}/issues/{parent_number}/sub_issues?per_page=100"
        )

    def reprioritize_sub_issue(
        self,
        parent_number: int,
        child_database_id: int,
        *,
        after_database_id: int,
    ) -> dict[str, Any]:
        return self.api(
            f"repos/{self.repo}/issues/{parent_number}/sub_issues/priority",
            method="PATCH",
            fields=(
                ("sub_issue_id", child_database_id),
                ("after_id", after_database_id),
            ),
        )

    def list_blockers(self, number: int) -> list[dict[str, Any]]:
        return self.api(
            f"repos/{self.repo}/issues/{number}/dependencies/blocked_by?per_page=100"
        )

    def add_blocker(
        self, number: int, blocker_database_id: int
    ) -> dict[str, Any]:
        return self.api(
            f"repos/{self.repo}/issues/{number}/dependencies/blocked_by",
            method="POST",
            fields=(("issue_id", blocker_database_id),),
        )


def _issue_marker_map(remote_issues: Sequence[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    mapped: dict[str, dict[str, Any]] = {}
    for issue in remote_issues:
        match = SOURCE_MARKER_RE.search(issue.get("body") or "")
        if not match:
            continue
        source = match.group("path")
        if source in mapped:
            raise MigrationError(f"multiple GitHub issues claim source {source}")
        issue["_source_sha256"] = match.group("sha")
        mapped[source] = issue
    return mapped


def _selected_issues(
    inventory: Inventory, features: set[str] | None
) -> tuple[LocalIssue, ...]:
    if not features:
        return inventory.issues
    missing = features - {issue.feature for issue in inventory.issues}
    if missing:
        raise MigrationError(f"unknown feature(s): {', '.join(sorted(missing))}")
    return tuple(issue for issue in inventory.issues if issue.feature in features)


def _number_map(manifest: dict[str, Any]) -> dict[str, int]:
    numbers: dict[str, int] = {}
    for source, data in manifest["items"].items():
        number = (data.get("github") or {}).get("number")
        if number is not None:
            numbers[source] = int(number)
    return numbers


def _preflight_references(
    selected: Sequence[LocalIssue], number_by_source: dict[str, int]
) -> None:
    selected_sources = {issue.source_path for issue in selected}
    for issue in selected:
        for reference in (issue.parent_source, *issue.blockers):
            if (
                reference is not None
                and reference not in selected_sources
                and reference not in number_by_source
            ):
                raise MigrationError(
                    f"{issue.source_path}: referenced issue is outside this apply set "
                    f"and has not been migrated: {reference}"
                )


def _expected_children(inventory: Inventory, parent_source: str) -> list[LocalIssue]:
    children = [
        issue for issue in inventory.issues if issue.parent_source == parent_source
    ]
    return sorted(
        children,
        key=lambda issue: (
            0 if issue.kind == "issue" else 1,
            int(issue.sequence) if issue.sequence is not None else 9999,
            issue.source_path,
        ),
    )


def apply_migration(
    *,
    root: Path,
    inventory: Inventory,
    manifest_path: Path,
    manifest: dict[str, Any],
    features: set[str] | None,
    client: GitHubClient,
) -> dict[str, Any]:
    client.verify_access()
    client.ensure_labels()
    selected = _selected_issues(inventory, features)

    remote_by_source = _issue_marker_map(client.list_issues())
    for source, remote in remote_by_source.items():
        if source not in manifest["items"]:
            continue
        expected_sha = manifest["items"][source]["sha256"]
        if remote["_source_sha256"] != expected_sha:
            raise MigrationError(
                f"{source}: remote source marker has a different SHA-256"
            )
        manifest["items"][source]["github"] = {
            "number": remote["number"],
            "id": remote["id"],
            "url": remote["html_url"],
        }

    number_by_source = _number_map(manifest)
    _preflight_references(selected, number_by_source)

    for issue in selected:
        github = manifest["items"][issue.source_path]["github"]
        if github.get("number"):
            continue
        placeholder_body = render_body(issue, number_by_source, inventory.aliases)
        created = client.create_issue(
            title=issue.github_title,
            body=placeholder_body,
            labels=issue.labels,
        )
        manifest["items"][issue.source_path]["github"] = {
            "number": created["number"],
            "id": created["id"],
            "url": created["html_url"],
        }
        write_manifest(manifest_path, manifest)
        number_by_source[issue.source_path] = created["number"]

    number_by_source = _number_map(manifest)
    for issue in selected:
        github = manifest["items"][issue.source_path]["github"]
        number = int(github["number"])
        expected_body = render_body(issue, number_by_source, inventory.aliases)
        expected_state, state_reason = issue.expected_state
        client.update_issue(
            number,
            title=issue.github_title,
            body=expected_body,
            labels=issue.labels,
            state=expected_state,
            state_reason=state_reason,
        )
        existing_comment_markers = {
            (
                match.group("path"),
                int(match.group("index")),
                match.group("sha"),
            )
            for remote_comment in client.list_comments(number)
            for match in [COMMENT_MARKER_RE.search(remote_comment.get("body") or "")]
            if match
        }
        for comment in issue.comments:
            marker = (issue.source_path, comment.index, comment.sha256)
            if marker in existing_comment_markers:
                continue
            client.create_comment(
                number,
                render_comment(
                    issue, comment, number_by_source, inventory.aliases
                ),
            )

    for issue in selected:
        if issue.parent_source is None:
            continue
        child = manifest["items"][issue.source_path]["github"]
        parent = manifest["items"][issue.parent_source]["github"]
        current_parent = client.get_parent(int(child["number"]))
        if current_parent is None:
            client.add_sub_issue(int(parent["number"]), int(child["id"]))
        elif int(current_parent["number"]) != int(parent["number"]):
            raise MigrationError(
                f"#{child['number']} has unexpected parent "
                f"#{current_parent['number']}; expected #{parent['number']}"
            )

    for issue in selected:
        child = manifest["items"][issue.source_path]["github"]
        existing_blockers = {
            int(blocker["number"]): blocker
            for blocker in client.list_blockers(int(child["number"]))
        }
        expected_numbers = {
            int(manifest["items"][source]["github"]["number"])
            for source in issue.blockers
        }
        extras = set(existing_blockers) - expected_numbers
        if extras:
            raise MigrationError(
                f"#{child['number']} has unexpected blockers: "
                + ", ".join(f"#{number}" for number in sorted(extras))
            )
        for blocker_source in issue.blockers:
            blocker = manifest["items"][blocker_source]["github"]
            if int(blocker["number"]) not in existing_blockers:
                client.add_blocker(int(child["number"]), int(blocker["id"]))

    selected_sources = {issue.source_path for issue in selected}
    parents_to_order = {
        issue.parent_source
        for issue in selected
        if issue.parent_source is not None
    }
    for parent_source in sorted(parents_to_order):
        assert parent_source is not None
        expected_children = [
            child
            for child in _expected_children(inventory, parent_source)
            if child.source_path in selected_sources
            or manifest["items"][child.source_path]["github"].get("number")
        ]
        if len(expected_children) < 2:
            continue
        parent_number = int(
            manifest["items"][parent_source]["github"]["number"]
        )
        previous_id = int(
            manifest["items"][expected_children[0].source_path]["github"]["id"]
        )
        for child in expected_children[1:]:
            child_id = int(manifest["items"][child.source_path]["github"]["id"])
            client.reprioritize_sub_issue(
                parent_number, child_id, after_database_id=previous_id
            )
            previous_id = child_id

    write_manifest(manifest_path, manifest)
    reconcile_migration(
        inventory=inventory,
        manifest=manifest,
        features=features,
        client=client,
    )
    return manifest


def reconcile_migration(
    *,
    inventory: Inventory,
    manifest: dict[str, Any],
    features: set[str] | None,
    client: GitHubClient,
) -> dict[str, int]:
    selected = _selected_issues(inventory, features)
    errors: list[str] = []
    for issue in selected:
        github = manifest["items"][issue.source_path]["github"]
        if not github.get("number"):
            errors.append(f"{issue.source_path}: no GitHub mapping")
            continue
        remote = client.get_issue(int(github["number"]))
        remote_labels = {label["name"] for label in remote.get("labels", [])}
        if remote.get("title") != issue.github_title:
            errors.append(f"#{remote['number']}: title mismatch")
        if set(issue.labels) != remote_labels:
            errors.append(f"#{remote['number']}: label mismatch")
        expected_state, expected_reason = issue.expected_state
        if remote.get("state") != expected_state:
            errors.append(f"#{remote['number']}: state mismatch")
        if expected_reason and remote.get("state_reason") != expected_reason:
            errors.append(f"#{remote['number']}: state reason mismatch")
        marker = SOURCE_MARKER_RE.search(remote.get("body") or "")
        if not marker or marker.group("sha") != issue.sha256:
            errors.append(f"#{remote['number']}: source marker mismatch")

        if issue.parent_source is not None:
            parent = client.get_parent(int(remote["number"]))
            expected_parent = int(
                manifest["items"][issue.parent_source]["github"]["number"]
            )
            if parent is None or int(parent["number"]) != expected_parent:
                errors.append(f"#{remote['number']}: parent mismatch")

        actual_blockers = {
            int(blocker["number"])
            for blocker in client.list_blockers(int(remote["number"]))
        }
        expected_blockers = {
            int(manifest["items"][source]["github"]["number"])
            for source in issue.blockers
        }
        if actual_blockers != expected_blockers:
            errors.append(f"#{remote['number']}: blocker mismatch")

        existing_comments = {
            (
                match.group("path"),
                int(match.group("index")),
                match.group("sha"),
            )
            for comment in client.list_comments(int(remote["number"]))
            for match in [COMMENT_MARKER_RE.search(comment.get("body") or "")]
            if match
        }
        for comment in issue.comments:
            marker_key = (issue.source_path, comment.index, comment.sha256)
            if marker_key not in existing_comments:
                errors.append(
                    f"#{remote['number']}: missing imported comment {comment.index}"
                )

    for parent in selected:
        expected_children = [
            child
            for child in _expected_children(inventory, parent.source_path)
            if manifest["items"][child.source_path]["github"].get("number")
        ]
        if not expected_children:
            continue
        parent_github = manifest["items"][parent.source_path]["github"]
        if not parent_github.get("number"):
            continue
        actual_children = client.list_sub_issues(int(parent_github["number"]))
        actual_numbers = [int(child["number"]) for child in actual_children]
        expected_numbers = [
            int(manifest["items"][child.source_path]["github"]["number"])
            for child in expected_children
        ]
        if actual_numbers != expected_numbers:
            errors.append(
                f"#{parent_github['number']}: ordered child list mismatch"
            )

    if errors:
        raise MigrationError("reconciliation failed:\n- " + "\n- ".join(errors))
    return {
        "issues": len(selected),
        "prds": sum(issue.kind == "prd" for issue in selected),
        "open": sum(issue.expected_state[0] == "open" for issue in selected),
        "closed": sum(issue.expected_state[0] == "closed" for issue in selected),
        "comments": sum(len(issue.comments) for issue in selected),
    }


def _repo_from_remote(root: Path) -> str:
    result = subprocess.run(
        ["git", "remote", "get-url", "origin"],
        cwd=root,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    remote = result.stdout.strip()
    patterns = (
        re.compile(r"git@github\.com:(?P<repo>[^/]+/[^/]+?)(?:\.git)?$"),
        re.compile(r"https://github\.com/(?P<repo>[^/]+/[^/]+?)(?:\.git)?$"),
    )
    for pattern in patterns:
        match = pattern.match(remote)
        if match:
            return match.group("repo")
    raise MigrationError(f"origin is not a GitHub repository: {remote}")


def _summary(inventory: Inventory, manifest: dict[str, Any]) -> dict[str, Any]:
    return {
        "repo": manifest["repo"],
        "canonical_items": len(inventory.issues),
        "prds": sum(issue.kind == "prd" for issue in inventory.issues),
        "issue_slices": sum(issue.kind == "issue" for issue in inventory.issues),
        "open": sum(issue.expected_state[0] == "open" for issue in inventory.issues),
        "closed": sum(
            issue.expected_state[0] == "closed" for issue in inventory.issues
        ),
        "comments": sum(len(issue.comments) for issue in inventory.issues),
        "legacy_aliases": len(inventory.aliases),
        "mapped": len(_number_map(manifest)),
    }


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group()
    action.add_argument(
        "--apply",
        action="store_true",
        help="create/update GitHub issues and reconcile the selected set",
    )
    action.add_argument(
        "--reconcile",
        action="store_true",
        help="read GitHub and verify the selected set without mutating it",
    )
    parser.add_argument(
        "--repo",
        help="GitHub OWNER/REPO; defaults to the origin remote",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=Path(".agent/Tasks/github-issue-migration.json"),
    )
    parser.add_argument(
        "--feature",
        action="append",
        default=[],
        help="limit apply/reconcile to a feature directory; repeatable",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    root = Path.cwd().resolve()
    try:
        inventory = discover_inventory(root)
        repo = args.repo or _repo_from_remote(root)
        existing = read_manifest(args.manifest)
        manifest = build_manifest(inventory, repo, existing=existing)
        write_manifest(args.manifest, manifest)
        features = set(args.feature) or None

        if args.apply:
            client = GitHubClient(repo)
            manifest = apply_migration(
                root=root,
                inventory=inventory,
                manifest_path=args.manifest,
                manifest=manifest,
                features=features,
                client=client,
            )
            print(json.dumps(_summary(inventory, manifest), indent=2))
            return 0
        if args.reconcile:
            client = GitHubClient(repo)
            client.verify_access()
            result = reconcile_migration(
                inventory=inventory,
                manifest=manifest,
                features=features,
                client=client,
            )
            print(json.dumps(result, indent=2))
            return 0

        print(json.dumps(_summary(inventory, manifest), indent=2))
        print(f"Dry run only. Manifest written to {args.manifest}.")
        return 0
    except (MigrationError, OSError, subprocess.CalledProcessError, json.JSONDecodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
