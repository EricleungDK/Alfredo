Status: ready-for-agent
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Add the Issue Assignment Board below Active Workstations as a compact ownership and coverage matrix. It should show relevant Issue Slices, ownership, lifecycle and readiness state, blocker state, and current workstation/session linkage while keeping issue browsing local to the side pane unless the Mission Commander explicitly changes Conversation Scope.

## Acceptance criteria

- [ ] The Issue Assignment Board appears below Active Workstations and shows every relevant Issue Slice.
- [ ] Each row shows Issue Slice identity, title or concise label, owner or assigned agent, assignment state, lifecycle/readiness state, blocker state, and linked workstation/session when present.
- [ ] Unassigned ready work, blocked work, active work, review-ready work, complete work, and failed work are distinguishable.
- [ ] Selecting an issue row focuses local side-pane detail without changing Conversation Scope or appending a prompt transcript turn.
- [ ] Issue rows offer an explicit scope-change action when a selected issue can become the next prompt target.
- [ ] Disabled actions explain why they are disabled, and blocked issues show blocker summaries.
- [ ] Projection tests verify the board derives accepted owner/state/workstation linkage from canonical mission or session data and does not invent accepted assignment state before Orchestrator acknowledgement.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/04-mission-work-pane-active-workstation-cards.md
