### Fixed

- `git_branch` and the ignore check behind `list_files`/`grep` refuse a repository config with
  `include.path` or `includeIf` before git runs, and `core.attributesFile`, `core.excludesFile`,
  `diff.orderFile` and `mailmap.file` in the repository config are overridden for the
  read-tier git tools, so a FIFO named there no longer stalls a call for 15 s. A repository's
  own `core.excludesFile` is no longer applied by these tools, so `git_status` and
  `list_files`/`grep` discovery (when the user has no global excludes file) also show files
  it used to hide, and a committed `.mailmap` is no longer read by `git_log` (#474).
