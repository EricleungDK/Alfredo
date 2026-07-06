Status: ready-for-agent
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Keep the Agent Console transcript and Activity Journal meaningful after the console-first redesign. Prompts, assistant/controller responses, consequential actions, command/grant approval decisions, assignments, outcomes, and evidence links should be reconstructable, while routine UI navigation and raw telemetry remain outside durable records unless summarized as evidence.

## Acceptance criteria

- [ ] Durable Agent Console history includes prompts, assistant/controller responses, consequential actions, command/grant approval decisions, and outcomes.
- [ ] Routine issue-board navigation, card selection, filtering, sorting, expansion, and evidence viewing stay out of the prompt transcript unless they trigger a consequential action.
- [ ] Activity Journal records meaningful attributed command, grant, assignment, approval, outcome, and evidence events.
- [ ] Raw token streams and terminal bytes are excluded from durable prompt history and Activity Journal entries unless summarized as evidence.
- [ ] Journal entries remain searchable or inspectable enough to reconstruct relevant command, grant, assignment, approval, and outcome decisions.
- [ ] Transcript and Activity Journal tests cover inclusion boundaries, exclusion boundaries, attribution, and evidence-link behavior.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/02-inline-command-execution-cards.md
- .scratch/alfredo-console-first-workstation-redesign/issues/03-inline-approvals-and-contextual-path-grants.md
- .scratch/alfredo-console-first-workstation-redesign/issues/06-governed-issue-assignment-and-launch-actions.md
