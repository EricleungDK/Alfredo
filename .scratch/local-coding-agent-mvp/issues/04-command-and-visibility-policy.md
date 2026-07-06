Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp/PRD.md

## What to build

Build Orchestrator-enforced command and visibility policy for Local Agent sessions. Commands must resolve to auto-allowed, frontier-approvable, or human-required, and files must resolve to Normal, Local-only, or Blocked for Frontier Model access.

## Acceptance criteria

- [x] Commands are classified before execution and blocked or marked approval-required when policy demands it.
- [x] Project-specific command approvals can be recorded in app-local runtime state.
- [x] File visibility classification is available to task packet and review generation.
- [x] Local-only files cause Frontier Reviewer limitations to be explicit.
- [x] Blocked files are excluded from Frontier Model context.

## Blocked by

- .scratch/local-coding-agent-mvp/issues/03-launch-local-agent-session.md
