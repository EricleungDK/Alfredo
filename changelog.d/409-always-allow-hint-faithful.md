### Fixed

- The `a` always-allow hint on a `run_command` card now shows the whole command as the
  card's `$` lines do (tab as `→`, newline as `↵`) instead of stripping control characters
  and cutting at 80 characters; a command too long to show, or holding other control
  characters, is not offered `a` and the key line says why. `/allow` renders the same way.
