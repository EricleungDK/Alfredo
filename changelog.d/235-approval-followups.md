### Fixed

- A chat approval card now ignores `y`/`n` until it has been on screen for 0.5 s, and
  restarts that delay when you switch back to its conversation; before, a card in a
  background conversation could be approved by keys typed elsewhere.
- An approval cancelled with Esc before the edit ran is now logged in F4 activity.
- Concurrent Alfredo processes no longer lose chat approval log records.
- Chat edits refuse symlinked targets and re-check the resolved path at approval.
