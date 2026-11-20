### Added

- `--chat --headless --output-format stream-json` now prints a `{"v":1,"type":"text","delta":"..."}`
  line for each chunk of the model's reply as it arrives, in order with the `tool` events and
  before the final `result` line, so a wrapper can show the reply live and spot a stalled
  model. The default output and `--go` streams are unchanged; no `text` events follow a stop.
