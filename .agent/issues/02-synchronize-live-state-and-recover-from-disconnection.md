# Synchronize Live State and Recover from Disconnection

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Keep the Command Deck synchronized with authoritative Orchestrator state after its initial snapshot. Deliver ordered batched updates, semantic action acknowledgements, stale-action protection, and reconnection recovery as one observable desktop workflow.

## Acceptance criteria

- [x] The client applies ordered batched state updates after the initial snapshot without losing entity identity or displaying contradictory state.
- [x] Semantic actions carry correlation and expected-state information, and the UI distinguishes pending, acknowledged, rejected, and stale outcomes.
- [x] A stale or invalid action is rejected by the Orchestrator and leaves accepted mission state unchanged.
- [x] Disconnecting shows a compact offline or reconnecting state; reconnecting obtains a fresh canonical snapshot rather than assuming missed events were received.
- [x] Contract and desktop tests cover ordered events, event lag, stale actions, malformed data, disconnect, reconnect, and resynchronization.

## Blocked by

- `01-open-and-restore-command-deck-workspace-session.md`

## Progress

- 2026-06-20: Issue 01 dependency completed. Began the approved TDD plan with the Orchestrator stale-action immutability tracer bullet.
- 2026-06-20: Backend TDD checkpoint complete: correlated expected-revision actions, atomic ordered event persistence, malformed/gap rejection, JSON CLI action/update boundaries, and stale-state immutability pass 12 focused integration tests.
- 2026-06-20: Added typed Rust/Tauri action and update commands with a real Python action→update integration test; default-feature Rust suite passes 8 tests.
- 2026-06-20: Added React ordered-batch reduction, pending/acknowledged/stale/rejected action states, update polling, compact offline state, and fresh-snapshot reconnect; frontend passes 16 tests, typecheck, and production build.
- 2026-06-20: Ticket integration passed 79 Python tests, 16 frontend tests, 8 Rust tests, typecheck/build, and a real CLI action→updates→stale-action→fresh-snapshot smoke sequence.
