### Fixed

- The autopilot PTY smoke no longer fails intermittently in the interrupted-run refusal
  test: it waits for the filter notice that replaces the footer, and for the full refusal
  text on one screen read, before sending the next command.
