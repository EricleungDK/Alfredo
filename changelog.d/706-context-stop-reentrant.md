### Fixed

- A `/go` run stopped because a worker prompt was larger than `--num-ctx`
  (`context-too-small`) can now be continued: after a restart with a larger `--num-ctx`,
  `/repair ID retry` runs the retry and the held dependents to one `alfredo/go-*` branch,
  and `--resume --headless` retries the stopped task without planning again. The headless
  `failed` entry names the stopped task, and stderr prints a `next:` line to raise
  `--num-ctx` (issue #706).
