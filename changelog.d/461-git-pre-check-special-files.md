### Security

- Chat git tools now refuse, before git runs, a FIFO, socket, device or link in any file
  git may open (`index`, `packed-refs`, `shallow`, `config.worktree`, `info/`, `refs/`,
  object stores and packs), a workspace with no `.git` or `HEAD` (git would read the root
  as a bare repository), and gitfile or alternates lines that git parses differently from
  the earlier check (`gitdir:  x`, quoted or padded lines). Alternates are checked per line
  against the call's time limit and each store is queued once. An oversize gitfile or
  alternates file is now reported as too large. The check is a best-effort fast path: git's
  15 s limit remains the backstop, and the alternates start-up behaviour of git 2.56 was
  inferred from CI (#461).
