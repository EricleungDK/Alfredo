### Security

- A sandboxed `run_command` in a Coding Workspace with no root `.git` can no longer create
  a repository there with `git init`, `mkdir .git` or a hand-built `.git`. The sandbox
  holds a read-only placeholder file at `.git` for the run and removes it afterwards, so
  the chat git tools never see a repository the model wrote.
