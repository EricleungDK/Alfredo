### Fixed

- `/repair ID <hint>` after a `/go` run ended `failed` or `partial` now re-enters the
  autopilot loop: the repair is approved, run and auto-reviewed, dependents unblock and the
  run integrates on an `alfredo/go-<id>` branch. It adds one repair to its family's budget;
  a second failure ends `failed` again with the same `Last failure:` and `Next:` lines.
  `/resume` on a finished run now names `/repair ID <hint>` or `/go GOAL`.
