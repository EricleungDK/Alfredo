### Fixed

- `--max-repairs` is documented, refused and listed in `--help` as applying only to `--go`
  and the terminal. The README, `docs/reference.md`, `--help` and the `--chat`/`--doctor`
  refusal no longer say it works with `--resume`, which keeps the paused run's own repair
  budget and exits 2 on an explicit one. `--on-finish` still applies to `--resume`.
