### Changed

- A collapsed `run_command` result row now reads `exit code 1 · FAILED test_x … (17 lines)`:
  the exit code and the last non-empty output line, from stderr on a non-zero exit and
  from stdout otherwise, with hidden characters shown as `<U+XXXX>`. Other tools' rows are
  unchanged, and the model still receives the full result.
