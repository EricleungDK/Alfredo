# Route the First Consequential Workstation Action

Status: ready-for-agent
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Prove the core governance loop with one end-to-end consequential side-pane action, preferably an approval decision on a waiting workstation card. The action should submit a typed Orchestrator request, append a human-readable prompt/orchestrator turn, wait for authoritative acknowledgement, update the card from canonical state, and record meaningful Activity Journal evidence.

## Acceptance criteria

- [ ] A waiting workstation card exposes approve, reject, and defer actions only when the Orchestrator says those actions are currently valid.
- [ ] The chosen action submits a typed request containing action type, actor, target identity, expected revision, and reason when required.
- [ ] The prompt pane shows a human-readable workstation-action turn and the live Orchestrator/controller reaction for validation, rejection, acceptance, or failure.
- [ ] The card exposes pending, accepted, rejected, failed, stale, and disabled states without treating a click as accepted state before acknowledgement.
- [ ] The Activity Journal records the meaningful attributed action and outcome without storing raw transient streams as domain events.

## Blocked by

- `23-project-live-agent-workstation-cards.md`

