### Fixed

- `git_branch` and the ignore check behind `list_files`/`grep` refuse a repository config with
  `include.path` or `includeIf` before git runs, and `core.attributesFile`, `core.excludesFile`
  and `mailmap.file` in the repository config are overridden for the read-tier git tools, so a
  FIFO named there no longer stalls a call for 15 s. A repository's own `core.excludesFile` is
  no longer applied by these tools (#474).
