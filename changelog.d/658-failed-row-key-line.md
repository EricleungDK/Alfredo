### Fixed

- A failed task's chat row, the `/go` step row and the footer now show the check's key
  error line instead of the head of its output, so a Python failure reads
  `✗ Task #1 failed · Check failed (exit 1): AssertionError` rather than
  `… stderr: Traceback (most recent`. The `stderr:` label is dropped, the head is cut
  first when the row is too long, and hidden characters still show as `<U+XXXX>`.
