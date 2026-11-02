### Fixed

- A sandboxed `run_command` that hides secret files (`.env`, `.npmrc` and so on) now says
  so: its result carries one line with the count of hidden files and directories, which
  fail with `Permission denied`, and tells the model not to retry on the host to read
  them. No names or contents are shown. The refusal for more than 128 secret files now
  says to remove or rename them, or move them out of the Coding Workspace, and still says
  nothing ran. `docs/reference.md` no longer claims a masked file reads as empty.
