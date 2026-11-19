### Fixed

- `/stop` now also cancels the running chat turn like Esc, including a running `run_command` (its process group is killed, the row reads `cancelled after Ns`, the approval log says `cancelled`); it still pauses autopilot when one runs and still says `No active autopilot` when neither a chat turn nor an autopilot is running. The F1 entry for `/stop` names the chat turn.
