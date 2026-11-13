### Added

- A finished `/go` summary now has a `Changes` row under `Branch`, for example
  `Changes   1 file   +1 −1`, and the headless result gains
  `"changes":{"files":N,"added":A,"deleted":D}`. Both come from one `git diff --numstat`
  from the run's baseline to the integration branch, taken when the run finishes and
  saved with it, so a restart shows the same numbers. A run that made no branch has
  neither.
