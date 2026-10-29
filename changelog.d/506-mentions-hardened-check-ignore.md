### Fixed

- `@path` attach and completion now run the ignore check through the same hardened git path
  as the chat tools (config includes refused, repository file keys such as
  `core.excludesFile` pointed at the null device, git directory pinned). A repository
  config naming a FIFO no longer stalls an `@path` for seconds, and an `include.path`
  refuses the mention at once with a message naming the include (#506).
