### Fixed

- "Write tests for the new service", "make sure the new app starts" and "fix the crash when I
  start the new app" are no longer treated as a Wayfinder project start (headless `--go`
  refused them, `/task` refused them and chat turned its tools off). "New app", "new service"
  and the other kinds now start Wayfinder only directly after a creation verb with at most two
  of `a`, `an`, `another`, `brand`, `my`, `our` between them ("build a brand new app", "set up
  a new project"), or at the start of the text. `the` no longer bridges the verb and "new".
