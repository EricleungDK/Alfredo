### Added

- Chat can run shell commands with your approval. `run_command` runs in the bubblewrap
  Sandbox by default (network off, only the working tree writable); the card shows the
  exact command, cwd and "sandboxed". "run the tests" works from chat.
- `escalate: true` runs a command on the host with network on (e.g. "push this branch")
  behind a red `HOST · network on` card. Denied escalations run nothing. Escalated
  outcomes are marked in `chat-approvals.jsonl` and F4 (`(HOST)`).
- Timeouts kill the whole process tree; output is capped with a marker; exit code and
  output go back to the model. A sandbox that cannot start suggests escalation instead
  of running on the host.

### Changed

- The chat system prompt no longer says chat cannot run commands.
