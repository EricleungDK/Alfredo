### Fixed

- Chat git refusals say what is wrong and how to proceed: a config include no longer claims
  `git_log` and `git_diff` still work (it refuses every chat git read tool), and a symbolic
  link, FIFO, socket or device in the git directory is named by its path relative to the git
  directory (control characters as `<U+XXXX>`) with advice to remove it or ask for an
  escalated `run_command`.
