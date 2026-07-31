from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from scripts.migrate_local_issues_to_github import (
    Inventory,
    LocalIssue,
    MigrationError,
    apply_migration,
    build_manifest,
    discover_inventory,
    reconcile_migration,
    render_body,
)


ROOT = Path(__file__).resolve().parents[1]


class FakeGitHubClient:
    def __init__(self) -> None:
        self.next_number = 1
        self.issues: dict[int, dict] = {}
        self.comments: dict[int, list[dict]] = {}
        self.parents: dict[int, int] = {}
        self.children: dict[int, list[int]] = {}
        self.blockers: dict[int, set[int]] = {}
        self.created_labels = False

    def verify_access(self) -> dict:
        return {"has_issues": True}

    def ensure_labels(self) -> None:
        self.created_labels = True

    def list_issues(self) -> list[dict]:
        return list(self.issues.values())

    def create_issue(self, *, title: str, body: str, labels: tuple[str, ...]) -> dict:
        number = self.next_number
        self.next_number += 1
        issue = {
            "number": number,
            "id": 1000 + number,
            "html_url": f"https://github.test/example/repo/issues/{number}",
            "title": title,
            "body": body,
            "labels": [{"name": label} for label in labels],
            "state": "open",
            "state_reason": None,
        }
        self.issues[number] = issue
        self.comments[number] = []
        self.children[number] = []
        self.blockers[number] = set()
        return issue

    def update_issue(
        self,
        number: int,
        *,
        title: str,
        body: str,
        labels: tuple[str, ...],
        state: str,
        state_reason: str | None,
    ) -> dict:
        issue = self.issues[number]
        issue.update(
            {
                "title": title,
                "body": body,
                "labels": [{"name": label} for label in labels],
                "state": state,
                "state_reason": state_reason,
            }
        )
        return issue

    def get_issue(self, number: int) -> dict:
        return self.issues[number]

    def list_comments(self, number: int) -> list[dict]:
        return list(self.comments[number])

    def create_comment(self, number: int, body: str) -> dict:
        comment = {"body": body}
        self.comments[number].append(comment)
        return comment

    def get_parent(self, number: int) -> dict | None:
        parent_number = self.parents.get(number)
        return self.issues[parent_number] if parent_number is not None else None

    def add_sub_issue(self, parent_number: int, child_database_id: int) -> dict:
        child_number = child_database_id - 1000
        self.parents[child_number] = parent_number
        self.children[parent_number].append(child_number)
        return self.issues[child_number]

    def list_sub_issues(self, parent_number: int) -> list[dict]:
        return [self.issues[number] for number in self.children[parent_number]]

    def reprioritize_sub_issue(
        self,
        parent_number: int,
        child_database_id: int,
        *,
        after_database_id: int,
    ) -> dict:
        child_number = child_database_id - 1000
        after_number = after_database_id - 1000
        children = self.children[parent_number]
        children.remove(child_number)
        children.insert(children.index(after_number) + 1, child_number)
        return self.issues[child_number]

    def list_blockers(self, number: int) -> list[dict]:
        return [self.issues[value] for value in sorted(self.blockers[number])]

    def add_blocker(self, number: int, blocker_database_id: int) -> dict:
        blocker_number = blocker_database_id - 1000
        self.blockers[number].add(blocker_number)
        return self.issues[blocker_number]


class InventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.inventory = discover_inventory(ROOT)
        cls.by_source = cls.inventory.by_source

    def test_discovers_only_canonical_prds_and_issue_slices(self) -> None:
        self.assertEqual(40, len(self.inventory.issues))
        self.assertEqual(4, sum(item.kind == "prd" for item in self.inventory.issues))
        self.assertEqual(
            36, sum(item.kind == "issue" for item in self.inventory.issues)
        )
        self.assertFalse(
            any(item.source_path.endswith("README.md") for item in self.inventory.issues)
        )

    def test_preserves_directory_hierarchy_and_nested_prd(self) -> None:
        child = self.by_source[
            ".scratch/local-coding-agent-mvp/issues/"
            "04-command-and-visibility-policy.md"
        ]
        self.assertEqual(
            ".scratch/local-coding-agent-mvp/PRD.md", child.parent_source
        )
        nested_prd = self.by_source[
            ".scratch/local-coding-agent-mvp-development/PRD.md"
        ]
        self.assertEqual(
            ".scratch/local-coding-agent-mvp/PRD.md", nested_prd.parent_source
        )

    def test_resolves_relative_and_legacy_duplicate_blockers(self) -> None:
        relative = self.by_source[
            ".scratch/alfredo-agent-workstation/"
            "24-expand-workstation-cards-for-operational-detail.md"
        ]
        self.assertEqual(
            (
                ".scratch/alfredo-agent-workstation/"
                "23-project-live-agent-workstation-cards.md",
            ),
            relative.blockers,
        )
        legacy = self.by_source[
            ".scratch/alfredo-console-first-workstation-redesign/issues/"
            "01-console-first-workstation-layout.md"
        ]
        self.assertEqual(
            (
                ".scratch/alfredo-agent-workstation/"
                "29-add-alfredo-release-seam-verification.md",
            ),
            legacy.blockers,
        )
        self.assertEqual(11, len(self.inventory.aliases))

    def test_derives_titles_for_legacy_files_without_h1(self) -> None:
        issue = self.by_source[
            ".scratch/local-coding-agent-mvp/issues/"
            "04-command-and-visibility-policy.md"
        ]
        self.assertEqual("04 — Command and Visibility Policy", issue.github_title)
        prd = self.by_source[".scratch/local-coding-agent-mvp/PRD.md"]
        self.assertEqual("[PRD] Local Coding Agent MVP", prd.github_title)
        development_prd = self.by_source[
            ".scratch/local-coding-agent-mvp-development/PRD.md"
        ]
        self.assertEqual(
            "[PRD] Local Coding Agent MVP Development Roadmap",
            development_prd.github_title,
        )
        self.assertEqual(
            4,
            len(
                {
                    item.github_title
                    for item in self.inventory.issues
                    if item.kind == "prd"
                }
            ),
        )

    def test_preserves_lifecycle_and_comments(self) -> None:
        open_items = [
            item for item in self.inventory.issues if item.expected_state[0] == "open"
        ]
        self.assertEqual(4, len(open_items))
        self.assertEqual(
            36,
            sum(
                item.expected_state == ("closed", "completed")
                for item in self.inventory.issues
            ),
        )
        self.assertGreater(
            sum(len(item.comments) for item in self.inventory.issues), 10
        )

    def test_rendered_body_uses_provenance_and_github_references(self) -> None:
        issue = self.by_source[
            ".scratch/alfredo-console-first-workstation-redesign/PRD.md"
        ]
        number_by_source = {
            ".scratch/alfredo-agent-workstation/"
            "29-add-alfredo-release-seam-verification.md": 42
        }
        body = render_body(issue, number_by_source, self.inventory.aliases)
        self.assertIn("local-issue-source:", body)
        self.assertIn("#42", body)
        self.assertNotIn(
            ".agent/issues/29-add-alfredo-release-seam-verification.md", body
        )


class MigrationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.inventory = discover_inventory(ROOT)

    def test_apply_is_idempotent_and_reconciles_full_hierarchy(self) -> None:
        client = FakeGitHubClient()
        manifest = build_manifest(self.inventory, "example/repo")
        with tempfile.TemporaryDirectory() as temporary:
            manifest_path = Path(temporary) / "manifest.json"
            first = apply_migration(
                root=ROOT,
                inventory=self.inventory,
                manifest_path=manifest_path,
                manifest=manifest,
                features=None,
                client=client,
            )
            issue_count = len(client.issues)
            comment_count = sum(len(comments) for comments in client.comments.values())

            second = apply_migration(
                root=ROOT,
                inventory=self.inventory,
                manifest_path=manifest_path,
                manifest=build_manifest(self.inventory, "example/repo", first),
                features=None,
                client=client,
            )

        self.assertTrue(client.created_labels)
        self.assertEqual(40, issue_count)
        self.assertEqual(issue_count, len(client.issues))
        self.assertEqual(
            comment_count, sum(len(comments) for comments in client.comments.values())
        )
        self.assertEqual(40, len(second["items"]))
        result = reconcile_migration(
            inventory=self.inventory,
            manifest=second,
            features=None,
            client=client,
        )
        self.assertEqual(
            {"issues": 40, "prds": 4, "open": 4, "closed": 36},
            {key: result[key] for key in ("issues", "prds", "open", "closed")},
        )

    def test_manifest_refuses_changed_source_after_remote_mapping(self) -> None:
        issue = self.inventory.issues[0]
        existing = build_manifest(self.inventory, "example/repo")
        existing["items"][issue.source_path]["github"] = {
            "number": 1,
            "id": 1001,
            "url": "https://github.test/example/repo/issues/1",
        }
        changed = LocalIssue(
            **{
                **issue.__dict__,
                "sha256": "0" * 64,
            }
        )
        changed_inventory = Inventory(
            issues=(changed, *self.inventory.issues[1:]),
            aliases=self.inventory.aliases,
        )
        with self.assertRaisesRegex(MigrationError, "source changed"):
            build_manifest(changed_inventory, "example/repo", existing)

    def test_manifest_is_json_serializable(self) -> None:
        manifest = build_manifest(self.inventory, "example/repo")
        encoded = json.dumps(manifest)
        self.assertIn('"schema_version": 1', encoded)
        self.assertIn('"legacy_agent_issues_included": false', encoded)


if __name__ == "__main__":
    unittest.main()
