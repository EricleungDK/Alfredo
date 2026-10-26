### Fixed

- Host `run_command` now appends the terminal hint when git fails with "terminal prompts
  disabled" or "could not read Username/Password", and the host command and git write
  tools share one terminal-need check.
