### Added

- `--on-finish PROGRAM` (env `ALFREDO_ON_FINISH`) runs your own executable, without a shell and outside the sandbox, when an autopilot run ends done, partial or failed, with the state, branch, goal and task counts in `ALFREDO_*` variables. It has a 10 s limit, a failure only shows a footer notice, and it fires once per finish.
