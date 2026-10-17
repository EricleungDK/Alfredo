### Fixed

- Typing `/task` or `/after` with no arguments (or `/after 1,2` with no description)
  now prints `Usage: /task DESCRIPTION` / `Usage: /after IDS DESCRIPTION` instead of
  `Unknown command`. Bare `/assign`, `/permit`, `/repair`, `/requeue`,
  `/resolve-repair`, `/review`, `/scope-confirm`, `/architect-revise` and
  `/retry-command` get the same usage line. A test now submits every command in the
  F1 registry bare and fails if any is reported unknown.
