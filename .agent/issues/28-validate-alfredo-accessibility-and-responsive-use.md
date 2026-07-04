# Validate Alfredo Accessibility and Responsive Use

Status: ready-for-human
Type: HITL

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Harden Alfredo's prompt-first workstation so dense live agent supervision remains usable for keyboard, screen-reader, low-vision, motion-sensitive, and constrained-width users. This slice should combine automated coverage with human validation because the final hierarchy and assistive-technology comprehension require judgment.

## Acceptance criteria

- [ ] Prompt operation, card navigation, expansion, approvals, review actions, and diff opening are keyboard-operable with logical focus order and visible focus.
- [ ] Semantic regions, card labels, status announcements, action names, stale-state explanations, and error messages are understandable to screen-reader users.
- [ ] Text, controls, compact badges, focus indicators, and semantic status colors meet WCAG AA contrast expectations.
- [ ] Reduced-motion preferences disable nonessential motion while preserving live status comprehension.
- [ ] Constrained-width tests prove the prompt remains primary and critical workstation decisions remain reachable.
- [ ] A human reviewer confirms the workstation hierarchy, keyboard flow, screen-reader comprehension, zoom/reflow behavior, and low-vision readability.

## Blocked by

- `22-build-prompt-dominant-workstation-shell.md`
- `24-expand-workstation-cards-for-operational-detail.md`
- `26-cover-governed-workstation-action-family.md`

