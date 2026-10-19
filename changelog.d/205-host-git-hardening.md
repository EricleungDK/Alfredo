### Security

- Alfredo's own host-side Git calls (workspace status, HEAD and repository checks,
  the plan linter's baseline listing, and the qualification fixture commit) no longer run
  a repository's `core.fsmonitor` command or hooks. They now pass the same command-line
  overrides as every other Git call (`core.hooksPath=/dev/null`, `core.fsmonitor=false`,
  `core.quotePath=false`), which outrank repository-local config.
