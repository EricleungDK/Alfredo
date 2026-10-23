### Fixed

- The eval grader no longer turns a `check.sh` that prints invalid UTF-8 into a
  `harness_error`; undecodable bytes are replaced so the trial grades on its exit code.
