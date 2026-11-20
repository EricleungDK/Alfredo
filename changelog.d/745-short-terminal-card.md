### Fixed

- On a short terminal (under 24 rows, such as a tmux half-split of 36 rows) a pending
  approval card shrinks the prompt box to one row that carries the approval hint and any
  draft, and a `run_command` card drops its `args` row when that is the row that does not
  fit and every argument is already drawn on the card (the `$` command rows, the cwd and
  timeout row and the HOST banner). `run_command touch x` can now be approved at 80x18.
  A card that still does not fit whole stays deny-only. The chat pane also keeps its last
  transcript line on screen at 10 to 12 rows by giving up the status line first.
