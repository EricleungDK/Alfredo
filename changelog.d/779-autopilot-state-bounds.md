### Fixed

- A garbled autopilot state file (a task count above 256, a first task id of 0, a first
  id and count that overflow, or more than 256 adopted tasks) is now refused with
  "Unsupported or invalid autopilot state" and left untouched, instead of aborting
  Alfredo on every launch.
