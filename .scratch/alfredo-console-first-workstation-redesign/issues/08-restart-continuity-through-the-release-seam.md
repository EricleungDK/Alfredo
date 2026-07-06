Status: ready-for-agent
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Extend task 29's launch-to-restore release seam so the redesigned console-first workstation remains usable after restart. Meaningful continuity state should restore across the Agent Console, Mission Work pane, Active Workstations, Issue Assignment Board, approvals, evidence links, and side-pane focus without replaying transient animation or raw terminal output.

## Acceptance criteria

- [ ] Restart restore covers selected workspace, recent workspaces, selected controller/model, Conversation Scope, prompt transcript, active sessions, approvals, evidence links, active workstation state, and Activity Journal availability.
- [ ] Meaningful Mission Work continuity restores, including selected workstation, selected issue, useful expanded/detail state, and side-pane state.
- [ ] Transient hover, animation, and raw terminal bytes are not persisted as restored UI state.
- [ ] The existing release-seam journey demonstrates console-first layout, Active Workstations, Issue Assignment Board, a governed action, visible console/orchestrator turns, Activity Journal evidence, and restored meaningful state after restart.
- [ ] Persistence tests verify canonical accepted state is restored without showing unacknowledged assignments or launches as accepted.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/04-mission-work-pane-active-workstation-cards.md
- .scratch/alfredo-console-first-workstation-redesign/issues/05-issue-assignment-board-projection-and-local-navigation.md
- .scratch/alfredo-console-first-workstation-redesign/issues/07-activity-journal-and-durable-transcript-boundaries.md
