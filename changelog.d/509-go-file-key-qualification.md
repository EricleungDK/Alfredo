### Fixed

- `/go` and `--go` now refuse a repository whose config sets `core.excludesFile`,
  `core.attributesFile`, `mailmap.file` or `diff.orderFile`, naming the key, before any
  worktree, worker or repair is created. A FIFO there used to stall host git for 15 s and
  then fail every worker.
