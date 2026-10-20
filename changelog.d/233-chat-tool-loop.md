### Added

- Chat can read your repository itself. Plain chat turns offer the model read-only tools
  (`read_file`, `list_files`, `grep`) on the working tree, so "summarise README" works
  without `@README.md`. Paths stay inside the repository (absolute, `..` and escaping
  symlinks are refused; `.git`, secrets and generated trees are excluded) and each
  result is capped at 16 KiB. A turn runs at most 8 tool rounds, then stops with
  `Stopped: tool step limit reached`. The transcript shows each call (`⚙ read_file
  README.md`) and a one-line cut of its result.

### Changed

- Tool calls and results stay part of their turn: they are sent as context, count toward
  the conversation limit and `--num-ctx`, and are summarised by compaction. Saved
  conversations are now schema v19; the first save by this build keeps the previous file
  as `<conversation>.v18-backup`, and older builds refuse a v19 file.
- The chat system prompt now describes the tools instead of saying chat cannot browse
  the repository.
