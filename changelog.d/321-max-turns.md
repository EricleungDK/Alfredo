### Added

- `--max-turns N` (env `ALFREDO_MAX_TURNS`, 1-32, default 8) sets how many tool rounds one
  chat turn may run, in the TUI and with `--chat --headless`. The stop notice and the
  headless `error` and `steps` use the actual cap; out of range or non-numeric exits 2.
