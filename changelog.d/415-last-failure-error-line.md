### Fixed
- `last failure:` for a failed check now ends with the check's key error line (for example
  `AssertionError` or `FAILED tests/x.py::t - assert 1 == 2`) instead of stopping at the
  traceback header, in headless JSON, the stderr line and the TUI summary.
