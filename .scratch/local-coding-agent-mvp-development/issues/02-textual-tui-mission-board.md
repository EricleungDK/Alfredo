Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp-development/PRD.md

## What to build

Add the first production TUI mission board for operating a local coding mission. The TUI should show the Issue Graph, readiness, blockers, assignment, lifecycle state, and next action in one app-native surface so the user does not have to infer mission status from generated markdown or raw runtime files.

## Acceptance criteria

- [x] The user can launch a TUI command from the local command surface.
- [x] The TUI shows ordered Issue Slices with status, blockers, assignment, and launch readiness.
- [x] The TUI highlights the next recommended action for the mission.
- [x] The TUI can show details for a selected Issue Slice, including acceptance criteria and blocked-by information.
- [x] Empty, missing, or malformed tracker state is handled with a readable error view.
- [x] Tests cover the TUI data model and rendering-ready board state without requiring fragile terminal snapshots.

## Blocked by

- .scratch/local-coding-agent-mvp-development/issues/01-lifecycle-state-cleanup-and-reopen-controls.md
