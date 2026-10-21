# Automation runbook (scheduled scan and build agents)

Read this before any scheduled or catch-up run. It records what earlier runs had to
rediscover, so the next run doesn't. Every entry came from a real run (see the run log,
issue #158). Add to it when you hit something new (see "Recording a new pitfall").

## 1. Set up the sandbox first (one command)

```bash
source alfredo-tui/scripts/agent-env.sh
```

It installs what CI has and the cloud sandbox lacks, and prints a one-line summary for
your run log:

| Missing in sandbox | Symptom if you skip it | Handled by |
|---|---|---|
| `bubblewrap` | ~90 Rust tests and every `/go` smoke fail with `Coding workers need bubblewrap` / `trusted Bubblewrap executable` | apt install |
| `cargo-audit` 0.22.2 | `audit_smoke.py` and the audit gate can't run | cargo install |
| `/usr/bin/time` | M16 (peak RSS) needs a fallback | apt install |
| small disk (~2–17 GB free) | builds fail or tests flake with "No space left" | `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_INCREMENTAL=0`, one shared `CARGO_TARGET_DIR` |
| Rust 1.97 vs CI's pinned 1.96.0 | only `release_smoke.py` refuses | `AGENT_ENV_TOOLCHAIN=1` (opt-in; costs disk) |

The sandbox has 2 CPUs. Don't build more than two branches at once, and share one target
dir across worktrees.

## 2. Run the gates with one command

```bash
alfredo-tui/scripts/agent-gates.sh --retry-flaky            # everything CI runs, except release packaging
alfredo-tui/scripts/agent-gates.sh --quick --retry-flaky    # fmt, clippy, tests, Python, audit
```

It reads the smoke list from `.github/workflows/rust-terminal.yml`, so it never drifts
from CI, and prints a Markdown table you can paste into a PR or issue comment. `FLAKY`
means a test failed once and passed on an exact re-run: report it, don't hide it.

## 3. GitHub access in scheduled runs

- Use the **GitHub connector tools** for issues, labels, comments and PRs. The `gh` CLI
  reports `The token in GH_TOKEN is invalid` in scheduled runs, and Bash `gh api` writes
  can be refused by the auto-mode classifier as "External System Writes".
- `git push origin claude/...` works. **Branch deletion is blocked** (HTTP 403); never
  create throwaway branches.
- `add_repo` may not exist; `git clone https://github.com/EricleungDK/alfredo` works.
- **Re-running a CI job returns 403** (`Resource not accessible by integration`). To
  retrigger CI after a suspected flake, push an empty commit once:
  `git commit --allow-empty -m "ci: retrigger (suspected flake: <test>)"` and say so in
  the PR. If it fails again, it's not a flake: investigate.
- A scheduled run can't start another scheduled task (needs approval nobody can give).
  The scan and build coordinate through the run log (#158) instead.

## 4. Root vs non-root

The sandbox runs as **root**; CI runs as a normal user. Anything involving file
permissions, cleanup of read-only files, or ownership can pass locally and fail in CI
(this happened in PR #179). For such changes, also run the relevant test as `nobody`:

```bash
setpriv --reuid=nobody --regid=nogroup --clear-groups --reset-env -- python3 alfredo-tui/tests/<test>.py
```

## 5. Merge-conflict hotspots

Parallel PRs from the same build run collide in the same few places:

| Hotspot | Rule |
|---|---|
| `CHANGELOG.md` | Never edit it. Add `changelog.d/<issue>-<slug>.md` (see `changelog.d/README.md`). |
| `--help` text in `src/main.rs` | It is `HELP_LINES`, one entry per output line. Add your flag as its own entry or edit only your flag's line; never reflow neighbouring lines. |
| `README.md` flag table, `docs/reference.md` | Add rows/paragraphs next to related ones; don't reflow existing paragraphs. |
| Tests appended at the end of the same file (`tests/autopilot.rs`, `tests/doctor.rs`) | Put new tests next to the tests for the same feature, not at the end of the file. |
| Shared functions (`App::select_model`, `health.rs`) | Mention the overlap in the PR body under "Merge order" so the human knows. |

The build's daily "keep open PRs mergeable" step resolves the rest.

## 6. Known flaky tests

| Test | Cause | Status |
|---|---|---|
| `tests/understanding.rs::concurrent_entry_creates_one_flow_and_work_through_survives_reload` | 8 writers vs a 250 ms lock deadline on slow runners (`LockError::Busy`); the deadline is now 3 s with backoff | fixed (issue #210) |
| `agent_terminal_smoke.py` F2 press swallowed by a dashboard redraw | timing race | fixed by #200 |
| `layout::capacity_wait_shows_a_static_cat…` | wall-clock label | fixed by #177 |
| Any `ollama_fixture` test: `tests/ollama_fixture/mod.rs:55` `AddrInUse` panic | `reserve()` closed its port and `serve()` rebound it, so a parallel test could take it in between; `serve` now retries then falls back to a fresh port (`serve_at` waits for the exact port) | fixed (issue #300) |

## 7. Measuring metrics (scan)

- Timings (M8–M11, M16) are noisy when sub-agents build in parallel on 2 CPUs. Measure
  them first, before starting parallel builds, with the release binary
  (`--binary alfredo-tui/target/release/alfredo-tui`); the eval warns on debug builds.
- For the Claude Code reference, never wrap `claude` in a logging shell script: the eval
  then measures the wrapper (M16 was invalid on 2026-10-19).
- `duration_ms − duration_api_ms` is negative for Claude Code, so M8 must use the eval's
  Enter→request / reply→screen probes, not that formula.

## 8. Reporting tokens

Sub-agents resumed with SendMessage for fix rounds don't report token totals. Record the
total from each agent's final result, and mark the build total "lower bound" when any
agent was resumed.

## 9. Recording a new pitfall

If a run loses time to something not covered here, add a line starting `Pitfall:` to its
run-log comment on #158: what happened, the exact error, what worked. The weekly scan (on
reference days) reviews `Pitfall:` lines; any seen in two or more runs becomes a
`technical` issue labelled `automation` to update this runbook or the scripts.
