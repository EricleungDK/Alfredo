### Fixed

- An autopilot state file that cannot be read at all (a directory, permission denied)
  now names the file and the way out (fix its permissions, move it aside, or start with
  `--new-mission NAME`) instead of a bare OS error (#790).
