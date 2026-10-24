### Fixed

- The `a` always-allow hint on a `run_command` card now shows the whole command exactly as
  the card's `$` lines draw it (tab as `→` plus padding, newline as `↵`, a literal `→`/`↵`
  as `<U+2192>`/`<U+21B5>`) instead of stripping control characters and cutting at 80
  characters. A command that shows as more than 80 characters, holds another control
  character (even only around the ends) or contains `<U+` text is not offered `a`, and the
  key line says why. `/allow` renders the same way, and the card's `$` lines now mark tabs
  with `→` too.
