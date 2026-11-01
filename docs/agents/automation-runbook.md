# Automation runbook (scheduled scan and build agents)

Read this before any scheduled or catch-up run. It records what earlier runs had to
rediscover, so the next run doesn't. Every entry came from a real run (see the run log,
issue #158). Add to it when you hit something new (see "Recording a new pitfall").

## 1. Set up the sandbox first (one command)

```bash
source alfredo-tui/scripts/agent-env.sh
```

On a fresh sandbox this takes 6–15 min (the cargo-audit compile, and apt hanging on
`download.docker.com connect_rejected`), which is more than the 10-minute Bash timeout.
Run it in the background with a log (`source … >"$TMPDIR/env.log" 2>&1 &`) or with an
explicit `timeout` of 600000 ms and re-run it, and poll the log instead of waiting.

It installs what CI has and the cloud sandbox lacks, and prints a one-line summary for
your run log:

| Missing in sandbox | Symptom if you skip it | Handled by |
|---|---|---|
| `bubblewrap` | ~90 Rust tests and every `/go` smoke fail with `Coding workers need bubblewrap` / `trusted Bubblewrap executable` | apt install |
| `cargo-audit` 0.22.2 | `audit_smoke.py` and the audit gate can't run | cargo install |
| `/usr/bin/time` | M16 (peak RSS) needs a fallback | apt install |
| small disk (~2–17 GB free) | builds fail or tests flake with "No space left" | `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_INCREMENTAL=0`, a `CARGO_TARGET_DIR` of your own (see below) |
| Rust 1.97 vs CI's pinned 1.96.0 | only `release_smoke.py` refuses | `AGENT_ENV_TOOLCHAIN=1` (opt-in; costs disk) |

The sandbox has 2 CPUs. Don't build more than two branches at once. Use **one
`CARGO_TARGET_DIR` per agent or worktree**, never one shared dir: a shared dir gave smokes
a stale binary, clippy "Finished in 0.18s" on a merged tree, a red-commit binary that ran
48 tests instead of 50, and 2 spurious `chat_approval` failures. If you must compare
against an older build, `touch` the sources first so cargo rebuilds.

## 2. Run the gates with one command

```bash
alfredo-tui/scripts/agent-gates.sh --retry-flaky            # everything CI runs, except release packaging
alfredo-tui/scripts/agent-gates.sh --quick --retry-flaky    # fmt, clippy, tests, Python, audit
```

The full run also exceeds the 10-minute foreground limit: start it in the background
with a log (`… >"$TMPDIR/gates.log" 2>&1 &`) and poll the log.

It reads the smoke list from `.github/workflows/rust-terminal.yml`, so it never drifts
from CI, and prints a Markdown table you can paste into a PR or issue comment. `FLAKY`
means a test failed once and passed on an exact re-run: report it, don't hide it. When
`cargo test` fails, the Note column names the failing test binary (`failing: --test
<name>`): that is a real test failure, not a script glitch.

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
- **`git reset --hard` in the main clone is denied** by the auto-mode classifier as
  `[Irreversible Local Destruction]`. Use `git fetch` then `git checkout --detach
  origin/main`, or add worktrees from `origin/main`.
- **`merge_pull_request` needs the full 40-character `expectedHeadSha`.** A short SHA
  gives a 409 or is rejected.
- **A security verdict doesn't carry over when the sync touches
  `alfredo-tui/docs/reference.md`.** Nearly every PR edits it, so re-check the verdict
  (about 1 min) after syncing.
- **Sub-agent Bash can fail with `server-side auto mode classifier gave no verdict
  (error)`.** Retry with a trivial command, then continue.
- **Give each agent its own subdirectory.** `cp -r` of a worktree shares its `.git`
  metadata, and one agent's `/tmp/x` cleanup wiped another's worktree. Use
  `git worktree add` or `git archive` for scratch copies.
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
| `autopilot::saved_state_carries_done_and_total_for_read_only_peeks_of_other_missions` panics `Task store is busy; refresh and retry` (`tests/autopilot.rs:73`) | store-lock deadline exceeded on slow CI runners, same family as #210 | fixed (issue #442, PR #445) |
| `chat_tools_terminal_smoke.py` Wayfinder test reads the footer mid-draw (`'/scope-clear' not found`) | `wait_for('tools off')` returns on a partial chunk, then `assertIn(rest, screen())` races the redraw | fixed (issue #443, PR #444) |
| `chat_git_tools` worktree tests panic `Os { code: 2, kind: NotFound }` | git maintenance pruned a half-built worktree entry mid-test | fixed (issue #448, PR #450) |
| `chat_secret_files::large_index_inexact_secret_copy_is_refused_by_git_commit` hits the link-check time limit | a detached `git gc`/maintenance pruned `objects/xx` during the link walk | fixed (issue #449, PR #451) |
| `chat_git_tools::an_alternates_fifo_refuses_promptly_and_never_hangs` times out (`git_status did not return within 10 s`) | `git rev-parse` ran before the FIFO checks and opened the alternates FIFO | fixed (issue #458, PR #459) |
| Any `ollama_fixture` test: `tests/ollama_fixture/mod.rs:55` `AddrInUse` panic | `reserve()` closed its port and `serve()` rebound it, so a parallel test could take it in between; `serve` now retries then falls back to a fresh port (`serve_at` waits for the exact port) | fixed (issue #300) |

Two rules from these rows:

- **PTY tests:** wait for the full string you then assert on (not `wait_for(part)` then
  `assertIn(rest, screen())`), and pause between Esc and the next key.
- **Git fixtures:** create repos with `-c gc.auto=0 -c maintenance.auto=false`, or a
  detached gc or maintenance run can prune `.git` mid-test.

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
