### Added

- `alfredo-tui --go GOAL --headless` runs autopilot without a terminal: one stderr
  progress line per state change, one JSON result object on stdout
  (`{state, goal, branch, accepted, total, repairs, failed}`), exit 0 when every task is
  accepted, 1 for partial/failed/interrupted runs and 2 for setup or usage errors.
  SIGINT/SIGTERM cancel workers like `/stop`. The no-terminal refusal now points to
  `--headless`.
