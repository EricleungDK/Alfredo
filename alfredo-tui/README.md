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
  approved check passes, auto-repairs failures (`--max-repairs`, default 2), and
  composes accepted work onto `alfredo/go-<id>`. F5 or `/pause` / `/resume`;
  `/stop` also cancels running workers; `/autopilot` shows status. After a restart
  the loop comes back paused and never replays work.
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
| `/go GOAL`, `/pause`, `/resume`, `/stop`, `/autopilot` | Autopilot loop |
| `/plan REQUEST`, `/plan-revise REQUEST`, `/plan-save`, `/plan-cancel` | Generate and save a task plan |
| `/task TEXT`, `/after 1,2 TEXT` | Propose a task (with dependencies) |
| `/permit ID JSON`, `/approve [ID]`, `/assign ID MODEL` | Policy, approval, worker model |
| `/run [ID]`, `/dispatch on\|off`, `/cancel-task [ID]` | Start work |
| `/evidence [ID]`, `/review ID JSON`, `/accept [ID]`, `/reject [ID]` | Review |
| `/repair ID REASON`, `/resolve-repair ID`, `/recover [ID]` | Repair and recovery |
| `/branch [ID]` | Local review branch for an accepted task |
| `/tasks [QUERY\|#ID]`, `/activity [QUERY\|#ID]`, `/chat`, `/refresh` | Views |
| `/models`, `/model NAME`, `/workspace` | Model and workspace switching |
| `/scope [JSON]`, `/scope-confirm REV`, `@wayfinder REQUEST` | Project scope agreement |

`[ID]` defaults to the task selected in Mission Work (F2). Everything is stored under
`~/.local/state/alfredo` (or `--state-dir`), never inside your repository. Workers
edit detached worktrees from committed HEAD; your branch, index and working files
are never modified.

## Dashboard

`/go` opens the dashboard (F2): one line per task (`✓` accepted, `▶` running, `○` pending,
`◐` awaiting review, `✗` failed, `‖` held/blocked) with `done/total` over planned tasks
(a task fixed by an accepted repair counts as done; repairs are counted separately). The
right pane shows the selected task's live worker output, or once finished its outcome,
the last 40 lines of failing check output (stderr, else stdout), diff and full check
output. While autopilot runs it follows the running task unless you moved the selection
in the last 10 s. Receipt IDs and revisions stay in F3 evidence and F4 activity.

A finished autopilot reads `✓ done` (every planned task accepted), `◐ partial` (some
accepted) or `✗ failed` (none accepted), and the footer shows its one-line result.

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

The crate compiles the shared execution provider from
`mission-control/src-tauri/src/execution.rs`, so build from a full checkout.

## Test

```bash
cargo fmt --manifest-path alfredo-tui/Cargo.toml -- --check
cargo clippy --locked --manifest-path alfredo-tui/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path alfredo-tui/Cargo.toml
cargo build --locked --manifest-path alfredo-tui/Cargo.toml
python3 alfredo-tui/tests/terminal_smoke.py            # PTY journeys against a fake Ollama
python3 alfredo-tui/tests/autopilot_terminal_smoke.py
python3 alfredo-tui/tests/inference_terminal_smoke.py
python3 alfredo-tui/tests/recovery_terminal_smoke.py
python3 alfredo-tui/tests/qualification_cli_smoke.py
python3 -m unittest discover -s alfredo-tui/tests -p 'test_*.py'
cargo audit --file alfredo-tui/Cargo.lock --deny warnings
```

The PTY smokes use `alfredo-tui/target/debug/alfredo-tui`; set
`ALFREDO_TUI_BINARY=/path/to/alfredo-tui` when using another binary or
`CARGO_TARGET_DIR`. They need Linux, Git and bubblewrap; Python is a test-only
dependency.

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
| `src/autopilot.rs` | `/go` loop, auto-review, bounded repair, integration branch |
| `src/tasks.rs`, `src/task_control.rs`, `src/dispatch.rs` | Durable task store, commands, dispatch |
| `src/worker.rs`, `src/run_boundary.rs` | Isolated worktree workers and bubblewrap sandbox |
| `src/planner.rs`, `src/planning_context.rs` | Plan generation from committed repo context |
| `src/provider.rs`, `src/inference_admission.rs`, `src/health.rs` | Ollama client, shared capacity, health polling |
| `src/commands.rs` | Command catalog (F1 picker) |
| `tests/*.rs`, `tests/*_smoke.py` | Integration tests and PTY smokes |
| `scripts/` | Packaging and third-party notice generation |

## Reference

- [docs/reference.md](docs/reference.md): detailed behavior and guarantees (storage
  limits, receipts, review/repair rules, scope, recovery, diagnostics).
- [docs/install-reference.md](docs/install-reference.md): state formats, upgrade and
  selection-journal details.
- [Launch acceptance](../.agent/Tasks/launch-acceptance.md) and
  [status](../.agent/Tasks/STATUS.md).
