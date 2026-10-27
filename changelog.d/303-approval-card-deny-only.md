### Fixed

- A long or multi-line `run_command` card, or an edit with a very long path, no longer
  scrolls its header off so that `y`/`n` never arm. `y` and `a` now arm only when the whole
  card (header, args, every `$` line, key line) is on screen, counted in display columns;
  a command too long to show makes the card deny-only with a note to enlarge the terminal
  or press `n`.
