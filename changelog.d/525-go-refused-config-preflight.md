### Fixed

- `/go`, `--go` and `--doctor` now refuse a repository whose Git config Alfredo will not run
  (filters, includes, partial clones, ssh/proxy commands, exclude/attributes files) before
  any planner request, naming the key as written and the fix
  (`git config --local --unset KEY`). `--go --headless` exits 2; `--doctor` no longer adds
  "Coding needs a Git repository with a commit" to this refusal.
