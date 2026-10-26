### Security

- Chat git tools (`git_status`, `git_branch`, `git_log`, `git_diff`, and the local and push
  writes `git_commit`, `git_switch`, `git_merge`, `git_push`) now refuse a Coding Workspace
  whose `.git` points at another repository, by gitfile or by a `commondir` file. Only the
  workspace's own repository, or a linked worktree of it, is read or written.
- `run_command` binds a linked worktree's shared Git directory into the sandbox only when
  the workspace's repository verifies that way, so another repository's config (which can
  hold remote tokens), objects and hooks are no longer readable from the sandbox.
