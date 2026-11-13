### Fixed

- At the 256-task limit, `/go`, `/plan`, `/plan-revise` and `/architect-revise` now show
  the whole refusal (the limit, slots needed and free, and `--new-mission NAME` or Start
  New Mission) as a chat row, with a short footer pointing at it, instead of a footer
  cut before the remedy at 80 and 120 columns.
- The `Task list 80% full (205/256)` warning is now a chat row, shown once per terminal,
  instead of a footer notice that a restored-session or autopilot notice hid.
