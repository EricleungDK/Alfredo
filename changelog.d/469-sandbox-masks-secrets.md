### Security

- A sandboxed `run_command`, including one an always-allow rule covers, no longer hands the
  model a secret file in the Coding Workspace: a rule approving `cat README.md` no longer
  also runs `cat README.md .env`. Every workspace file the secret-file list matches is masked
  inside the sandbox (it reads as empty and cannot be written), and every matching directory
  such as `.ssh/` appears empty (#469).
- A sandboxed run that cannot mask every secret file is refused with a reason and runs
  nothing: more than 128 masks, a symbolic link named like a secret file, or a workspace the
  scan cannot finish. Committed history is not masked yet: `git show <rev>:.env` inside the
  sandbox can still print a secret that was committed and later removed. An `escalate: true`
  run is on the host and is not masked (#469).
