# Validate Accessible and Responsive Command Deck Workflows

Status: ready-for-human
Type: HITL

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Harden and validate the completed Mission Control App workflows against the approved Command Deck direction. Ensure the dense desktop experience remains understandable, keyboard-operable, screen-reader-friendly, responsive at constrained widths, and safe for motion-sensitive and low-vision users, then obtain human confirmation of the resulting interaction hierarchy.

## Acceptance criteria

- [ ] Core journeys are fully keyboard-operable with logical focus order, visible focus, managed focus for inspectors and decisions, and no pointer-only actions.
- [ ] Semantic regions, labels, status announcements, and error messages make live workflows understandable with a screen reader.
- [ ] Text, controls, state accents, and focus indicators meet WCAG AA contrast; lime and cyan accents communicate consistent meaning rather than decoration.
- [ ] Reduced-motion preferences disable nonessential motion, and zoom/reflow preserve content and controls.
- [ ] Wide and constrained desktop layouts keep Agent Console or Shell Terminal, current Conversation Scope, the primary operational object, pending governance, and actionable errors reachable.
- [ ] Dangerous or irreversible actions remain visually distinct, while borders and uppercase metadata remain subordinate to primary content.
- [ ] Automated accessibility checks and keyboard journey tests cover Conversation Scope, Issue Graph inspection, Workspace Queue decisions, Review Workspace, Activity, and Shell Terminal.
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
