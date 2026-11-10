### Changed

- A failed dispatched or `/run` task's chat row now says why, from the receipt's own
  detail: `✗ Task #1 failed · Check failed (exit 3): stderr: AssertionError: v should be 1 b…`.
  The cause is one line, hidden characters show as `<U+XXXX>`, and it is cut with `…`
  at 80 cells. The collapsed autopilot step reads `failed · cause` the same way.
