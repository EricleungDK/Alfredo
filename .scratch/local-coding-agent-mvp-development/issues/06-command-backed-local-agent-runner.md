Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp-development/PRD.md

## What to build

Run a configured command-backed Local Agent inside the isolated worktree. The Orchestrator should pass the task packet to the runner, stream or capture logs, record exit status, enforce configured command policy, and prevent the runner from becoming an untracked editing path.

## Acceptance criteria

- [x] A command-backed Local Agent can be configured, assigned, and launched for an approved ready Issue Slice.
- [x] The runner receives the task packet and runs with the isolated worktree as its working context.
- [x] Runner stdout, stderr, exit status, and start/end times are recorded as session artifacts.
- [x] Command policy is checked before runner execution and human-required commands are blocked.
- [x] The session records success, failure, or needs-human-review without losing logs.
- [x] Tests cover successful execution, nonzero exit, missing command, and command-policy blocking.

## Blocked by

- .scratch/local-coding-agent-mvp-development/issues/05-fake-local-agent-runner.md
