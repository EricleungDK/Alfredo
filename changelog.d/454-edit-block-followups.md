### Fixed

- Worker EDIT blocks: the SEARCH match count stops at the second match, so a degenerate
  file of newlines no longer takes seconds (4.6 s for 64 KiB in release); the ambiguity
  error now says "matches more than once".
- A SEARCH can match the last line of a file with no trailing newline (the prompt already
  showed one); the file still ends without a newline after the edit.
- After a truncated EDIT block the repair note asks for fewer or shorter SEARCH/REPLACE
  pairs instead of whole files.
- The live display recognises `=== EDIT:` and `=== END EDIT ===` lines only outside FILE
  blocks, so file content cannot spoof a heading or hide lines.
