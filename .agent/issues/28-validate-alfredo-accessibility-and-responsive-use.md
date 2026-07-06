# Validate Alfredo Accessibility and Responsive Use

Status: ready-for-human
Type: HITL

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Harden Alfredo's prompt-first workstation so dense live agent supervision remains usable for keyboard, screen-reader, low-vision, motion-sensitive, and constrained-width users. This slice should combine automated coverage with human validation because the final hierarchy and assistive-technology comprehension require judgment.

## Acceptance criteria

- [x] Prompt operation, card navigation, expansion, approvals, review actions, and diff opening are keyboard-operable with logical focus order and visible focus.
- [x] Semantic regions, card labels, status announcements, action names, stale-state explanations, and error messages are understandable to screen-reader users.
- [x] Text, controls, compact badges, focus indicators, and semantic status colors meet WCAG AA contrast expectations.
- [x] Reduced-motion preferences disable nonessential motion while preserving live status comprehension.
- [x] Constrained-width tests prove the prompt remains primary and critical workstation decisions remain reachable.
- [ ] A human reviewer confirms the workstation hierarchy, keyboard flow, screen-reader comprehension, zoom/reflow behavior, and low-vision readability.

## Blocked by

- `22-build-prompt-dominant-workstation-shell.md`
- `24-expand-workstation-cards-for-operational-detail.md`
- `26-cover-governed-workstation-action-family.md`

## Comments

### 2026-07-06 — automated hardening complete

- Prompt Composer is now a named region, and active execution is exposed as a live status near the prompt.
- Workstation cards are keyboard-focusable named articles with screen-reader summaries, explicit status descriptions, `aria-expanded` expansion controls, `aria-pressed` pin state, and disabled-action explanations.
- Reduced-motion mode removes nonessential animation and transition effects while textual status remains available.
- The constrained layout adds a 520px breakpoint for stacked prompt composer controls, readable status chips, reflowed workstation cards, and reachable queue/review/card decisions.
- Audited contrast ratios range from 6.10:1 to 16.33:1 for base text, compact labels, semantic accents, warning/danger actions, and focus indicators.
- Fresh automated gates: `npm test -- App.test.tsx -t "workstation|contrast" --run` (9 passed), `npm run typecheck`, `npm test -- --run` (110 passed), `python3 -m unittest discover -s tests` (176 passed), `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` (27 passed), and `npm run build`.
- Remaining gate: human confirmation of workstation hierarchy, keyboard flow, screen-reader comprehension, zoom/reflow, and low-vision readability.
- Audit report: `.agent/Reports/2026-07-06-alfredo-accessibility-responsive-workstation.md`.
