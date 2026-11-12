### Fixed

- A `run_command` result that has to be cut to a tight chat budget (a small `--num-ctx`
  or a nearly full conversation) now keeps the exit code, the end of stderr and the end
  of stdout, each marked `[… N earlier bytes cut]`, instead of keeping only the front of
  the report and dropping the failure reason (issue #692). Results that fit are
  unchanged.
