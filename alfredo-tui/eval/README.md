# Harness eval

One command that runs a fixed task set through Alfredo (or Claude Code on the same repos and
the same graders) and prints schema-valid JSON with the harness metrics `M1`-`M16` from
scoreboard #159. Standard library Python only.

```bash
cargo build --manifest-path alfredo-tui/Cargo.toml        # the binary under test
python3 alfredo-tui/eval/run_eval.py --agent alfredo --backend fixture --trials 3 --out out.json
```

Fixture mode is deterministic, needs no model, and takes about 40 s on 2 CPUs (6 tasks x 3
trials). CI runs a one-trial pass of it (`tests/eval_smoke.py`).

## How a trial works

1. Copy `tasks/<id>/repo/` into a fresh temp directory, `git init`, commit it on `main`.
2. Run the agent on `goal.md`:
   - **Alfredo**: start the TUI in a pty with a fresh `--state-dir`, send `/go <goal>`, wait for
     `git switch alfredo/go-` (done) or `Autopilot ✗ failed`, then quit with Ctrl+Q.
   - **Claude Code**: run it with the goal on stdin in the temp repo (see below).
3. Grade the candidate without reading any screen or log: `git archive` the `alfredo/go-*`
   branch (Claude Code: a commit of the final working tree), extract it, and run the task's
   `check.sh` there. Exit 0 is a pass. No branch means fail.
4. Delete the temp directory (use `--keep` to inspect it).

`check.sh` lives outside the repo the agent sees, so an agent cannot read or edit its grader.
`python3 alfredo-tui/eval/run_eval.py --agent alfredo --validate-tasks` checks that every
`check.sh` fails on its untouched repo.

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

The negative control is scripted, so it runs only with `--backend fixture`; other modes list
it under `config.skipped_tasks`. `--tasks T1,T4` selects tasks by id or prefix.

`answers.json` has `plan` (what the fixture planner returns) and `workers`: a list of
`{"prefix": ..., "replies": [...]}` matched in order against the start of the worker prompt
(`Implement this task: ...`, repairs start `Implement this task: Repair #`). Replies are
consumed in order and the last repeats; a reply is `{"files": [...]}` or `{"prose": "..."}`.

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
| `--binary PATH` | `alfredo-tui` binary (default `$ALFREDO_TUI_BINARY`, else `target/debug`) |
| `--ollama-endpoint URL` | default `http://127.0.0.1:11434`; must be a loopback address |
| `--tmp-root DIR`, `--keep` | where temp repos go; keep them |

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

## Output

`schema.json` (`schema_version: 1`; validated by `schema_check.py` before the file is written):

- `trials[]`: `task, kind, trial, pass, outcome, turns, tool_calls, tokens{prompt, eval,
  planner, peak_ctx}, errors{malformed, refused, sandbox}, repairs, wall_s, ttft_ms, rss_kb,
  exit_code, panic`, plus `cold_start_ms`, `overhead_s`, `graded_ref`, `grade_output`, `notes`.
  A value the mode cannot produce is `null`, with the reason in that trial's `unavailable`.
- `tasks`: pass counts per task; `metrics`: `M1`..`M16`, each `{name, value, unit, reason}`
  where `value` is `null` with a `reason` when it cannot be produced; `warnings`;
  `likely_broken`.

| Metric | Definition (positive tasks unless noted; names follow scoreboard #159) |
| --- | --- |
| M1 | pass@1: share of trials that pass |
| M2 | pass^3: mean over tasks of P(3 distinct trials all pass); needs >= 3 trials |
| M3 | repair recovery rate: repairs that ended accepted / repairs attempted (`null`, "no repair attempted", if none). `secondary.repairs_per_accepted_task` = repairs / accepted tasks (Alfredo only) |
| M4 | turns per task: model requests per trial, median (Alfredo planner + worker + repair requests; Claude Code `num_turns`) |
| M5 | tool/command calls per task, median: Alfredo worker runs + check runs; Claude Code `tool_use` blocks |
| M6 | tokens per task: prompt + completion tokens per trial, median; `secondary.peak_ctx_max` is the largest prompt + completion of one request. Fixture counts are synthetic (`ceil(chars / 4)`, deterministic), not model counts |
| M7 | tool-call error rate: 100 x (malformed + refused + sandbox errors) / M5 calls; `null` if there were no calls |
| M8 | harness overhead per turn (Enter to request at server, reply to screen, p50 ms): `null`. Reply-to-screen is not measured because spinner redraws make byte-level change detection unreliable at the 30 ms poll. `secondary.enter_to_request_p50_ms` is measured exactly, but for `/go` it includes preflight, git and planning preparation (about 1 s), so do not score it against the ~50 ms chat baseline; `trial.overhead_s` is the whole non-model time |
| M9 | keystroke echo p50/p95: `null`, the `rendering_cost.rs` bench wrapper is a follow-up |
| M10 | frame render cost p50/p95: `null`, same follow-up |
| M11 | cold start to first frame, median ms from spawn until the header is drawn in the pty (Claude Code: first stream event) |
| M12 | panics/crashes: count of eval trials (all kinds) with a panic, a crash or a non-zero exit; target 0. The smoke suites are not included |
| M13, M14, M15 | `null`: need repeated builds, a kill probe or static analysis |
| M16 | peak RSS during `/go`: median `VmHWM` of the main `alfredo-tui` process in kB (children and process tree not sampled); `secondary.release_binary_kb` is the size of `target/release/alfredo-tui` when it exists, else `null` |

Metrics may also carry a `note` (always-true caveats) and `secondary` values.

Error classes: `malformed` is a worker answer Alfredo could not parse (no FILE blocks, bad
JSON, truncated block); `refused` a file outside the task's allowed paths (Claude Code: a
denied permission); `sandbox` an unavailable sandbox. Claude Code reports `malformed` for
rejected edits and `sandbox` as 0. Fixture replies are instant, so `ttft_ms` is `null` there.

### Broken-task guard

If no positive trial passes, `likely_broken` is `true` with a `zero_pass_rate` warning (a
single positive task at 0% gets `task_zero_pass`). A negative control that passes gets
`control_passed`. In fixture mode a task that does not grade as scripted gets
`fixture_mismatch`. Any of these makes the exit code 1.

## Limits

Tasks are tiny on purpose (deterministic, fast). They measure the harness, not model quality.
RSS covers the `alfredo-tui` process only, not its sandboxed workers.
