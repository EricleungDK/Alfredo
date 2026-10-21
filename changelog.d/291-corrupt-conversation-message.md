### Fixed

- A malformed, oversized or unsupported conversation file now fails with the absolute
  file path and one way out (`mv "<path>" "<path>.bad"` or `--conversation NAME`) in the
  TUI, headless runs and the `--doctor` storage line. The file is never modified.
