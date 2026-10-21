# alfredo-tui

Native Rust (ratatui) terminal that orchestrates local Ollama coding agents on a Git
repository: chat, plan, isolated sandboxed workers, review, repair, and an autopilot
loop (`/go GOAL`) that ends in one local `alfredo/go-<id>` branch.

Requirements, install, quickstart, keys, flags and troubleshooting are in the
[root README](../README.md). This page covers using the terminal day to day and
building it from source.

## Use

```bash
cd ~/code/my-repo
alfredo-tui --model qwen2.5-coder:14b
```

Opens straight into the repository root, mission `default`. Then either:

- **Autopilot**: `/go GOAL`. Plans, approves, dispatches, auto-accepts tasks whose
  approved check passes, auto-repairs failures (`--max-repairs`, default 3), and
  composes accepted work onto `alfredo/go-<id>`. F5 or `/pause` / `/resume`;
  `/stop` also cancels running workers; `/autopilot` shows status. After a restart
  the loop comes back paused and never replays work.
- **Plan first**: `/plan GOAL`, read the draft (`/plan-revise` to change it), then
  bare `/go` runs exactly that plan under autopilot, with no second planner call.
  Bare `/go` also runs tasks saved with `/plan-save` and not yet started. Your review
  stands in for autopilot's own approval of each task's files and check; a plan with
  an unrunnable check or unordered writers is still refused and left untouched.
- **Manual**: drive each step yourself.

```text
/task Create hello.py that prints Hello
/permit 1 {"files":["hello.py"],"check":["/usr/bin/python3","-B","hello.py"]}
/approve 1
/run 1
/evidence 1
/accept 1
/branch 1
```

F1 on an empty prompt lists every command. Most used:

| Command | Purpose |
| --- | --- |
| `/go GOAL`, `/go` (run the open `/plan` draft or saved tasks), `/pause`, `/resume`, `/stop`, `/autopilot` | Autopilot loop |
| `/plan REQUEST`, `/plan-revise REQUEST`, `/plan-save`, `/plan-cancel` | Generate and save a task plan |
| `/task TEXT`, `/after 1,2 TEXT` | Propose a task (with dependencies) |
| `/permit ID JSON`, `/approve [ID]`, `/assign ID MODEL` | Policy, approval, worker model |
| `/run [ID]`, `/dispatch on\|off`, `/cancel-task [ID]` | Start work |
| `/evidence [ID]`, `/review ID JSON`, `/accept [ID]`, `/reject [ID]` | Review |
| `/repair ID REASON`, `/requeue ID`, `/resolve-repair ID`, `/recover [ID] [requeue]` | Repair, rerun a cancelled task, recovery |
| `/branch [ID]` | Local review branch for an accepted task |
| `/tasks [QUERY\|#ID]`, `/activity [QUERY\|#ID]`, `/chat`, `/refresh` | Views |
| `/clean`, `/clean yes` | List, then remove, the worktrees of finished runs (see below) |
| `/models`, `/model NAME`, `/workspace` | Model and workspace switching |
| `/clear`, `/compact` | Restart this conversation (tasks are kept), or summarise its older turns now. Long chats are summarised automatically near the size limit instead of stopping (see `docs/reference.md#long-conversations-compaction-and-clear`) |
| `/scope [JSON]`, `/scope-confirm REV`, `@wayfinder REQUEST` | Project scope agreement |

Chat reads your repository itself: the model can call read-only tools (`read_file`,
`list_files`, `grep`) on the working tree, so "summarise README" works without naming
the file. Each call and a one-line cut of its result show in the transcript; see
`docs/reference.md#chat-tools-read-only`. Chat can also edit files (`edit_file`,
`write_file`): each call shows an approval card with a diff; `y` approves, `n` denies,
Esc denies and ends the turn (`docs/reference.md#chat-approval-and-file-edits`). Chat prompts also accept `@path/to/file`
mentions: the file's committed HEAD content (at most 8 files, 32 KiB) is sent with that
message; see `docs/reference.md#chat-file-mentions-path`.

`[ID]` defaults to the task selected in the task detail (F2). Everything is stored under
`~/.local/state/alfredo` (or `--state-dir`), never inside your repository. Workers
edit detached worktrees from committed HEAD; your branch, index and working files
are never modified.

Each run keeps a full checkout under the state directory and a `git worktree list`
entry in your repository; reviewed results are held by `refs/alfredo/candidates/*`.
`/clean` shows which finished runs' worktrees can go and how much disk that frees;
`/clean yes` removes them and prunes. Evidence, candidate refs, `/branch` and
`alfredo/go-*` branches stay. See the
[reference](docs/reference.md#clean-finished-run-worktrees).

## Screen

```text
 ALFREDO  default · my-repo   1 review                   ollama ✓ qwen2.5-coder warm
 Autopilot ▶ running   1/2   00:12
┌ missions ──────────────────┐┌ Task #2 · running ──────────────────────────┐
│ ● default         1/2 00:12 ││ ⠼ #2  Create tests                          │
│ · docs-cleanup         idle ││ Running · qwen2.5-coder:14b · 0:09          │
│                             ││                                             │
├ work  1/2 done ─────────────┤│ Stage    check                              │
│ ▾ textutil module         2 ││ Files    test_textutil.py                   │
│   ▤ ✓ #1 Create textutil    ││ Check    python3 -m unittest test_textutil  │
│   ▤ ⠼ #2 Create tests       ││                                             │
│       check  qwen2.5  0:09  ││ Check stdout                                │
│ ◈ ○ chat 1           ready  ││ ...                                         │
└─────────────────────────────┘└─────────────────────────────────────────────┘
```

- **Header**: mission, repository directory, attention items only when non-zero
  (`1 review`, `1 decision`, `dispatch on`) and server health. A second row appears
  only while an autopilot run exists: state, `done/total`, failures and repairs when
  present, elapsed time and branch. The goal is the group title in the tree.
- **Side pane** (left, a quarter of the width, 28–44 columns; below 88 columns one
  summary row, F6 opens it as an overlay): **missions** of this repository (current
  first; others show their saved autopilot state, `idle`, or `?` if unreadable) and
  **work**: the architect while planning or holding a draft, plan groups with their
  tasks (`▤`) and repairs (`⑂`, indented under their parent), then chats (`◈`).
  Status: braille spinner working, `◌` queued for the model, `◐` awaiting review,
  `●` decision needed, `✗` failed, `‖` blocked, `✓` complete, `○` idle/pending.
  A running task has a dim second line: stage, model, elapsed. Completed groups
  collapse while another group is active.
- **Right pane**: F2 switches between the selected task's detail and the chat.
  Task detail is labeled sections (`Files`, `Check`, `Depends`, `State`, `Next`,
  `Result`, then the diff); live worker output follows the tail. A group shows its
  goal, progress and tasks. Receipt IDs and revisions stay in F3 evidence and F4
  activity. While autopilot runs the detail follows the running task unless you
  moved the selection in the last 10 s.
- **Footer**: one line of hints for the focused area; F1 lists everything.

F6 focuses the side pane: Up/Down move, Tab switches missions/work, Enter opens the
row (a task or the architect opens its agent view, a group its detail, a chat that
conversation; another mission switches to it under the `/workspace` rules),
Alt+Left/Right fold, Esc returns to the prompt. Typed characters do not reach the
prompt while the pane has focus; the draft is kept.

### Agent view

Enter on a task (or `/watch ID`) opens the transcript of that task's worker and its
repairs, newest at the bottom, following the tail while it runs:

```
┌ Agent · worker #2 · running ────────────────────┐
│ Autopilot → worker #2                           │
│ Create greet.py and test_greet.py               │
│ files greet.py, test_greet.py · check python3 … │
│                                                 │
│ References                                      │
│ README.md                                       │
│                                                 │
│ Worker                                          │
│ ▸ greet.py                                      │
│ def greet(name):                                │
└─────────────────────────────────────────────────┘
```

Turns: the instruction sent to the model (two lines; Ctrl+O shows the full request),
read-only references (names only), the answer as code per file, the check command
with its output tail, the outcome, then each repair attempt and your notes. A run
without a retained conversation shows what its evidence holds, with a one-line note.
PageUp/PageDown scroll like the chat. Enter on the architect shows the planning
request and the streamed or saved draft.

While an agent view is open the prompt reads `To worker #2 · Enter send · Esc back`
and Enter instructs that agent (slash commands still run as commands):

| Agent state | Your note |
| --- | --- |
| generating | steers: the generation is cancelled and the task reruns with your note (a repair that does not count against the autopilot budget) |
| running its check | is queued; it becomes the repair reason if the check fails, and is dropped with `Note not needed: check passed` if it passes |
| failed, rejected | repairs it with your note as the reason |
| awaiting review | records the review as needs-repair, then repairs with your note |
| accepted | creates a follow-up task depending on it with the same files and check |
| architect planning or draft | revises the plan (`/plan-revise`) |
| held for human review | is refused; resolve it with `/review ID JSON` |

A note approves the inherited files and check only; it never widens them. It leads
the next worker request (`OWNER INSTRUCTION`, above `WHAT IS STILL FAILING`) and is
recorded in the repair reason or task title (F4 activity) and in the owner
instruction file beside the autopilot state. Autopilot keeps running: a task you
instructed is yours until the instructed run starts, then autopilot reviews it as
usual (auto-accept on pass, bounded repair on failure); a follow-up joins the run
and is integrated on a new `alfredo/go-ID-N` branch. `/tell ID TEXT` does the same
from anywhere. Esc returns to the previous pane; each agent keeps its unsent draft.

`--icons nerd|unicode|ascii` (env `ALFREDO_ICONS`) chooses record icons;
`--no-motion` (or `ALFREDO_NO_MOTION=1`) shows a static `▶`. Colours are truecolor
when `COLORTERM` is `truecolor`/`24bit`, otherwise the 16 named colours; `NO_COLOR`
turns colour off.

A finished autopilot reads `✓ done` (every planned task accepted), `◐ partial` (some
accepted) or `✗ failed` (none accepted), and the footer shows its one-line result
(`Autopilot done   1/1 accepted   git switch alfredo/go-…`). The Autopilot panel
lists `Tasks`, `Repairs`, `Branch`, one line per task, then `Review` and `Merge`
commands. In the chat, autopilot's steps for one task read as one line:
`✓ #1 planned → approved → started → check passed → accepted`; details stay in F4.

Workers also receive, read-only, the committed files the approved check names (such as
`test_cron.py` in `python3 -m unittest test_cron.py`) and files the goal or task names
verbatim. They are never writable; returning one fails the run with the offending path and
the allowed list. Repairs carry the failing check's output tail.

## Build from source

Linux x86-64, Rust 1.96.0, Git. From the repository root:

```bash
cargo build --release --locked --manifest-path alfredo-tui/Cargo.toml
./alfredo-tui/target/release/alfredo-tui --version
# or install to ~/.cargo/bin
cargo install --locked --path alfredo-tui
```

## Test

```bash
cargo fmt --manifest-path alfredo-tui/Cargo.toml -- --check
cargo clippy --locked --manifest-path alfredo-tui/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path alfredo-tui/Cargo.toml
cargo build --locked --manifest-path alfredo-tui/Cargo.toml
python3 alfredo-tui/tests/terminal_smoke.py            # PTY journeys against a fake Ollama
python3 alfredo-tui/tests/autopilot_terminal_smoke.py
python3 alfredo-tui/tests/headless_smoke.py            # --go --headless without a TTY
python3 alfredo-tui/tests/agent_terminal_smoke.py
python3 alfredo-tui/tests/inference_terminal_smoke.py
python3 alfredo-tui/tests/num_ctx_terminal_smoke.py
python3 alfredo-tui/tests/compaction_terminal_smoke.py
python3 alfredo-tui/tests/chat_tools_terminal_smoke.py   # read_file round trip; edit approve/deny; propose_plan prefill
python3 alfredo-tui/tests/recovery_terminal_smoke.py
python3 alfredo-tui/tests/qualification_cli_smoke.py
python3 alfredo-tui/tests/eval_smoke.py                # harness eval, fixture mode
python3 -m unittest discover -s alfredo-tui/tests -p 'test_*.py'
cargo audit --file alfredo-tui/Cargo.lock --deny warnings
```

The PTY smokes use `alfredo-tui/target/debug/alfredo-tui`; set
`ALFREDO_TUI_BINARY=/path/to/alfredo-tui` when using another binary or
`CARGO_TARGET_DIR`. They need Linux, Git and bubblewrap; Python is a test-only
dependency.

`ALFREDO_TEST_BWRAP_PATH` is a test-only seam: it redirects only the bubblewrap
preflight probe (`--doctor`, `/go`, `--go`, `/run`) so a smoke can simulate a missing
bwrap. It never changes the executable workers run, so it cannot weaken the sandbox.
Do not set it outside tests.

The harness eval (`eval/`) measures pass rate, repairs, turns, tokens, overhead, cold start
and memory on a fixed task set, for Alfredo or Claude Code; see
[eval/README.md](eval/README.md).

Optional live checks against a real local model (`ALFREDO_SMOKE_MODEL`, default
`qwen2.5-coder:14b`):

```bash
cargo test --locked --manifest-path alfredo-tui/Cargo.toml --test live -- --ignored --nocapture
cargo test --locked --manifest-path alfredo-tui/Cargo.toml --test worker live_local_model -- --ignored --nocapture
```

## Package a release archive

```bash
python3 alfredo-tui/scripts/package_release.py --output /tmp/alfredo-release   # dir must be new
python3 alfredo-tui/tests/release_smoke.py /tmp/alfredo-release/*.tar.gz
```

The archive `alfredo-tui-VERSION-x86_64-unknown-linux-gnu.tar.gz` (plus `.sha256`)
contains the binary, [INSTALL.md](INSTALL.md), `LICENSE`, `Cargo.lock`, `BUILD.json`
(provenance: compiler, commit, dirty flag, checksums), `DEPENDENCIES.json` and
`THIRD_PARTY_NOTICES.txt`. Packaging requires Rust 1.96.0 on x86_64 Linux GNU,
is deterministic for identical inputs on one host, honors `CARGO_TARGET_DIR`, and
never publishes. The release smoke verifies checksums and notices, rejects a
corrupted copy, installs the binary into a temporary `PATH` and runs every PTY
journey against it.

CI: [.github/workflows/rust-terminal.yml](../.github/workflows/rust-terminal.yml)
runs all of the above; [release.yml](../.github/workflows/release.yml) builds the
archive on a `v*` tag and creates a **draft** GitHub release.

## Source layout

| Path | Contents |
| --- | --- |
| `src/main.rs` | CLI flags, event loop, key handling |
| `src/ui.rs`, `src/model.rs` | Rendering and conversation state |
| `src/compaction.rs` | Chat compaction: summarise older turns into a note, `/clear`, `/compact` |
| `src/side_pane.rs`, `src/theme.rs` | Side pane projection and keys; icons, status glyphs, palette, spinner |
| `src/autopilot.rs` | `/go` loop, auto-review, bounded repair, integration branch |
| `src/tasks.rs`, `src/task_control.rs`, `src/dispatch.rs` | Durable task store, commands, dispatch |
| `src/worker.rs`, `src/run_boundary.rs` | Isolated worktree workers and bubblewrap sandbox |
| `src/planner.rs`, `src/planning_context.rs` | Plan generation from committed repo context |
| `src/provider.rs`, `src/inference_admission.rs`, `src/health.rs` | Ollama client, shared capacity, health polling |
| `src/commands.rs` | Command catalog (F1 picker) |
| `src/execution.rs` | Bounded process execution: limits, sandbox, process-group cleanup, execution receipts |
| `src/wake.rs` | Run-loop wake source: one channel for terminal input and "a result is ready" wakes |
| `src/headless.rs` | `--go GOAL --headless`: autopilot without a terminal, JSON result on stdout |
| `src/on_finish.rs` | `--on-finish PROGRAM` completion hook (no shell, outside the sandbox, 10 s limit) |
| `src/clean.rs` | `/clean`: removes retained worktrees of finished runs only |
| `src/chat_mentions.rs`, `src/chat_context.rs` | `@path` mentions (committed HEAD content) and harness context for plain chat turns |
| `src/chat_tools/mod.rs`, `src/chat_tools/read.rs`, `src/chat_tools/git_read.rs`, `src/chat_tools/run.rs`, `src/chat_tools/edit.rs`, `src/chat_tools/approval.rs`, `src/chat_tools/propose.rs` | Chat tool registry (definitions, tiers, dispatch, path confinement), read-only file and git tools, capped tool loop, write_file/edit_file, approval gate (card, decision, outcome log), propose_plan (prefills /plan or /go) |
| `src/workstation.rs`, `src/wayfinder.rs` | One admitted workspace/mission plus its conversation owner; first-contact routing |
| `src/selection.rs`, `src/selection_command.rs`, `src/selection_store.rs`, `src/missions.rs` | Startup workspace selection, its intent and journal; mission namespace discovery |
| `src/conversations.rs`, `src/reading.rs`, `src/transcript_cache.rs`, `src/agent_drafts.rs` | Conversation snapshots, reading positions, rendered-message cache, unsent agent drafts |
| `src/agent.rs`, `src/agent_view.rs`, `src/instruct.rs`, `src/assignment.rs`, `src/architecture.rs` | Local Agent continuity, agent view, owner instructions, `/assign`, Architect revisions |
| `src/planner_command.rs`, `src/plan_lint.rs`, `src/understanding.rs` | Saved planner provenance, unattended-plan lint, project understanding |
| `src/review.rs`, `src/assessment.rs`, `src/dependencies.rs`, `src/branch.rs` | Review projection, reviewer assertions, composing accepted results, branch handoff |
| `src/command_intent.rs`, `src/console_command.rs`, `src/control_command.rs` | Persistable command intent and operator actions (never approval or proof of effect) |
| `src/activity.rs`, `src/task_view.rs`, `src/mission_work.rs`, `src/dashboard.rs`, `src/metrics.rs`, `src/client_timing.rs` | Read-only projections: activity, task search, work tree, dashboard helpers, timing |
| `src/doctor.rs`, `src/models.rs`, `src/inference_profile.rs`, `src/inference_runtime.rs` | `--doctor`, model-name comparison, diagnostic request identities and runtime observations |
| `src/qualification.rs`, `src/qualification_oracle.rs`, `src/qualification_runner.rs` | Opt-in model qualification experiments and fixtures |
| `src/lib.rs` | Library crate root: module declarations shared by the binary and integration tests |
| `tests/*.rs`, `tests/*_smoke.py` | Integration tests and PTY smokes |
| `scripts/` | Packaging and third-party notice generation |
| `eval/` | Reproducible harness eval and metrics ([eval/README.md](eval/README.md)) |

## Reference

- [docs/reference.md](docs/reference.md): detailed behavior and guarantees (storage
  limits, receipts, review/repair rules, scope, recovery, diagnostics).
- [docs/install-reference.md](docs/install-reference.md): state formats, upgrade and
  selection-journal details.
- [Launch acceptance](../.agent/Tasks/launch-acceptance.md) and
  [status](../.agent/Tasks/STATUS.md).
