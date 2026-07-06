Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp-development/PRD.md

## What to build

Add TUI support for reviewing completed Local Agent sessions and routing repair decisions. The user should be able to inspect evidence, select a review outcome, see the resulting repair or PR-readiness path, and trust that the mission state updates consistently.

## Acceptance criteria

- [x] The TUI shows sessions that are ready for review and links them to their Issue Slice and Evidence Package.
- [x] The user can record Approved, Approved with limitations, Needs repair, Needs human review, and Rejected outcomes from the TUI.
- [x] Approved outcomes require valid evidence and are blocked when evidence is missing or invalid.
- [x] Rejection routing follows the repair policy for same-agent repair, fresh-agent repair, architect revision, or user escalation.
- [x] Review outcomes update the Issue Slice, session, mission summary, and timeline consistently.
- [x] Tests cover each review outcome and repair route through TUI-accessible core actions.

## Blocked by

- .scratch/local-coding-agent-mvp-development/issues/07-automated-evidence-collection.md
