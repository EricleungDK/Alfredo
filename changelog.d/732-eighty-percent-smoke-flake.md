### Fixed

- The 80%-full task list PTY smoke no longer fails intermittently on a loaded machine:
  it waits for the "Task #205 saved" footer before pressing F2, and presses F2 again only
  while the task view is still showing, instead of reading the chat view before the save
  had reached the screen. Test-only; the notice itself was always rendered.
