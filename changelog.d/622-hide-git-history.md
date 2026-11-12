### Security

- Sandboxed `run_command` no longer reads the Coding Workspace's `.git`: a secret that was
  committed and later removed, or stashed, could be printed through `git show`, `git log -p`,
  `git cat-file` or `git stash show` even though the working-tree file was masked (#622).
  The root `.git` is now hidden from sandboxed runs, so git there reports `not a git
  repository`; use the chat git tools (`git_status`, `git_branch`, `git_log`, `git_diff`)
  instead. Escalated (host) runs are unchanged.
