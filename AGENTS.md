You are in ubuntu environment

## Agent skills

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

Before planning or coding, verify `.agent/Tasks/context.md` contains a fresh `## Active Orchestration Context` section for this run. If the block is stale or incomplete, update it first.
After significant planning or implementation, update `.agent/Tasks/context.md` with any changes to the above before returning to coding.

### Issue tracker

GitHub Issues is the authoritative tracker. Each PRD is a `[PRD]` parent issue with ordered native Issue Slice sub-issues and native dependency edges. External PRs are not a triage surface. `.scratch/` is a read-only migration archive. See `docs/agents/issue-tracker.md`.

### Triage labels

The default five-label triage vocabulary is used unchanged. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repo with a root `CONTEXT.md`. See `docs/agents/domain.md`.

### User instruction

After each implementation, provide user the instruction reply with the commands that they can use for viewing the current coding results, i.e. application GUI skeleton.
