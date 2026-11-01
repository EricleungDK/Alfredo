### Added

- Chat approval cards offer `a` (always allow): it approves the call and adds a rule so
  later calls of that tool, or sandboxed `run_command` commands starting with the same
  words (any arguments after them), run without asking in this mission. Rules survive
  restarts; `/allow` lists them and `/allow clear [ID]` removes them. `git_push`, and
  `git … push` through `run_command`, are never always-allowed; HOST (escalated) runs
  never are (issue #311); commands with shell operators need an exact rule.
  Auto-approved calls are still checked against their preview and logged with the rule
  id in F4.
