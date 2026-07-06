You are in ubuntu environment

## Agent skills

### Shared context and agent orchestration

Before planning or implementing any change:

1. Read `.agent/README.md`.
2. Read `.agent/Tasks/context.md`.
3. Read the relevant docs under `.agent/System/`, `.agent/SOP/`, and `.agent/Tasks/`.
4. If there is no active planning artifact, read issue PRDs / slices from `.scratch/`.

This repository uses `.agent/Tasks/context.md` as the source of truth for agent orchestration:

- The active mission/issue slice focus.
- Active model roles and assignments for this cycle.
- Pending blockers, risks, approvals, and next actions.
- A short log of local orchestration-relevant decisions.

Before planning or coding, verify `.agent/Tasks/context.md` contains a fresh `## Active Orchestration Context` section for this run. If the block is stale or incomplete, update it first.
After significant planning or implementation, update `.agent/Tasks/context.md` with any changes to the above before returning to coding.

### Issue tracker

Issues and PRDs are tracked as local markdown files under `.scratch/`. See `docs/agents/issue-tracker.md`.

### Triage labels

The default five-label triage vocabulary is used unchanged. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repo with a root `CONTEXT.md`. See `docs/agents/domain.md`.
