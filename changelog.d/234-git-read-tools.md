### Added

- Chat can read git state itself. The model gets read-only tools `git_status`,
  `git_branch`, `git_log` and `git_diff`, so "which branch are we on" or "what does
  alfredo/go-12 change vs main" is answered from the real repository. They never fetch
  or change refs or the index, refuse option-like refs, leave secret files out of diffs
  and cap output at 16 KiB.
