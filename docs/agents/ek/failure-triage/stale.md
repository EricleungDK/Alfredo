# Stale checks

Run the ones that fit the failure; each yields evidence for the triage line.

- **Test expects old behaviour**: `git log -p -S'<expected text>' -- <test file>` and the same on the code. If the code changed on purpose (spec, issue, commit message) and the test lags, update the test and say why. If the intent is unclear, treat it as a real bug.
- **Port held**: `ss -ltnp 'sport = :<port>'`. Kill only the exact PID you started this session; otherwise use another port.
- **Local main behind**: `git fetch && git status -sb` / `git log --oneline HEAD..origin/main`. Rebase or merge before judging.
- **CI SHA ≠ HEAD**: `gh run view <id> --json headSha` against `git rev-parse HEAD`. Judge only the run for your SHA.
- **Cached build / deps**: clear the specific cache (`.pytest_cache`, `target/`, `node_modules/.vite`, lockfile drift), then rerun once.
- **Task docs out of date** (`.agent/Tasks`, context notes): trust the code and git log over the doc; note the drift.
