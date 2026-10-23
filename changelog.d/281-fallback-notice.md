### Fixed

- The "model has no tool support" notice is no longer stored as assistant text, so it is
  not replayed to the model on later turns or glued to the front of the headless `reply`.
  It now shows in the footer (TUI) and on stderr (headless), and headless JSON adds
  `"tools": false` when the fallback ran.
