Status: ready-for-agent
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Move risky command approvals and Additional Path Grant requests into contextual Agent Console turns. Alfredo should ask for authority only when it is needed, show exactly what is being requested, and keep all accepted or denied decisions auditable without presenting grants as a standing configuration form.

## Acceptance criteria

- [ ] Commands that require approval produce inline approval prompts before execution can proceed.
- [ ] Additional Path Grant requests are raised contextually when a blocked command or agent action needs out-of-workspace access.
- [ ] Path-grant prompts include path, access level, duration, reason, and affected action.
- [ ] Agents and skills cannot create or expand grants without explicit Mission Commander authority.
- [ ] Active, denied, and expired grants are visible when inspecting relevant command, approval, Activity Journal, or detail surfaces.
- [ ] Tests cover command approval prompts, contextual grant prompts, grant decision outcomes, and absence of a standing default grant form.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/02-inline-command-execution-cards.md
