### Security

- Host-side Git no longer reads submodule (gitlink) directories in a worker worktree. A
  check could turn an empty gitlink path into a nested repository whose clean filter ran
  on the host, outside the sandbox; every host Git call now ignores submodules.
- A worker run whose index or HEAD holds a submodule (`160000`) entry after the check is
  refused, with the path named in the run's evidence, before any diff or candidate is
  saved.
