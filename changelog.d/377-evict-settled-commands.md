### Fixed

- A long-lived conversation no longer stops `/go` with `Command history capacity
  reached` (headless `--go` paused on about the 25th run in one conversation). At the
  512-command / 512 KiB budget the oldest settled commands (refused, interrupted or
  finished) are dropped; live pending and submitted commands, selections and the parents
  an automatic `/run` or Architect draft still needs are kept. The footer and headless
  stderr warn at 80% and name `/clear`.
