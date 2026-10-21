### Fixed

- `edit_file` now matches multi-line `old_text` in CRLF files and keeps CRLF; a `write_file`/`edit_file` card for a line-ending or final-newline change shows `line endings CRLF → LF` / `final newline removed` instead of `(no line changes)`. Mixed-ending files are matched on exact bytes.
