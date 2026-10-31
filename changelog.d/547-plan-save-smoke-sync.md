### Fixed

- The terminal smoke `test_run_refusals_reach_the_footer_while_the_task_panel_is_shown`
  now waits for the planner job to finish (`Esc cancel plan` gone) before sending
  `/plan-save`, so it no longer fails when the command is refused while the draft is
  still being finished. Test-only; no product change.
