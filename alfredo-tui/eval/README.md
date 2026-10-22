# Harness eval

One command that runs a fixed task set through Alfredo (or Claude Code on the same repos and
the same graders) and prints schema-valid JSON with the harness metrics `M1`-`M16` from
scoreboard #159. Standard library Python only.

```bash
cargo build --manifest-path alfredo-tui/Cargo.toml        # the binary under test
python3 alfredo-tui/eval/run_eval.py --agent alfredo --backend fixture --trials 3 --out out.json
```

Fixture mode is deterministic, needs no model, and takes about 40 s on 2 CPUs (7 tasks x 3
trials). CI runs a one-trial pass of it (`tests/eval_smoke.py`).

## How a trial works

1. Copy `tasks/<id>/repo/` into a fresh temp directory, `git init`, commit it on `main`.
2. Run the agent on `goal.md`:
   - **Alfredo**: start the TUI in a pty with a fresh `--state-dir`, send `/go <goal>`, wait for
     `git switch alfredo/go-` (done) or `Autopilot ✗ failed`, then quit with Ctrl+Q.
   - **Alfredo, chat tasks** (`task.json` `"mode": "chat"`, issue #240): run one Chat Agent
     turn headless, `alfredo-tui --chat <goal> --headless --approve workspace-edits`
     (`chat_trial`). Read tools run; the scripted policy approves `edit_file`/`write_file`
     inside the task workspace and denies anything else, and every outcome is logged to the
     same `chat-approvals.jsonl` as an interactive approval. The candidate is a commit of the
     working tree the approved edits left. A chat trial passes only if `check.sh` passes **and**
     `chat.read_before_edit` holds (a successful `read_file` of the edited file, or `grep` over
     it, came before the first applied edit); a blind write that happens to pass is a fail
     with the reason in `grade_output` and `notes`.
   - **Claude Code**: run it with the goal on stdin in the temp repo (see below).
3. Grade the candidate without reading any screen or log: `git archive` the `alfredo/go-*`
   branch (Claude Code: a commit of the final working tree), extract it, and run the task's
   `check.sh` in the grading sandbox (below). Exit 0 is a pass. No branch means fail.
4. Delete the temp directory (use `--keep` to inspect it).

`check.sh` lives outside the repo the agent sees, so an agent cannot read or edit its grader.
`python3 alfredo-tui/eval/run_eval.py --agent alfredo --validate-tasks` checks that every
`check.sh` fails on its untouched repo.

### Grading sandbox

The candidate is model-written code, and `check.sh` imports it (`from mathx import ...`), so
grading runs it under bubblewrap with the same profile the worker uses
(`alfredo-tui/src/worker.rs`, `check_request`):

- `bwrap --unshare-all --unshare-net --die-with-parent --new-session` with a tmpfs `/` and a fresh
  `/dev`, `/proc` and `/tmp`. The host home directory, your files and the network are not visible.
- System directories (`/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, `/etc`) are read-only.
- Only the extracted export is bound read-write, and `HOME` points at it. `check.sh` is bound
  read-only at `/grader/check.sh`.
- The check runs under `prlimit` with the worker's limits: address space 8 GiB, file size 32 MiB,
  1024 open files, 128 processes. The timeout is 60 s.

Archive members are checked before extraction. An absolute or `..` path, a symlink whose target
leaves the export, a hardlink outside the archive, or a device or fifo makes the grade fail with
`refused: ...` and nothing is extracted. After extraction, any symlink that still resolves outside
the export also fails the grade.

Requirements: `/usr/bin/bwrap` (package `bubblewrap`) and `/usr/bin/prlimit` (`util-linux`). CI
installs both. Without them grading refuses to run and reports
`grading sandbox unavailable`, so Alfredo and Claude Code runs stop before the first trial.
`--unsafe-unsandboxed-grading` is the only way to grade on the host without the sandbox. It
runs the candidate as you, with your files and network, so use it only on a disposable machine.
When the sandbox is present the flag changes nothing. `tests/eval_smoke.py` passes no such flag:
it needs bwrap, and in CI it grades inside the sandbox. `tests/test_eval_grader.py` runs the
#207 attack (a candidate that writes a marker outside the export) and its live bwrap tests are
skipped without `/usr/bin/bwrap`.

The sandbox covers the grader only. It does not sandbox the agent run itself (for Claude Code
see the residual risk below).

## Tasks

`tasks/<id>/` holds `repo/` (fixed starting files), `goal.md`, `check.sh`, `answers.json`
(the scripted fixture replies, used only by `--backend fixture`) and `task.json` (title,
`kind`).

| Task | What it tests |
| --- | --- |
| `T1-add-function` | add a function to a module |
| `T2-auto-repair` | a deliberately wrong first answer, then the automatic repair |
| `T3-dependent-tasks` | two planned tasks, the second depends on the first |
| `T4-prose-control` | **negative control**: the worker only replies in prose; must grade as failed |
| `T5-test-and-fix` | fix a bug and add a regression test file |
| `T6-multifile-rename` | rename a function across four files |
| `T7-chat-tool-edit` | Chat Agent tools: `grep` -> `read_file` -> approved `edit_file` fixes a function (chat mode) |

The negative control is scripted, so it runs only with `--backend fixture`; other modes list
it under `config.skipped_tasks`. `--tasks T1,T4` selects tasks by id or prefix.

`answers.json` has `plan` (what the fixture planner returns) and `workers`: a list of
`{"prefix": ..., "replies": [...]}` matched in order against the start of the worker prompt
(`Implement this task: ...`, repairs start `Implement this task: Repair #`). Replies are
consumed in order and the last repeats; a reply is `{"files": [...]}` or `{"prose": "..."}`.

A chat task's `answers.json` has `chat` instead: one entry per tool round,
`{"tool_calls": [{"name": ..., "arguments": {...}}]}`, ending with `{"content": "..."}`. The
fixture recognises a chat request by its `tools` and answers round n (the number of
assistant tool-call messages already in the request) with entry n.

## Options

| Option | Meaning |
| --- | --- |
| `--agent alfredo\|claude-code` | required |
| `--backend fixture\|ollama` | Alfredo only; default `fixture` |
| `--model NAME` | Ollama model (required with `--backend ollama`); Claude Code alias (default `haiku`) |
| `--trials N` | trials per task, default 3, each with a fresh repo and `--state-dir` |
| `--tasks IDS` | comma-separated ids or prefixes |
| `--out FILE` | write JSON there (default stdout); progress goes to stderr |
| `--timeout S` | per trial: fixture 120, ollama 900, claude-code 600 |
| `--binary PATH` | `alfredo-tui` binary (default `$ALFREDO_TUI_BINARY`, else `target/debug`). Use `target/release/alfredo-tui` for M11/M16 that compare with the scoreboard baselines: a debug build adds a `debug_profile` warning and a note on both metrics |
| `--probes` | Alfredo only, opt-in: also measure M8, M9, M10 and M14 (see Probes). Records `config.probes` (counts, binary, profile) |
| `--no-render-probe` | with `--probes`: skip the M10 `cargo test --release` (M10 stays `null`, reason says why) |
| `--code-health` | opt-in, either agent: also compute M15 (clippy pedantic, non-test unwrap/expect, largest non-test module, test counts; see Code health). Runs `cargo clippy` and `cargo test -- --list`: minutes on a cold build. Without it M15 is `null` |
| `--probe-chats N`, `--probe-keys N`, `--probe-recovery-trials N` | probe sample sizes; defaults 20 chats, 100 keys, 3 kill/resume trials (smaller values are for smoke tests) |
| `--ollama-endpoint URL` | default `http://127.0.0.1:11434`; must be a loopback address |
| `--tmp-root DIR`, `--keep` | where temp repos go; keep them |
| `--unsafe-unsandboxed-grading` | grade on the host when bubblewrap is missing (see Grading sandbox) |

Exit code: 0 ok, 1 if the run is flagged (below) or a fixture task did not grade as scripted,
2 for usage or environment errors (missing binary, Ollama unreachable, `claude` not found).

### Ollama mode (Eric, local only)

```bash
python3 alfredo-tui/eval/run_eval.py --agent alfredo --backend ollama --model qwen2.5-coder:14b \
  --trials 3 --out ollama.json
```

Alfredo talks to a recording proxy on loopback that forwards to your local Ollama, so token
counts, time to first token and call counts are measured, not guessed. The planner is the real
model, so the plan and the check commands are whatever it writes; the verdict is still the
independent `check.sh`. It fails fast with a message if Ollama is unreachable, not on
loopback, or lacks the model. Not run in CI.

### Claude Code mode

```bash
python3 alfredo-tui/eval/run_eval.py --agent claude-code --trials 3 --out claude.json
```

Per trial this runs, in the temp repo (outside this checkout), with `goal.md` on stdin:

```bash
claude -p --model haiku --output-format stream-json --verbose --max-turns 30 \
  --permission-mode acceptEdits --allowedTools 'Read,Edit,Write,Glob,Grep' \
  --disallowedTools 'WebFetch,WebSearch' --no-session-persistence \
  --setting-sources project --strict-mcp-config
```

`acceptEdits` confines Edit and Write to the working directory. There is no Bash tool, because
a shell (even `python3`) could write anywhere you can. The process gets a minimal environment
(`PATH`, `HOME`, locale, proxy/CA and Anthropic auth variables only), and only project
settings load, so your user hooks, memory and MCP servers are not used. Residual risk: Read,
Glob and Grep are not path-restricted, so the model can read files outside the temp repo.
Without Bash, Claude Code cannot run tests itself, so it may score lower than Alfredo (which
runs the approved check) on tasks like T5. Only Anthropic API-key, OAuth or subscription
login is supported; Bedrock, Vertex, `apiKeyHelper` and custom-header setups are not passed
through. It uses your logged-in Claude Code and costs real tokens, so start with `--tasks T1 --trials 1`.
The grade of its result runs in the grading sandbox, but the Claude Code process itself is not sandboxed.

## Output

`schema.json` (`schema_version: 1`; validated by `schema_check.py` before the file is written):

- `trials[]`: `task, kind, trial, pass, outcome, turns, tool_calls, tokens{prompt, eval,
  planner, peak_ctx}, errors{malformed, refused, sandbox}, repairs, wall_s, ttft_ms, rss_kb,
  exit_code, panic`, plus `cold_start_ms`, `overhead_s`, `graded_ref`, `grade_output`, `notes`.
  Chat trials add `approvals[]` (`{tool, target, outcome: applied|failed|denied|cancelled,
  reason}`, from the chat-approval log) and `chat` (`{policy, reply, steps, calls[],
  read_before_edit, applied}`). For them `turns` counts chat requests, `tool_calls` the tool
  calls, `errors.refused` denied approvals, `errors.malformed` only malformed calls (unknown
  tool, bad arguments) and `tool_errors` well-formed calls a tool refused (text not found,
  missing or excluded file, an approved edit that failed);
  `repairs`, `cold_start_ms` and `enter_to_request_ms` are `null` with a reason.
  A value the mode cannot produce is `null`, with the reason in that trial's `unavailable`.
- `config.binary` is the resolved binary path and `config.profile` its cargo profile (`debug`, `release`, or `unknown` when the binary's parent directory is neither, e.g. a copied or renamed binary, which gets no warning and no note; both `null` for Claude Code).
- `tasks`: pass counts per task; `metrics`: `M1`..`M16`, each `{name, value, unit, reason}`
  where `value` is `null` with a `reason` when it cannot be produced; `warnings`;
  `likely_broken`.

| Metric | Definition (positive tasks unless noted; names follow scoreboard #159) |
| --- | --- |
| M1 | pass@1: share of trials that pass |
| M2 | pass^3: mean over tasks of P(3 distinct trials all pass); needs >= 3 trials |
| M3 | repair recovery rate: repairs that ended accepted / repairs attempted (`null`, "no repair attempted", if none). `secondary.repairs_per_accepted_task` = repairs / accepted tasks over all tasks (Alfredo only); `secondary.per_task` = the same ratio for each task (`null` if it accepted none), the figure to compare with the per-task baseline (T2 1.0) |
| M4 | turns per task: model requests per trial, median (Alfredo planner + worker + repair requests; Claude Code `num_turns`); `secondary.per_task` = each task's own median, because the global median hides one task regressing |
| M5 | tool/command calls per task, median: Alfredo worker runs + check runs; Claude Code `tool_use` blocks; `secondary.per_task` as for M4 |
| M6 | tokens per task: prompt + completion tokens per trial, median; `secondary.peak_ctx_max` is the largest prompt + completion of one request. `secondary.per_task` as for M4. Fixture counts are synthetic (`ceil(chars / 4)`, deterministic), not model counts. Alfredo trials also cross-check the server's planner count against the `usage` saved in the plan receipt and raise a `planner_token_mismatch` warning if they differ |
| M7 | tool-call error rate: 100 x (malformed + refused + sandbox errors) / M5 calls; `null` if there were no calls |
| M8 | harness overhead per turn. With `--probes`: value is reply to screen p50 ms (server finishes sending a marker reply, to the marker appearing in raw pty bytes); `secondary` has `reply_to_screen_p95_ms`, `chat_enter_to_request_p50_ms`/`_p95_ms` (plain chat: Enter to the request reaching the server) and `chats`. `secondary.enter_to_request_p50_ms` is unchanged: the `/go` trial figure, which includes preflight, git and planning preparation, so do not score it against the chat baseline. `trial.overhead_s` is the whole non-model time. Without `--probes`: `null` |
| M9 | keystroke echo, with `--probes`: value is p95 ms from a key written to the pty to the first output byte, over idle-input keys; `secondary.echo_p50_ms`, `keys`. Target <= 8 ms. Else `null` |
| M10 | frame render cost, with `--probes`: value is chat transcript warm p95 ms from `tests/rendering_cost.rs`; `secondary` has `chat_warm_p50_ms`, `dashboard_average_ms` and per-cohort `mission_work_warm_p95_ms`/`mission_work_cold_p95_ms` (`panel@WxH`). Else `null` |
| M11 | cold start to first frame, median ms from spawn until the header is drawn in the pty (Claude Code: first stream event). Debug builds are slower: see `config.profile` and the `debug_profile` warning |
| M12 | panics/crashes: count of eval trials (all kinds) with a panic, a crash or a non-zero exit; target 0. The smoke suites are not included |
| M14 | crash recovery, with `--probes`: value is the share of kill/resume trials that recovered; `secondary.trials`, `recovered`. A trial runs the first positive task, `kill -9`s Alfredo while its worker request is parked at the fixture server, restarts it (must show `Autopilot ‖ paused` and `interrupted`), sends `/resume`, and counts it only if the task is accepted and the result grades pass. Else `null` |
| M13 | `null`: needs repeated builds |
| M15 | code health, with `--code-health`: value is the `cargo clippy -- -W clippy::pedantic` warning count (lib + bin). `secondary`: `pedantic_lib`, `pedantic_bin`, `unwrap_expect_nontest`, `largest_module {path, loc_nontest}`, `total_loc` (non-test lines of `src/**/*.rs`), `rust_tests`, `rust_tests_ignored`, `py_tests`. A part that cannot be measured is `null` with the cause in `note` (if clippy fails the value itself is `null` with its `reason`). Without the flag: `null`, reason says to pass `--code-health`. Method below |
| M16 | peak RSS during `/go`: median `VmHWM` of the main `alfredo-tui` process in kB (children and process tree not sampled); `secondary.binary_kb` is the size of the binary under test (`config.binary`); `secondary.release_binary_kb` is the same value for a release build, else `null`. A debug build carries the `debug_profile` warning |

Metrics may also carry a `note` (always-true caveats) and `secondary` values.

Error classes: `malformed` is a worker answer Alfredo could not parse (no FILE blocks, bad
JSON, truncated block); `refused` a file outside the task's allowed paths (Claude Code: a
denied permission); `sandbox` an unavailable sandbox. Claude Code reports `malformed` for
rejected edits and `sandbox` as 0. Fixture replies are instant, so `ttft_ms` is `null` there.

### Code health (M15)

`eval/code_health.py` (also runnable alone: `python3 alfredo-tui/eval/code_health.py`) measures:

- **pedantic**: `cargo clippy --locked --lib --bins --message-format=short -- -W clippy::pedantic`; counts the
  `path:line:col: warning:` lines, never the `generated N warnings` summary lines (counting every line that
  starts with `warning` gives 2 extra). lib and bin come from those summary lines, which must add up to the
  diagnostic count or the part is `null`. The number depends on the clippy version (CI pins Rust 1.96.0).
- **Non-test code**: `src/**/*.rs` minus every item gated by a test-only `#[cfg(...)]`: a predicate that names
  `test` and has no `not(`, so `#[cfg(test)]`, `#[cfg(all(test, unix))]` and `#[cfg(any(test, ...))]`. The
  gated item (module, `impl`, `fn`, `use`, struct field, statement) is skipped by brace matching, so real code
  after a test module still counts, which "everything after the first `#[cfg(test)]`" gets wrong
  (`execution.rs` has about 700 lines of production code after its first test-only item). A file-level
  `#![cfg(test)]` hides the file. Comments and string/char literals are masked first. `tests/*.rs` is test
  code and is excluded from every number here.
- **unwrap/expect**: `.unwrap(` and `.expect(` occurrences in non-test code (`unwrap_or`, `expect_err` and
  text in comments or strings do not count).
- **LOC**: physical lines (`wc -l`, blank lines and comments included) of non-test code; the largest module
  is the `src/**/*.rs` file with the most (`path` is relative to `alfredo-tui/`).
- **Tests**: `rust_tests` is the `: test` entries of `cargo test --locked -- --list` minus those of
  `-- --list --ignored` (`rust_tests_ignored`); `py_tests` is the `def test_` methods in
  `alfredo-tui/tests/test_*.py` (the `*_smoke.py` journeys are not counted).

### Broken-task guard

If no positive trial passes, `likely_broken` is `true` with a `zero_pass_rate` warning (a
single positive task at 0% gets `task_zero_pass`). A negative control that passes gets
`control_passed`. In fixture mode a task that does not grade as scripted gets
`fixture_mismatch`. Any of these makes the exit code 1.

## Probes

`--agent alfredo --probes` adds four measurements after the trials. All use the binary under test
(`config.probes.binary`/`profile`; use a release build, a debug build adds a note on M8/M9/M14), and
none touch your Ollama:

- **M8/M9**: a marker-replying fake server (every reply is a unique `RPLYnnnX` string) and a pty.
  The probe scans raw pty bytes for the marker, so reply to screen is exact to about a millisecond
  even while the spinner redraws. The server stamps the send time just before it writes the reply
  (not after the flush, which let a fast reader see the marker first), so reply to screen includes
  the write itself and is clamped at 0; `run_chat` waits up to 1 s for a late stamp, and its error
  says whether the request never arrived or only the stamp is missing. M8 runs `--probe-chats` chats; M9 types `--probe-keys` keys.
- **M10**: shells out to `cargo test --release --locked --test rendering_cost -- --ignored
  --nocapture` and parses the JSON it prints. This needs a release build of the test crate, which
  takes several minutes on 2 CPUs the first time (seconds once cached); skip it with
  `--no-render-probe`. It honours `CARGO_TARGET_DIR`.
- **M14**: `--probe-recovery-trials` kill -9 / restart / `/resume` runs (the journey of
  `tests/recovery_terminal_smoke.py`).

A probe that fails leaves only its own rows `null`, with the error as the `reason`. Probes are
timing-sensitive: they run after the trials, so use an otherwise idle machine and no parallel builds. `tests/eval_smoke.py` runs them
with tiny counts and no render probe (about 10 s).

## Limits

Tasks are tiny on purpose (deterministic, fast). They measure the harness, not model quality.
RSS covers the `alfredo-tui` process only, not its sandboxed workers.
