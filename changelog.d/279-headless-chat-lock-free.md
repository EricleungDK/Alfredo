### Fixed
- `--chat … --headless` no longer takes the conversation owner lock or the selection journal: it runs beside an open TUI or another headless chat on the same repository instead of failing with "Another alfredo-tui already owns conversation", and it writes no conversation file. `--go --headless` locking is unchanged.
