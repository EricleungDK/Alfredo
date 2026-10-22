### Changed

- Chat `⚙` tool-call rows now lead with the path or primary argument and cut long text
  (`⚙ edit_file README.md`, `⚙ write_file src/x.rs (42 lines)`, `⚙ git_commit "msg" (1 file)`,
  `⚙ grep "TODO" in src/`) instead of listing argument values in alphabetical key order.
  Unknown tools keep the old format (#274).
