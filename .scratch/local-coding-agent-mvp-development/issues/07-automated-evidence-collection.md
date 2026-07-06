Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp-development/PRD.md

## What to build

Automate Evidence Package collection from Local Agent sessions. After a runner completes, the Orchestrator should collect changed files, diff summary, command logs, test results or not-applicable markers, known-risk placeholders, and artifact links so review is grounded in concrete run output.

## Acceptance criteria

- [x] Evidence collection derives changed files from the isolated worktree.
- [x] Evidence collection records a diff summary or an explicit no-diff result.
- [x] Runner logs and relevant command output are linked as artifacts rather than embedded into mission markdown.
- [x] Test results are captured when configured or marked not applicable with a reason.
- [x] Known risks and proposed context updates are present as explicit fields, even when empty.
- [x] Tests cover evidence collection for changed files, no changes, test success, test failure, and missing optional evidence.

## Blocked by

- .scratch/local-coding-agent-mvp-development/issues/06-command-backed-local-agent-runner.md
