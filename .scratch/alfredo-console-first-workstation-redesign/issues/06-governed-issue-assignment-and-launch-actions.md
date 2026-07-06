Status: Completed
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Wire consequential Issue Assignment Board and workstation-card actions through the same typed Orchestrator governance path as Agent Console actions. Launches, assignments, scope-affecting changes, retry, repair, review, and related decisions should show pending intent immediately when useful, but accepted state should appear only after Orchestrator acknowledgement.

## Acceptance criteria

- [x] Unassigned ready issues offer governed launch or assignment actions only when the Orchestrator says those actions are valid.
- [x] Consequential actions submit typed Orchestrator requests with action type, actor, target identity, expected revision, and reason where required.
- [x] Accepted consequential actions create visible human-readable Agent Console or orchestrator turns.
- [x] Stale issue assignment actions explain the current state and a recovery path rather than failing ambiguously.
- [x] Routine navigation, filtering, sorting, evidence opening, and detail expansion do not append durable prompt transcript turns.
- [x] Governance tests cover assignment, launch, retry, repair, review escalation, model assignment, and scope-affecting actions through typed validation.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/05-issue-assignment-board-projection-and-local-navigation.md

## Comments

- Added canonical Issue Assignment Board governed actions: only unassigned-ready rows derived from Orchestrator launch eligibility expose Launch and Assign model controls, and those controls submit typed workstation action requests with Mission Commander actor, target identity, expected revision, and required assignment reason/agent id.
- Added direct review-ready workstation card controls for accept, repair, and human escalation. These reuse the Review Workspace decision path and preserve pending/acknowledged Agent Console turns while reloading accepted state from the canonical snapshot.
- Extended explicit Conversation Scope changes to carry action type, actor, and target identity metadata through React, Tauri, and the Python CLI, with backend validation rejecting mismatched target identity before state mutation.
- Routine board selection, card expansion, filtering, sorting, evidence/diff opening, and detail navigation remain local UI actions and do not append durable prompt transcript turns.
- Note: `.agent/issues/29-add-alfredo-release-seam-verification.md` is now registered as Mission Commander-approved and Completed; Issue 05 is also locally marked Completed.
- 2026-07-06 verification pass: fixed a stale `CommandDeck` prop path for workstation action status focus and updated Review Workspace request expectations to include the required typed governance metadata (`action_type`, actor, and target identity).
- Verification: `npm test -- --run workstation-projection.test.ts App.test.tsx` passes 76 tests; `npm run typecheck` passes; `python3 -m unittest tests.test_workspace_snapshot` passes 100 tests; `npm test -- --run` passes 125 frontend tests; `npm run build` passes; `python3 -m unittest discover -s tests` passes 181 tests with 1 skip; `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` passes; `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` passes 27 Rust bridge tests.
