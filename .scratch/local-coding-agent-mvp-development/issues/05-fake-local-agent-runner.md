Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp-development/PRD.md

## What to build

Add a deterministic fake Local Agent runner that exercises the full Orchestrator loop without invoking a real model. A fake-agent launch should create the isolated worktree, write the task packet, emit a log artifact, simulate a completion result, and produce a valid Evidence Package that can move to review.

## Acceptance criteria

- [x] A configured fake Local Agent can be assigned to and launched for an approved ready Issue Slice.
- [x] Launch creates an isolated worktree outside the target repo and writes a readable task packet.
- [x] The fake runner records a durable log artifact and a deterministic completion result.
- [x] The fake runner produces a valid Evidence Package without manual evidence flags.
- [x] The session lifecycle moves from launched to completed or evidence-ready in persisted runtime state.
- [x] Tests cover the fake runner end-to-end from assignment through evidence validation.

## Blocked by

- .scratch/local-coding-agent-mvp-development/issues/04-tui-assignment-and-launch-controls.md
