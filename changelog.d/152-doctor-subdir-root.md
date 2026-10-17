### Fixed

- `--doctor` run from a repository subdirectory, or with `--workspace` pointing at one,
  now checks the repository root like the TUI does instead of failing with `Choose the
  repository root with --workspace`. The storage check runs against that root and
  `PASS worker workspace:` names it.
