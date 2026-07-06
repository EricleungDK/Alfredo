Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp/PRD.md

## What to build

Build PR readiness and GitHub fallback behavior. Frontier approval means PR-ready, not merge-approved; the Orchestrator prepares a readable PR summary using GitHub automation when available and local instructions when it is not.

## Acceptance criteria

- [x] Approved or Approved with limitations slices can be marked PR-ready.
- [x] One Issue Slice maps to one PR by default.
- [x] Branch naming includes mission and Issue Slice identity.
- [x] PR summaries include Issue Slice, changed behavior, acceptance criteria, evidence, Frontier Review, and Local Agent activity.
- [x] If `gh` is available and authenticated, the tool can produce the command it would use for PR creation.
- [x] If `gh` is missing or unauthenticated, the tool generates local PR-ready instructions.
- [x] The tool never marks a slice merge-approved or auto-merges to main.

## Blocked by

- .scratch/local-coding-agent-mvp/issues/07-mission-record-generation.md
