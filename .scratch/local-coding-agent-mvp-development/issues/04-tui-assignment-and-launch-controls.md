Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp-development/PRD.md

## What to build

Let the user assign configured agents and launch approved ready Issue Slices from the TUI while preserving the existing CLI path. The TUI should act as the mission-control surface for common review, assignment, and launch decisions rather than only displaying state.

## Acceptance criteria

- [x] The TUI shows available configured agents for an Issue Slice assignment decision.
- [x] The user can assign or override a slice's Local Agent from the TUI.
- [x] The user can approve and launch an eligible slice from the TUI.
- [x] Blocked or invalid launches are prevented in the TUI with the same error reason as the CLI.
- [x] TUI assignment and launch actions persist to runtime state and are visible after restart.
- [x] Tests cover assignment, approval, launch, blocked launch, and persisted TUI action results through the same core APIs used by the CLI.

## Blocked by

- .scratch/local-coding-agent-mvp-development/issues/02-textual-tui-mission-board.md
- .scratch/local-coding-agent-mvp-development/issues/03-agent-model-configuration-registry.md
