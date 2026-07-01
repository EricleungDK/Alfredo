# Validate Accessible and Responsive Command Deck Workflows

Status: ready-for-human
Type: HITL

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Harden and validate the completed Mission Control App workflows against the approved Command Deck direction. Ensure the dense desktop experience remains understandable, keyboard-operable, screen-reader-friendly, responsive at constrained widths, and safe for motion-sensitive and low-vision users, then obtain human confirmation of the resulting interaction hierarchy.

## Acceptance criteria

- [x] Core journeys are fully keyboard-operable with logical focus order, visible focus, managed focus for inspectors and decisions, and no pointer-only actions.
- [x] Semantic regions, labels, status announcements, and error messages make live workflows understandable with a screen reader.
- [x] Text, controls, state accents, and focus indicators meet WCAG AA contrast; lime and cyan accents communicate consistent meaning rather than decoration.
- [x] Reduced-motion preferences disable nonessential motion, and zoom/reflow preserve content and controls.
- [x] Wide and constrained desktop layouts keep Agent Console or Shell Terminal, current Conversation Scope, the primary operational object, pending governance, and actionable errors reachable.
- [x] Dangerous or irreversible actions remain visually distinct, while borders and uppercase metadata remain subordinate to primary content.
- [x] Automated accessibility checks and keyboard journey tests cover Conversation Scope, Issue Graph inspection, Workspace Queue decisions, Review Workspace, Activity, and Shell Terminal.
- [ ] A human reviewer confirms that the production interface preserves the approved Command Deck hierarchy and accessibility expectations.

## Blocked by

- `03-agent-console-with-explicit-conversation-scope.md`
- `04-inspect-and-curate-working-context.md`
- `05-switch-active-and-background-missions.md`
- `06-navigate-issue-graph-and-inspect-issue-slice.md`
- `07-ollama-provider-neutral-model-assignments.md`
- `08-review-evidence-packages.md`
- `09-resolve-change-proposals-and-frontier-confirmations.md`
- `10-approve-and-supervise-ad-hoc-delegation.md`
- `11-create-and-confirm-mission-draft.md`
- `12-search-and-reconstruct-activity-journal.md`
- `13-governed-shell-terminal-and-path-grants.md`

## Comments

### 2026-07-01 — automated hardening complete

- Public interaction tests verify named landmarks and labelled controls in both left-lane modes, arrow-key lane switching, Conversation Scope, Issue Graph inspection, Workspace Queue decisions, Review Workspace decisions, Activity filters, and governed Shell Terminal workflows.
- Focus now moves to the Issue Slice Inspector and to Queue, Review, Mission Draft, and Terminal decision outcomes. Visible focus covers buttons, inputs, textareas, selects, links, and programmatically focused result regions.
- The constrained layout keeps the left lane first, reflows Operations Workspace to one column with a horizontal view rail, stacks terminal forms, and collapses Issue Graph rows without horizontal-only access.
- Reduced-motion disables animation repetition, transitions, and smooth scrolling. Dangerous actions use explicit danger treatment; repair uses warning treatment.
- Audited contrast ratios range from 6.53:1 to 16.33:1 for base text and semantic accent combinations.
- Fresh automated gates: 157 Python tests, 70 frontend tests, TypeScript typecheck, production build, Rust formatting, and 24 Rust tests pass.
- Remaining gate: human confirmation of hierarchy, keyboard flow, assistive-technology comprehension, zoom/reflow, and low-vision expectations.
- Audit report: `.agent/Reports/2026-07-01-accessible-responsive-command-deck.md`.
