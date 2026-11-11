### Fixed

- `grep` no longer answers `no matches` for files it cannot read. A search that skipped files over 4 MiB,
  non-UTF-8 files or binary files ends with `[not searched: big.log (over 4 MiB), latin.txt (not UTF-8);
  use run_command grep for these]`, and `grep` with a `path` naming one returns that file's read error.
  Skipped over-limit files no longer use up the scan budget.
