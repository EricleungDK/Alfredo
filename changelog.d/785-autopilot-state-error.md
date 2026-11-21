### Changed

- A garbled autopilot state file (an oversized task count, truncated JSON, or a file over
  its size bound) now stops opening the mission with an error that names the absolute
  state-file path and the way out: `mv "<path>" "<path>.bad"` or `--new-mission NAME`.
  The file is never modified.
