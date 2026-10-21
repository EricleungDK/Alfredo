### Added

- `alfredo-tui --chat PROMPT --headless` runs one Chat Agent turn without a terminal and
  prints one JSON result. `--approve workspace-edits` approves `edit_file`/`write_file`
  inside the repository and denies everything else; without it every change is denied.
  Outcomes are logged to `chat-approvals.jsonl`. Interactive chat still always asks.
- Eval task `T7-chat-tool-edit`: a chat turn finds, reads and fixes a function through the
  chat tools with a scripted approval, graded by `check.sh`. Trials record approvals and
  the tool-call flow.
