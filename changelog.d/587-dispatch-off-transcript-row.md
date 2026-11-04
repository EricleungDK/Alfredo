### Fixed

- When dispatch turns itself off because the task store stayed busy or a refresh failed,
  the chat transcript now gets one row with the reason and `/dispatch on` to resume. It
  used to appear only on the F2 task-view status line, leaving `✓ Dispatch on` as the
  last word in chat. The row is display only and is not kept across a restart.
