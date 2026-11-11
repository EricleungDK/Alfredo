### Added

- `read_file` takes an optional inclusive `end_line`, so a model can ask for a short slice instead of a whole window; an `end_line` before `start_line` or a non-integer one is refused, and one past the end of the file reads to the end. Arguments it does not know (`offset`, `limit`) now get the same `[note: unknown arguments … ignored; accepted: …]` line as `list_files` and `grep`.
