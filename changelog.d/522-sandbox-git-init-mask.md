### Security

- A sandboxed `run_command` in a Coding Workspace with no root `.git` can no longer create
  a repository there with `git init`, `mkdir .git` or a hand-built `.git`. The sandbox
  holds a read-only placeholder file at `.git` for the run and removes it when the run
  ends. If the process is killed mid-run, the file stays until the next sandboxed run in
  that workspace replaces it, and the chat git tools refuse it meanwhile.
