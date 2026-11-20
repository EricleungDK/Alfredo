### Fixed

- An explicit `--on-finish` or `--max-repairs` with `--chat` or `--doctor` now exits 2
  naming the flag and where it applies (`--go`, `--resume` or the terminal), instead of
  being silently ignored. `ALFREDO_ON_FINISH` is still ignored there. `--help` and the
  reference say which modes the two flags apply to.
