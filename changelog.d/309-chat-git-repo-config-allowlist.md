### Security

- Chat git writes (commit, switch, merge and push) are refused when the repository's own
  git config sets a key that can run a program or redirect a push, such as a signing key
  command, a git proxy, a pager, a remote helper or an HTTP proxy. Only a short list of
  harmless keys is allowed; the error names the key.
- `git_push` now sends only to `https://`, `ssh://` and scp-style remotes. A remote that
  is a local path, `file://`, `git://`, `http://` or `ext::` is refused, so a repository's
  own hooks never run on a push. Pushes send only the named branch, never tags or
  submodules.
- The read-only chat git tools (`git_status` and working-tree `git_diff`) refuse a
  repository whose config sets `core.worktree`, which could have shown files from
  outside the workspace.
- A change to the repository config or to a remote URL after a git write card is shown
  now refuses the approval, as a change to HEAD or the files already does.
