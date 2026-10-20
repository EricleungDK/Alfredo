### Fixed

- A repaired task's commit on the `alfredo/go-*` branch now uses the original task's
  title as its subject (following repair-of-repair chains) instead of the truncated
  `Repair #N: autopilot: Check failed … Traceback` text; the body still says
  `Repair of #N`. Task titles in the UI are unchanged.
