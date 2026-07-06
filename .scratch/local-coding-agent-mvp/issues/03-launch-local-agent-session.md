Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp/PRD.md

## What to build

Build the Local Agent launch path for approved, unblocked Issue Slices. The Orchestrator should create or simulate an isolated worktree outside the target repo, produce a narrow task packet, enforce launch readiness, and record the Local Agent session lifecycle.

## Acceptance criteria

- [x] Only approved Issue Slices with satisfied blockers can launch.
- [x] Launch creates a mission-scoped Local Agent session record tied to one primary Issue Slice.
- [x] The task packet contains the slice goal, acceptance criteria, allowed paths, command policy, evidence requirements, and assignment.
- [x] The isolated worktree path is outside the target repo.
- [x] Accepted or completed worktrees are marked cleanup-eligible rather than deleted.

## Blocked by

- .scratch/local-coding-agent-mvp/issues/02-review-locking-and-assignment.md
