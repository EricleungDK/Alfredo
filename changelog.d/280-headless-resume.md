### Added

- `--resume --headless` continues the mission's paused autopilot run without a terminal,
  with the same progress lines, JSON result and exit codes as `--go`; it exits 2 when
  there is nothing to resume.

### Changed

- `--go GOAL --headless` over a mission with a paused run now exits 2, naming the paused
  goal and its progress, instead of planning a second task family beside it. Use
  `--resume --headless` to continue it, or `--conversation NAME` for a separate run.
