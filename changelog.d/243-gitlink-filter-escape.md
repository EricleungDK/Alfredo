### Security

- Host-side Git no longer reads submodule (gitlink) directories in a worker worktree.
  A check could turn an empty gitlink path into a nested repository whose clean filter
  ran on the host, outside the sandbox; every host Git call now ignores submodules.
- A worker run whose baseline commit, index or HEAD holds a submodule (`160000`) entry
  is refused, with the path named in the run's evidence. The baseline is refused before
  the check runs; the index and HEAD are refused again after it, before any diff or
  candidate is saved.
