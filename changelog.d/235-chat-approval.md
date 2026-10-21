### Added

- Chat can edit files with your approval. `edit_file` (replace one exact, unique match)
  and `write_file` (create or replace a file) pause the turn and show a card with the
  tool, its arguments and a diff; `y` approves, `n` denies (the model is told nothing
  changed), Esc denies and ends the turn. Keys pressed in the first 0.5 s are ignored (and later `y`/`n` only answer an empty prompt, issue #264), and
  a file that changed after the preview is not written. "redraft README" now edits it in place.
  Missing or ambiguous matches, paths outside the repository, `.git` and secrets are
  refused before asking.
- Each outcome (applied, failed, denied, cancelled) is logged to `chat-approvals.jsonl`
  in the State Directory and shown in F4 activity. The task store is unchanged.

### Changed

- The chat system prompt no longer says chat cannot edit files.
