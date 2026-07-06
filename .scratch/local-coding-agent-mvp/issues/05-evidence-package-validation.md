Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp/PRD.md

## What to build

Build Evidence Package collection and validation. A Local Agent session cannot move to Frontier Reviewer evaluation until changed files, diff summary, commands run, test results, risks, and proposed context updates are present or explicitly marked not applicable.

## Acceptance criteria

- [x] Evidence Packages can be recorded for a Local Agent session.
- [x] Missing required evidence blocks review with clear validation errors.
- [x] Evidence links are stored without embedding bulky raw evidence in mission Markdown.
- [x] Known risks and proposed context updates are visible in the review summary.
- [x] Evidence validation result is persisted in mission state.

## Blocked by

- .scratch/local-coding-agent-mvp/issues/03-launch-local-agent-session.md
