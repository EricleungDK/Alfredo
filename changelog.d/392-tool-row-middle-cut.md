### Fixed

- Chat `⚙` tool rows for `read_file`, `write_file`, `edit_file` and `list_files` now cut an
  over-long path in the middle (`src/very/…/main.rs`) instead of at the end, so the file
  name stays visible at the actual terminal width and sibling files no longer look identical. Paths are measured by
  display width, so CJK and accented paths are never split mid-character. Command, grep
  and git rows still cut at the end.
