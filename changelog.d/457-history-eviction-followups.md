### Fixed

- Command-history eviction follow-ups. A conversation no longer stops with `Command
  history capacity reached` after 8192 lifetime commands while only a few hundred are
  kept: command and task-receipt numbers are renumbered in order at the cap. A duplicate
  request correlation is refused at the budget as it is under it, instead of being
  admitted after its original is evicted. Admitting a command at the budget measures the
  history a bounded number of times (about 3x faster in a release build) and the
  headless 80% check runs only when a command count changes. An evicted command no longer
  leaves the reader's scroll anchor dangling. The `/go` acknowledgement stays in the
  footer beside the 80% warning, which is now shorter (`Command history 80% full; /clear
  starts a fresh one`). Command numbers can change when the history renumbers at the
  cap, so an older `/retry-command` or "command #N" notice may name a different command.
