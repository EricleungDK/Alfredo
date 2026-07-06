Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp/PRD.md

## What to build

Build the Frontier Reviewer decision path and tiered repair policy. Review outcomes should be evidence-based and should update the Issue Slice, Local Agent session, and mission summary consistently.

## Acceptance criteria

- [x] Review outcomes support Approved, Approved with limitations, Needs repair, Needs human review, and Rejected.
- [x] Missing or invalid evidence prevents Approved outcomes.
- [x] First rejection routes repair to the same Local Agent.
- [x] Second rejection routes repair to a fresh Local Agent.
- [x] Repeated architecture failure routes back to Frontier Architect revision.
- [x] Critical, security, or merge-risk failure escalates to the user.

## Blocked by

- .scratch/local-coding-agent-mvp/issues/05-evidence-package-validation.md
