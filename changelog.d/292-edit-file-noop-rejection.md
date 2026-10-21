### Fixed

- `edit_file` now rejects edits that change nothing (identical `old_text` and `new_text`,
  or line-ending conversions in CRLF files) before asking for approval instead of
  reporting success.
