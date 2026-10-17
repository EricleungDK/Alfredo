### Fixed

- `--mission NAME` for a missing mission now says `Mission 'NAME' does not exist; use
  --new-mission NAME to create it (existing: default, m1)` (first 5 names, then `…`), and
  `--new-mission NAME` for an existing one says `Mission 'NAME' already exists; use
  --mission NAME to resume it`, instead of the selector's "choose Start New Mission" /
  "choose Resume" wording. The in-TUI selector wording is unchanged.
