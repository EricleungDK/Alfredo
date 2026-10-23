### Fixed

- A worker run whose candidate diff against the baseline is empty no longer reaches
  review when its check passes. It now fails with `No files changed:` and the allowed
  files, so `/go` repairs it instead of reporting `done 1/1` and `--headless` exiting 0
  (#350).
