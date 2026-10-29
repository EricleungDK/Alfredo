### Fixed

- Chat `list_files` and `grep` now start with a `[note: .gitignore not applied (...)]` line
  naming the cause when a repository config include or filter key (for example
  `git lfs install --local`) makes git refuse the listing and the plain walk is used.
  What is read and the secret filtering are unchanged (#511).
