Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp-development/PRD.md

## What to build

Surface PR readiness from the TUI after a slice has passed review. The user should be able to see PR-ready slices, generate branch and PR instructions, inspect GitHub command availability, and keep final merge authority explicit.

## Acceptance criteria

- [x] The TUI shows which Issue Slices are PR-ready and which are still blocked from PR preparation.
- [x] The user can prepare PR instructions for a selected PR-ready slice from the TUI.
- [x] The generated PR summary includes Issue Slice, changed behavior, acceptance criteria, evidence, review outcome, and Local Agent activity.
- [x] The tool reports whether `gh` automation is available and falls back to manual instructions when it is not.
- [x] The TUI and CLI use the same PR readiness rules and never mark a slice merge-approved.
- [x] Tests cover PR-ready display, unavailable `gh` fallback, available `gh` command generation, and non-PR-ready blocking.

## Blocked by

- .scratch/local-coding-agent-mvp-development/issues/08-tui-review-and-repair-loop.md
