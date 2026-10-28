### Security

- Chat git tools now refuse, before git runs, a FIFO, socket, device or link anywhere under
  the repository's git and common directories (`rebase-merge/`, `rebase-apply/`,
  `sequencer/`, `modules/`, `worktrees/*`; git's `fsmonitor--daemon.ipc` socket stays
  allowed), and `git_log` and `git_diff` with a target refuse a config with `include.path`
  or `includeIf` as `git_status` does, so none of them waits out the 15 s limit (#465).
