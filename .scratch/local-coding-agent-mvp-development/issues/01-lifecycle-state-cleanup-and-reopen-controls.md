Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp-development/PRD.md

## What to build

Make the mission lifecycle explicit and recoverable across tracker markdown, app-local runtime state, board output, and launch behavior. The user should be able to understand why a slice is blocked, reopen a completed or stuck slice deliberately, and move a slice through approval, launch, completion, and repair states without contradictory status messages.

## Acceptance criteria

- [x] The board and launch path use the same lifecycle rules for approved, blocked, launched, PR-ready, complete, and repair states.
- [x] The user can inspect one Issue Slice and see its tracker status, runtime status, review state, blockers, and next valid actions.
- [x] A completed Issue Slice cannot be silently re-approved for launch; reopening is explicit and recorded.
- [x] Reopening a slice resets only the state needed for re-review and does not erase assignment notes, evidence history, or mission timeline entries.
- [x] Invalid state transitions produce actionable errors without Python tracebacks.
- [x] Tests cover approval, launch blocking, completion, reopening, and tracker/runtime precedence.

## Blocked by

None - can start immediately
