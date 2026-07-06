Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp/PRD.md

## What to build

Build the first usable mission state path: a CLI/TUI-first mission command can load the local Product Requirements Document and Issue Slice markdown records, derive the Issue Graph, show mission status, and persist app-local runtime state without requiring GitHub.

## Acceptance criteria

- [x] The user can run a local command and see the current Product Requirements Document title, Issue Slice count, blockers, and readiness summary.
- [x] Issue Slices are loaded from local markdown tracker records and ordered by dependency.
- [x] App-local runtime state is persisted outside the target repo and keyed by project identity.
- [x] The target repo is not polluted with bulky runtime state.
- [x] Missing or malformed issue records produce actionable errors instead of crashes.

## Blocked by

None - can start immediately
