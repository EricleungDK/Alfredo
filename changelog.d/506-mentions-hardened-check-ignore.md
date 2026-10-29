### Fixed

- `@path` attach and completion now run the ignore check through the same hardened git path
  as the chat tools (config includes refused, git directory pinned). A repository config
  naming a FIFO as `core.excludesFile` or `include.path` no longer stalls an `@path` for
  seconds: it is refused at once with a message naming the setting. A repository
  `core.excludesFile` that is an ordinary file still hides the files it lists (#506).
