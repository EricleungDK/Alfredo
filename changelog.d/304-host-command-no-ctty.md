### Fixed

- A host-escalated `run_command` now runs in its own session with no controlling
  terminal (and `GIT_TERMINAL_PROMPT=0`), so `sudo`, `ssh` and git credential prompts can
  no longer draw over the TUI or read its keystrokes. A command that needed such a prompt
  fails as a normal command failure, with a note to use ssh-agent or a credential helper.
