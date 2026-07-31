# Issue tracker: GitHub

GitHub Issues in `EricleungDK/Alfredo` is the authoritative tracker. Use the `gh` CLI for tracker operations.

## PRD hierarchy

The GitHub hierarchy preserves the useful directory shape of the former local tracker:

- One feature directory becomes one parent issue titled `[PRD] <feature title>`.
- A PRD parent has the `type:prd` label and one triage label while it is open.
- Approved Issue Slices become ordinary issues attached to the PRD as ordered native sub-issues.
- Keep the original `01`, `02`, ... sequence at the start of each Issue Slice title. Native sub-issue order must match that sequence.
- Blocking relationships use GitHub's native issue dependencies. Body text may explain a dependency, but it is not the authoritative edge.
- A PRD may itself be a sub-issue of a larger PRD when the product hierarchy requires it.

The PRD parent is the GitHub equivalent of `.scratch/<feature-slug>/`; its ordered child list is the equivalent of that directory's `issues/` folder.

## Common operations

- Create an issue: `gh issue create --title "..." --body-file <file> --label "..."`
- Read an issue: `gh issue view <number> --comments`
- List open issues: `gh issue list --state open --json number,title,labels,url`
- Comment: `gh issue comment <number> --body "..."`
- Change labels: `gh issue edit <number> --add-label "..." --remove-label "..."`
- Close as completed: `gh issue close <number> --reason completed`
- Close as not planned: `gh issue close <number> --reason "not planned"`

Infer the repository from `git remote -v` when working inside this clone.

For GitHub CLI versions without sub-issue and dependency flags, use the REST endpoints through `gh api`:

- Add a child: `POST repos/<owner>/<repo>/issues/<parent>/sub_issues` with the child's numeric database `issue_id`.
- Reorder a child: `PATCH repos/<owner>/<repo>/issues/<parent>/sub_issues/priority`.
- Add a blocker: `POST repos/<owner>/<repo>/issues/<issue>/dependencies/blocked_by` with the blocker's numeric database `issue_id`.

Use `gh api repos/<owner>/<repo>/issues/<number> --jq .id` to get the database ID; it is not the visible `#number` or GraphQL node ID.

## Pull requests as a triage surface

External pull requests are **not** a request or triage surface for this repository. `/triage` should operate on GitHub issues only.

GitHub shares one number space across issues and pull requests. If a bare `#42` is ambiguous, resolve it with `gh pr view 42` and then fall back to `gh issue view 42`.

## Skill vocabulary

When a skill says "publish to the issue tracker", create a GitHub issue. When publishing a PRD, create the `[PRD]` parent first; when publishing its tickets, attach and order the Issue Slice sub-issues and create their native blocker edges.

When a skill says "fetch the relevant ticket", run `gh issue view <number> --comments`. For a PRD, also inspect its native sub-issues and dependency relationships.

## Local migration archive

`.scratch/` retains the source Markdown imported on 2026-07-23. It is a read-only provenance archive, not a second tracker:

- Do not create new PRDs, Issue Slices, comments, or status changes there.
- Do not use archived Markdown state to override GitHub.
- Do not delete the archive until the team makes a separate retention decision.

The frozen migration manifest is `.agent/Tasks/github-issue-migration.json`. The historical importer and reconciler are `scripts/migrate_local_issues_to_github.py`. Do not run `--apply` after cutover as an ongoing synchronization mechanism: it treats archived Markdown as expected state and could overwrite legitimate GitHub changes. Likewise, `--reconcile` compares GitHub with the migration snapshot and is expected to report later GitHub lifecycle changes.
