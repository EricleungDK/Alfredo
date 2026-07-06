Status: Completed
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Add the Issue Assignment Board below Active Workstations as a compact ownership and coverage matrix. It should show relevant Issue Slices, ownership, lifecycle and readiness state, blocker state, and current workstation/session linkage while keeping issue browsing local to the side pane unless the Mission Commander explicitly changes Conversation Scope.

## Acceptance criteria

- [x] The Issue Assignment Board appears below Active Workstations and shows every relevant Issue Slice.
- [x] Each row shows Issue Slice identity, title or concise label, owner or assigned agent, assignment state, lifecycle/readiness state, blocker state, and linked workstation/session when present.
- [x] Unassigned ready work, blocked work, active work, review-ready work, complete work, and failed work are distinguishable.
- [x] Selecting an issue row focuses local side-pane detail without changing Conversation Scope or appending a prompt transcript turn.
- [x] Issue rows offer an explicit scope-change action when a selected issue can become the next prompt target.
- [x] Disabled actions explain why they are disabled, and blocked issues show blocker summaries.
- [x] Projection tests verify the board derives accepted owner/state/workstation linkage from canonical mission or session data and does not invent accepted assignment state before Orchestrator acknowledgement.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/04-mission-work-pane-active-workstation-cards.md

## Comments

- Implemented the Issue Assignment Board as the lower Mission Work matrix below Active Workstations. Rows project every ordered Issue Slice from canonical mission board/session data with owner, assignment state, readiness/lifecycle, blocker summaries, and workstation/session linkage.
- Added board-local issue detail focus so row browsing stays inside the side pane and does not append Agent Console transcript turns or silently retarget Conversation Scope.
- Added explicit per-row `Set scope` controls that submit through the existing Orchestrator-backed Conversation Scope acknowledgement path; disabled scope actions show visible reasons.
- Projection coverage now distinguishes unassigned-ready, blocked, active, review-ready, complete, merged, and failed rows without inventing accepted ownership before canonical assignment/session data exists.
- Mission Commander approval for `.agent/issues/29-add-alfredo-release-seam-verification.md` is registered, so this ticket is marked Completed with its blocker status resolved locally.
- Verification: `npm test -- --run workstation-projection.test.ts App.test.tsx` passes 72 tests; `npm run typecheck` passes; `npm test -- --run` passes 121 frontend tests; `npm run build` passes; `python3 -m unittest discover -s tests` passes 180 tests with 1 skip; `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` passes; `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` passes 27 Rust bridge tests.
