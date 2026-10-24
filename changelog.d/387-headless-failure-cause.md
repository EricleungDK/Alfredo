### Fixed

- Headless `--go` failures now carry their cause: `failed[].reason` and the stderr line end with the task's last failure line (new additive `last_failure` field), and each repair progress line names its trigger (`check failed`, `no files changed` or `worker failed`) instead of always `check failed`.
