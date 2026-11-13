### Changed

- `--headless` now accepts `--mission NAME` and `--new-mission NAME` with `--go`,
  `--resume` and `--chat`, so a script whose default mission holds its 256 tasks can plan
  in a new one. Without them it still opens the default mission. A missing `--mission`
  name exits 2 naming `--new-mission`.
- The headless task-limit refusal names `--new-mission NAME` on the `--headless` command
  instead of the terminal's mission picker.
