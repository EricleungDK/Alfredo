
### Shared context and agent orchestration

Before planning or implementing any change:

1. Read `.agent/README.md`.
2. Read `.agent/Tasks/context.md`.
3. Read the relevant docs under `.agent/System/`, `.agent/SOP/`, and `.agent/Tasks/`.
4. If there is no active planning artifact, read the relevant GitHub PRD parent and its Issue Slice sub-issues.

This repository uses `.agent/Tasks/context.md` as the source of truth for agent orchestration:

- The active mission/issue slice focus.
- Active model roles and assignments for this cycle.
- Pending blockers, risks, approvals, and next actions.
- A short log of local orchestration-relevant decisions.

Before planning or coding, verify `.agent/Tasks/context.md` contains a fresh `## Active Orchestration Context` section for this run. If the block is stale or incomplete, update it first when the run changes durable project orchestration state. A bounded scheduled-maintenance run may use its explicit GitHub trigger as the current run context and must not create a context-only change when its correct outcome is no change.
After significant planning or implementation, update `.agent/Tasks/context.md` with any changes to the above before returning to coding.

An explicit current user request or GitHub issue, pull-request, or scheduled-automation trigger is the authority for that run. Completed-run restrictions retained in an older orchestration block are historical constraints, not standing prohibitions on a newly authorized run. Preserve their factual history, but do not let stale `do not push`, `do not create a pull request`, or similar wording override the current trigger.

### Issue tracker

GitHub Issues is the authoritative tracker. Each PRD is a `[PRD]` parent issue with ordered native Issue Slice sub-issues and native dependency edges. External PRs are not a triage surface. `.scratch/` is a read-only migration archive. See `docs/agents/issue-tracker.md`.

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

### Persistent Apple container development workstation

On the macOS development host, use the repository's persistent Apple `container` environment for manual browser viewing and user acceptance:

```bash
./scripts/apple-container-dev status
./scripts/apple-container-dev start
```

The canonical browser workstation is `http://127.0.0.1:1420`. Prefer `restart` after a process-level change and leave the named `alfredo-dev` container running for the user unless they ask to stop it. Do not start a competing host `npm run dev` process on port 1420 or use Docker/Compose. Use the persistent workstation for human visual testing; continue to run focused automated checks when implementation risk requires them, but do not launch an additional browser merely to reproduce a surface the user is already inspecting unless the task specifically requires automated browser evidence.
