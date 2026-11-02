### Fixed

- Chat git writes (`git_commit`, `git_switch`, `git_merge`) work in a repository with an
  initialised submodule: `submodule.<name>.url` and a boolean `submodule.<name>.active`
  in the repository config are now allowed, where they were refused as unsafe config. Every
  other submodule key (`update`, `ignore`, `branch`, `submodule.recurse`,
  `fetch.recurseSubmodules`) stays refused (#539).
