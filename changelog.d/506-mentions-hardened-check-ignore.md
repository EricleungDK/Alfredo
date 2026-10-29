### Fixed

- `@path` attach and completion now run the ignore check through the same hardened git path
  as the chat tools (config includes refused, git directory pinned). A repository config
  naming a FIFO as `core.excludesFile` or `include.path` no longer stalls an `@path` for
  seconds: it is refused at once with a message naming the setting. A repository
  `core.excludesFile` that is an ordinary file still hides the files it lists (#506).
- An excludes file whose path is not valid UTF-8 is refused, naming `core.excludesFile`,
  rather than passed to git in a lossy form that would silently skip it. A workspace that is
  a subdirectory of a larger repository is refused with a message saying it is not the top of
  a git repository; the ignore check does not look in the parent (#506).
