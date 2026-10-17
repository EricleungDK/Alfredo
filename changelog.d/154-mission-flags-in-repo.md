### Fixed

- `--mission NAME` / `--new-mission NAME` without `--workspace` no longer silently open the
  selector and drop the flag. Inside a Git repository they use the repository root and
  resume or create NAME exactly as with `--workspace` (including `Mission does not exist`
  and `Mission already exists`); outside one they exit with a message naming
  `--workspace DIR`.
