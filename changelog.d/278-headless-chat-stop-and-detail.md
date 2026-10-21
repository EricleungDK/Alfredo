### Fixed

- `--chat … --headless` no longer reports a turn cut off by the 8-round tool step cap or
  the conversation size limit as `"state":"done"` with exit 0: it is now `"state":"stopped"`
  with `"stopped":"step_limit"` or `"output_limit"` and exit 1, and the reply no longer
  tells the script to "ask again". Each failed entry in `calls[]` carries `detail` (the
  tool's error text, at most 200 bytes), and the stderr tool lines show the path and the
  reason.
