### Added

- Long conversations no longer stop at the 128 KiB limit. Near it (96 KiB, or less with
  `--num-ctx`), Alfredo asks the conversation's model to summarise the oldest turns into
  a short note that replaces them, keeps the newest turns verbatim and sends your prompt.
  The transcript shows `N earlier turns compacted into a summary` with the note, and the
  saved conversation stores the note instead of the compacted turns. If the summary fails
  the prompt stays in the draft and the footer names Ctrl+N and `/clear`.
- `/clear` restarts the current conversation (tasks are kept) and `/compact` summarises
  its older turns on demand. Both appear in F1 and Tab completion.

### Changed

- The remaining hard-cap message now reads `Conversation limit reached; press Ctrl+N for
  a new conversation or type /clear to restart this one`.
