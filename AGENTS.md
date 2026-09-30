
### Shared context and agent orchestration

Before planning or implementing any change:

1. Read `.agent/README.md` and `.agent/Tasks/STATUS.md` (current state; `.agent/Tasks/context.md` is a historical log, not a source of truth).
2. Read the relevant docs under `.agent/System/`, `.agent/SOP/`, and `.agent/Tasks/`.
3. If there is no active planning artifact, read the relevant GitHub PRD parent and its Issue Slice sub-issues.

Update `.agent/Tasks/STATUS.md` after significant work changes durable project state.

An explicit current user request or GitHub issue, pull-request, or scheduled-automation trigger is the authority for that run. Completed-run restrictions retained in older notes are historical constraints, not standing prohibitions on a newly authorized run. Preserve their factual history, but do not let stale `do not push`, `do not create a pull request`, or similar wording override the current trigger.

### Issue tracker

GitHub Issues is the authoritative tracker. Each PRD is a `[PRD]` parent issue with ordered native Issue Slice sub-issues and native dependency edges. External PRs are not a triage surface. See `docs/agents/issue-tracker.md`.

### GitHub issue instruction

GitHub is authoritative. Match the GitHub access method to the execution environment:

- In a local or CLI checkout, request network escalation on the first live GitHub command, use authenticated `gh` for issue/PR operations, and infer the repository from the configured remote. A sandbox DNS/API failure is an environment restriction, not a product failure.
- In a GitHub-triggered Codex Cloud run, treat the repository, issue or pull request, selected branch, and checked-out commit supplied by the cloud task as authoritative. The checkout may intentionally have no configured Git remote, remote-tracking refs, authenticated `gh`, or `GH_TOKEN`; their absence is not a failed precondition. Use the connected GitHub integration and the cloud task's result/publishing surface for issue reporting and pull-request handoff.
- Codex Cloud authenticates through the user's ChatGPT session. Never request, create, or require an `OPENAI_API_KEY` for a native Codex Cloud run.
- For a scheduled issue-triggered run, the final response is the issue report delivered by the connector. Do not require a separate `gh issue comment` command. If the connected environment cannot perform a requested GitHub mutation, report that exact publishing limitation after completing every safe read-only analysis and verification step; do not substitute API-key or GitHub Actions infrastructure.

### Triage labels

The default five-label triage vocabulary is used unchanged. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repo with a root `CONTEXT.md`. See `docs/agents/domain.md`.
