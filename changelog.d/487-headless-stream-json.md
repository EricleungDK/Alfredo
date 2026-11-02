### Added

- `--output-format stream-json` for `--headless` (`--go`, `--resume`, `--chat`): stdout becomes one
  JSON object per line (`{"v":1,"type":"task_started"|"repair"|"check"|"tool"|"result",...}`), ending
  with the usual final object as the `result` line. Without the flag the output is unchanged; any other
  value exits 2.
