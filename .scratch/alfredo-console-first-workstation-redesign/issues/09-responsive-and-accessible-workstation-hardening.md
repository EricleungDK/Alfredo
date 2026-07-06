Status: ready-for-agent
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Harden the redesigned workstation for constrained widths, keyboard users, screen-reader users, low-vision users, and motion-sensitive users. The prompt, composer, active workstation decisions, and issue assignment state should remain reachable and understandable without overlapping content or relying on motion.

## Acceptance criteria

- [ ] The Agent Console, Active Workstations, and Issue Assignment Board expose semantic regions with understandable accessible names.
- [ ] Keyboard focus order moves logically through the composer, active workstation cards, issue assignment rows, approvals, review actions, and detail expansion.
- [ ] Text remains readable with sufficient contrast, visible focus, and stable compact rows in the assignment matrix.
- [ ] Reduced-motion preferences disable nonessential live-status animation while preserving understandable status changes.
- [ ] Constrained-width layouts keep the prompt, composer, active workstation decisions, and issue assignment state reachable without incoherent overlap.
- [ ] Accessibility and responsive tests or documented checks cover keyboard operation, semantic hierarchy, contrast/focus, reduced motion, and constrained-width behavior.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/01-console-first-workstation-layout.md
- .scratch/alfredo-console-first-workstation-redesign/issues/02-inline-command-execution-cards.md
- .scratch/alfredo-console-first-workstation-redesign/issues/03-inline-approvals-and-contextual-path-grants.md
- .scratch/alfredo-console-first-workstation-redesign/issues/04-mission-work-pane-active-workstation-cards.md
- .scratch/alfredo-console-first-workstation-redesign/issues/05-issue-assignment-board-projection-and-local-navigation.md
- .scratch/alfredo-console-first-workstation-redesign/issues/06-governed-issue-assignment-and-launch-actions.md
- .scratch/alfredo-console-first-workstation-redesign/issues/07-activity-journal-and-durable-transcript-boundaries.md
- .scratch/alfredo-console-first-workstation-redesign/issues/08-restart-continuity-through-the-release-seam.md
