### Added

- `/clean` lists the retained worktrees of finished runs (accepted, rejected, failed,
  cancelled) with their total size, and `/clean yes` removes them with
  `git worktree remove` and `git worktree prune`. Active, interrupted and awaiting-review
  runs, and runs whose saved evidence or candidate ref is missing, are kept; evidence,
  `refs/alfredo/candidates/*`, `/branch` and `alfredo/go-*` branches are untouched. It is
  refused while autopilot or a worker is active. README and the reference now document
  worktrees, `.git/worktrees` entries and `refs/alfredo/*`.
