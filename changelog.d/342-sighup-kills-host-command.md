### Fixed

- Closing the terminal (SIGHUP) or sending SIGTERM to the interactive TUI during an
  approved host `run_command` now quits like Ctrl+Q: the command's whole process group is
  killed, `cancelled` is logged in `chat-approvals.jsonl`, the conversation is saved and the
  terminal restored. Before, the command ran on unattended with no log entry (#342).
