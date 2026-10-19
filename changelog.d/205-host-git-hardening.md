### Security

- Alfredo's own host-side Git calls (workspace status, HEAD and repository checks,
  the plan linter's baseline listing, and the qualification fixture commit) no longer run
  a repository's `core.fsmonitor` command or hooks. They now pass the same command-line
  overrides as every other Git call (`core.hooksPath=/dev/null`, `core.fsmonitor=false`,
  `core.quotePath=false`), which outrank repository-local config.
- Starting `/go` no longer runs `git status` in a repository whose config defines filter
  drivers or includes (`filter.*`, `include.*`, `includeif.*`), because a clean filter
  would run repository-chosen commands. The check covers both the local config and the
  per-worktree config (`.git/config.worktree`). The uncommitted-changes warning reports
  those changes as not checked instead. Worker starts, the doctor and dependency
  composition read the same listing, so they refuse those entries and custom merge
  drivers in either scope.
