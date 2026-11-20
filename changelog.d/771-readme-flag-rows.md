### Fixed

- The root README `--output-format` row now lists the `text` event for `--chat` (matching
  the reference event table), and the `--max-repairs` and `--on-finish` rows say they apply
  only to `--go`, `--resume` and the terminal (an explicit one with `--chat` or `--doctor`
  exits 2). A docs test now keeps these rows in step with `docs/reference.md`.
