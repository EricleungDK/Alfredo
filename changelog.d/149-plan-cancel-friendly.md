### Fixed

- `/plan-cancel` no longer fails with `Invalid planner command bounds or source binding`
  when the plan draft is invalid (for example the model returned prose) or absent. An
  invalid draft is discarded, its pane closes and the status line reads `Plan draft
  discarded`; with nothing to cancel it reads `No plan draft to cancel`. Cancelling an
  in-flight generation still stops it as before; cancelling a valid draft still drops
  it and now also closes the empty draft pane.
