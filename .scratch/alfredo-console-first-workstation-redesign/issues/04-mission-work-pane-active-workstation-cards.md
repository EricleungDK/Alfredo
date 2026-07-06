Status: complete
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Create the Mission Work side pane with Active Workstations as the top region and primary live-supervision surface. The Mission Commander should be able to see active, blocked, waiting-approval, reviewing, review-ready, failed, done, and idle workstation state while continuing to prompt in the Agent Console.

## Acceptance criteria

- [x] The side pane is labeled and structured as Mission Work rather than a tab switcher between Workstations and Shell Terminal.
- [x] Active Workstations are the top region and remain visible while the Mission Commander prompts.
- [x] Workstation cards show agent or subagent identity, Issue Slice, actual model, role, current state, last meaningful activity, latest command/test summary, blocker or next action, and review/evidence affordances when present.
- [x] Blocked and waiting-approval cards sort above routine active work while other states remain visually distinct.
- [x] Workstation cards remain the primary live-supervision model and are not replaced by a dense table.
- [x] Tests verify card projection, sorting priority, state distinctions, and visibility alongside the Agent Console.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/01-console-first-workstation-layout.md

## Comments

- Implemented the Issue 04 frontend slice at the existing Mission Work card seam. Active Workstations remain the top side-pane region while the Agent Console stays visible and prompt-first.
- Workstation cards now show explicit Issue Slice metadata and distinguish canonical `idle` state from active thinking/running states.
- Projection tests cover blocked/waiting-approval sorting above routine work plus distinct tones for waiting, blocked, failed, reviewing, running, idle, and review-ready cards.
- App tests cover Issue Slice visibility and review/evidence affordances beside the Agent Console without moving review authority out of Review Workspace.
- Verification: `npm test -- --run workstation-projection.test.ts` passes 11 tests; `npm test -- --run App.test.tsx` passes 58 tests; `npm test -- --run` passes 118 frontend tests; `npm run typecheck` passes; `npm run build` passes; `python3 -m unittest discover -s tests` passes 180 tests with 1 skip; `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` passes; `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` passes 27 Rust tests.
