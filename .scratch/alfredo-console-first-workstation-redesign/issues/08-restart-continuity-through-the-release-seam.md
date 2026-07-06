Status: complete
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Extend task 29's launch-to-restore release seam so the redesigned console-first workstation remains usable after restart. Meaningful continuity state should restore across the Agent Console, Mission Work pane, Active Workstations, Issue Assignment Board, approvals, evidence links, and side-pane focus without replaying transient animation or raw terminal output.

## Acceptance criteria

- [x] Restart restore covers selected workspace, recent workspaces, selected controller/model, Conversation Scope, prompt transcript, active sessions, approvals, evidence links, active workstation state, and Activity Journal availability.
- [x] Meaningful Mission Work continuity restores, including selected workstation, selected issue, useful expanded/detail state, and side-pane state.
- [x] Transient hover, animation, and raw terminal bytes are not persisted as restored UI state.
- [x] The existing release-seam journey demonstrates console-first layout, Active Workstations, Issue Assignment Board, a governed action, visible console/orchestrator turns, Activity Journal evidence, and restored meaningful state after restart.
- [x] Persistence tests verify canonical accepted state is restored without showing unacknowledged assignments or launches as accepted.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/04-mission-work-pane-active-workstation-cards.md
- .scratch/alfredo-console-first-workstation-redesign/issues/05-issue-assignment-board-projection-and-local-navigation.md
- .scratch/alfredo-console-first-workstation-redesign/issues/07-activity-journal-and-durable-transcript-boundaries.md

## Comments

- Extended workstation continuity persistence so restart restores meaningful Mission Work state: selected issue with Issue Assignment Board vs Mission Board focus, selected workstation/session, expanded and pinned cards, workstation filter/sort, selected diff, and command audit side-pane state.
- The launch-to-restore release seam now verifies console-first layout, launch context, prompt transcript, Active Workstations, Issue Assignment Board detail focus, governed launch acknowledgement, Activity Journal evidence, and restored meaningful state after restart.
- Backend restart coverage verifies canonical accepted state restores selected workspace, Conversation Scope, active session, pending approval, evidence link, durable Agent Console history, and Activity Journal entries while excluding raw terminal bytes from durable records.
- Post-review hardening preserves review-decision action type, actor, and target identity through React, Tauri, and Python CLI validation before mutating accepted mission state.
- Verification: `npm test -- --run App.test.tsx alfredo-release-seam.test.tsx` passes 63 tests; `python3 -m unittest tests.test_workspace_snapshot` passes 101 tests; `npm run typecheck` passes; `npm test -- --run` passes 125 tests; `python3 -m unittest discover -s tests` passes 183 tests with 1 skip; `npm run build` passes; `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` passes; and `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` passes 28 tests.
