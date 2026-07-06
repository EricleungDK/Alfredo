Status: ready-for-agent
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Wire consequential Issue Assignment Board and workstation-card actions through the same typed Orchestrator governance path as Agent Console actions. Launches, assignments, scope-affecting changes, retry, repair, review, and related decisions should show pending intent immediately when useful, but accepted state should appear only after Orchestrator acknowledgement.

## Acceptance criteria

- [ ] Unassigned ready issues offer governed launch or assignment actions only when the Orchestrator says those actions are valid.
- [ ] Consequential actions submit typed Orchestrator requests with action type, actor, target identity, expected revision, and reason where required.
- [ ] Accepted consequential actions create visible human-readable Agent Console or orchestrator turns.
- [ ] Stale issue assignment actions explain the current state and a recovery path rather than failing ambiguously.
- [ ] Routine navigation, filtering, sorting, evidence opening, and detail expansion do not append durable prompt transcript turns.
- [ ] Governance tests cover assignment, launch, retry, repair, review escalation, model assignment, and scope-affecting actions through typed validation.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/05-issue-assignment-board-projection-and-local-navigation.md
