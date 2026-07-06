Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp/PRD.md

## What to build

Build mission Markdown generation for human and AI-readable records. The app should maintain a navigable mission summary, timeline, Local Agent tracker, evidence index, Frontier Review summary, and per-Issue Slice execution records without duplicating full tracker tickets.

## Acceptance criteria

- [x] Mission records are generated inside the target repo and are git-trackable.
- [x] Runtime state and bulky evidence remain outside the target repo.
- [x] The mission summary shows current status and next action.
- [x] The timeline records meaningful state transitions and decisions, not command approval noise.
- [x] Per-Issue Slice mission records link to tracker issues and evidence artifacts.
- [x] A returning agent can identify the next action by reading the mission summary first.

## Blocked by

- .scratch/local-coding-agent-mvp/issues/06-frontier-review-and-repair-policy.md
