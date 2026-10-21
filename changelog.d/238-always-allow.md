### Added

- Chat approval cards offer `a` (always allow): it approves the call and adds a rule so
  later calls of that tool, or `run_command` commands starting with the same words, run
  without asking in this mission. Rules survive restarts; `/allow` lists them and
  `/allow clear [ID]` removes them. `git_push` (and `git … push` via `run_command`) is never
  always-allowed, a host
  (escalated) run needs an exact host rule, and commands with shell operators need an exact
  rule. Auto-approved calls are still checked against their preview and logged with the
  rule id in F4.
