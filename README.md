<div align="center">

# Alfredo

**Give a local model a goal. Get back a reviewed Git branch.**

A native terminal that plans, runs, checks and repairs coding tasks with your own
[Ollama](https://ollama.com) models, entirely on your machine.

[![CI](https://github.com/EricleungDK/Alfredo/actions/workflows/rust-terminal.yml/badge.svg)](https://github.com/EricleungDK/Alfredo/actions/workflows/rust-terminal.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.96](https://img.shields.io/badge/rust-1.96-orange.svg)](alfredo-tui/Cargo.toml)
[![Platform: Linux x86-64](https://img.shields.io/badge/platform-linux--x86--64-lightgrey.svg)](#requirements)

[Quickstart](#60-second-quickstart) ·
[Install](#install) ·
[How it works](#how-it-works) ·
[Keys](#keys) ·
[Flags](#flags) ·
[Docs](#documentation) ·
[Contributing](CONTRIBUTING.md)

</div>

<p align="center">
  <img src="docs/assets/tui-recording.gif" alt="alfredo-tui in a sample repository: the header shows the model warm, /go starts autopilot, two tasks run in isolated worktrees, one check fails and is repaired, the agent view shows the repair worker, and the summary names the review branch while main stays untouched" width="860">
</p>

<sub>A real run, not a mock-up: `qwen2.5-coder:14b` on a throwaway sample repo, 25 s, one failed check
repaired automatically; the two model waits are cut and labelled. [MP4](docs/assets/tui-recording.mp4) ·
[WebM](docs/assets/tui-recording.webm) · [poster](docs/assets/tui-recording-poster.png) ·
[recording script](alfredo-tui/recording/record.py) · [still: repair](docs/assets/screenshot-repair.png) ·
[still: done](docs/assets/screenshot-done.png)</sub>

Type `/go Add a --verbose flag to the CLI and a test for it`. Alfredo plans the work,
runs each task in an isolated Git worktree inside a sandbox, runs the task's check,
repairs failures, and leaves one local branch, `alfredo/go-<id>`, for you to review.
`/go` never touches your branch, index or working files (only chat edits you approve do), nothing is pushed, and
nothing leaves your machine.

## Why Alfredo

- **Local-first.** Ollama models on your hardware. No API keys, no cloud; the only
  network peer is your Ollama endpoint.
- **Your repo stays yours.** Workers edit detached worktrees from committed `HEAD`.
  The result is a local branch you merge, or don't. Alfredo does leave a little Git
  state behind; see [What Alfredo leaves behind](#what-alfredo-leaves-behind).
- **Sandboxed.** Workers and their checks run under bubblewrap with resource limits
  and a task-scoped file and command policy. A worker cannot widen its own scope.
- **Checks decide, not vibes.** A task is accepted when its approved check passes.
  Failures get bounded, targeted repairs (default 3 per task) with the failing test
  output attached. Risky or held reviews always wait for you.
- **You can watch and steer.** Open any agent to read its transcript live, then
  instruct it: steer a running worker, queue a note, repair a failure, or add a
  follow-up to accepted work.
- **Durable.** Tasks, receipts and evidence survive restarts. After a crash the
  autopilot comes back paused and never replays work.
- **Native and fast.** One Rust binary on ratatui. No Node, Python or browser to run it.

## How it works

```text
 /go GOAL
    │
    ▼
 Architect ──▶ plan: tasks + files + checks + dependencies
    │
    ▼
 for each ready task (dependencies respected, --parallel-models at a time)
    ├─▶ worker (Ollama) writes the files in an isolated worktree
    ├─▶ sandboxed check runs (bwrap + prlimit, read-only system)
    ├─▶ pass ──▶ accepted        fail ──▶ repair with the failure output (bounded)
    └─▶ risky or held ──▶ waits for your review
    │
    ▼
 accepted work is composed onto one local branch: alfredo/go-<id>
```

Every autopilot choice is an ordinary command (`/plan`, `/approve`, `/run`,
`/accept`, …) saved with a receipt, so you can take over at any step. Details:
[alfredo-tui/docs/reference.md](alfredo-tui/docs/reference.md).

## Requirements

- Linux x86-64 (WSL2 works; see [WSL notes](#wsl-notes)).
- Git at `/usr/bin/git`, and a repository with at least one commit.
  Alfredo refuses a repository whose Git config sets filters (`filter.*`), includes
  (`include.*`, `includeIf.*`), a partial clone (`extensions.partialClone`, `remote.*.promisor`),
  transport or ssh/proxy programs (`remote.*.uploadPack`, `core.sshCommand`, `core.gitProxy`),
  `protocol.*.allow`, or `core.excludesFile`, `core.attributesFile`, `mailmap.file`,
  `diff.orderFile`: `/go`, `--go` and `--doctor` name the key and the fix,
  `git config --local --unset KEY`.
- [Ollama](https://ollama.com) running locally, plus a model, e.g.
  `ollama pull qwen2.5-coder:14b`.
- For coding workers: bubblewrap (`/usr/bin/bwrap`) and prlimit (`/usr/bin/prlimit`,
  from util-linux). Debian/Ubuntu: `sudo apt install git bubblewrap util-linux`.
- To build from source: Rust 1.96 (`rustup toolchain install 1.96.0`).

Chat works without the worker tools; only coding tasks need them.

Chat's file tools (read, search, edit) need a tool-capable Ollama model. If the model
rejects tools, chat shows one notice and continues as plain chat without them.

## Install

From source:

```bash
git clone https://github.com/EricleungDK/Alfredo.git
cd Alfredo
cargo install --locked --path alfredo-tui   # installs ~/.cargo/bin/alfredo-tui
alfredo-tui --version
```

From a release archive (`alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz` plus its
`.sha256`):

```bash
sha256sum -c alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz
install -m 755 alfredo-tui-0.1.0-x86_64-unknown-linux-gnu/alfredo-tui ~/.local/bin/
alfredo-tui --version
```

See [alfredo-tui/INSTALL.md](alfredo-tui/INSTALL.md) for upgrade, uninstall and state notes.

## 60-second quickstart

```bash
cd ~/code/my-repo                          # any Git repo with a commit
alfredo-tui --doctor --model qwen2.5-coder:14b   # optional preflight (exit 0 = ok)
alfredo-tui --model qwen2.5-coder:14b
```

1. The main screen opens on the repository root, mission `default`. No setup prompts.
   The header shows server health (`ollama ✓ MODEL warm`).
2. Type `/go Add a --verbose flag to the CLI and a test for it` and press Enter.
   Autopilot plans tasks, approves them, runs workers, auto-accepts tasks whose
   approved check passes, and retries failures (up to 3 repairs per task).
   Prefer to review first? `/plan Add a --verbose flag to the CLI and a test for it`
   shows the draft (`/plan-revise` to change it); a bare `/go` then runs exactly that
   plan with no second planner call. Bare `/go` also runs tasks you saved with
   `/plan-save` and have not started.
3. Press F5 to pause or resume at any time; `/stop` also cancels running workers;
   `/autopilot` shows status.
4. When done, the summary names the branch `alfredo/go-<id>`. Your HEAD, index and
   working files were never touched. Review and merge it yourself:

```bash
git log --stat HEAD..alfredo/go-<id>   # what autopilot produced
git switch alfredo/go-<id>             # optional: try it, then switch back
git merge alfredo/go-<id>              # on your branch, when satisfied
```

You can also start unattended: `alfredo-tui --model qwen2.5-coder:14b --go "GOAL"`.
For scripts and CI, add `--headless`: it runs without a terminal, prints progress lines
on stderr and ends with one JSON object on stdout
(`{state, goal, branch, accepted, total, repairs, failed:[{id,title,reason[,last_failure]}], usage}`);
SIGINT/SIGTERM cancel workers like `/stop`; `--resume --headless` continues a paused run. Exit 2 means setup failed (Ollama down, model
not installed) with the reason on stderr; with `--on-finish`, headless waits for the hook
(10 s limit) after printing the JSON and reports a failed hook on stderr.
Manual control is always available (`/plan`, `/task`, `/approve`, `/run`,
`/evidence`, `/accept`, …); press F1 on an empty prompt for the command list.

Chat can also act: it reads and searches the repository, and with your approval edits files
(`write_file`, `edit_file`), runs shell commands (`run_command`) and does git writes. Each
needs a `y / n / a` card (`a` adds an always-allow rule; `/allow` lists and removes them). For
large work it calls `propose_plan`, which only prefills `/plan` or `/go` for you to confirm.
Tool list and details: [alfredo-tui/README.md](alfredo-tui/README.md) and
[the reference](alfredo-tui/docs/reference.md#chat-always-allow-rules).

## Keys

| Key | Action |
| --- | --- |
| Enter | Send prompt or command |
| Enter (while a reply runs) | Queue the prompt (`You · queued`); it is sent when the reply completes. Esc or a failed reply returns it to the prompt, never sent |
| Ctrl+J / Alt+Enter | New line on any terminal; Shift+Enter too where the terminal reports it, and paste also works. The prompt box grows to show up to six lines |
| F1 | Command picker (empty prompt; type to filter); typing `/` lists matching commands, Tab or Enter completes |
| F2 | Right pane: task detail / conversation |
| F3 | Evidence for the selected task (task detail) |
| F4 | Activity (saved task receipts) |
| F5 | Pause / resume autopilot |
| F6 | Focus the side pane (overlay below 88 columns); again or Esc returns to the prompt |
| Up / Down | Move between lines of a multi-line draft, then prompt history from its first / last line; in task detail, select task; in the side pane, move |
| Tab (side pane) | Switch between missions and work |
| Enter (side pane) | Open the row: a task's or the architect's agent view, group detail, chat, or switch mission |
| Enter (agent view) | Instruct that agent: steer, queued note, repair, follow-up or plan revision |
| Esc (agent view) | Back to the previous pane; the agent's unsent draft is kept |
| Esc (plan draft, while `/plan` or `/plan-revise` generates) | With an empty prompt, cancel the request exactly like `/plan-cancel` (footer: `Esc cancel plan`); once the draft is shown, Esc keeps it. This outranks stopping a chat reply while the plan pane is open; `/go` planning stays on `/stop` |
| Ctrl+O (agent view) | Expand / collapse the full instruction text |
| Ctrl+O (conversation) | Show tool results in full (200 lines each) / one line |
| Ctrl+O (Plan draft) | Readable task list / raw draft JSON |
| Alt+Left / Alt+Right | Collapse / expand a task branch |
| PageUp / PageDown | Scroll transcript, agent view, details or evidence |
| Home / End (empty prompt), Ctrl+Home / Ctrl+End | Jump to the oldest / latest transcript row; End resumes following new output. With text in the prompt, Home / End move the cursor |
| Tab / Shift+Tab | Next / previous conversation |
| Ctrl+N | New conversation (max 8); `/clear` restarts the current one, `/compact` summarises its older turns |
| Esc | Cancel the current model request (keeps partial reply); close pickers. You can then send a new prompt or switch `/model`. Esc (or Ctrl+Q, which also quits) stops a running chat `run_command`; `/stop` does too |
| Ctrl+R | Retry a failed or cancelled turn, until you send a new prompt |
| y / n / a (approval card) | Approve / deny / always allow (`a`, see `/allow`; not offered for pushes, HOST (escalated) runs, over-long commands, or writes that create directories) a chat tool call: file edit, shell command, git write or push; Esc denies and ends the turn |
| Ctrl+A / Ctrl+E | Move to the start / end of the prompt |
| Ctrl+Left / Ctrl+Right (Alt+B / Alt+F) | Move one word left / right |
| Ctrl+K | Delete from the cursor to the end of the prompt |
| Ctrl+W / Ctrl+U | Delete word / clear prompt |
| Ctrl+Q | Quit (cancels workers and waits for their results) |
| Ctrl+C | While a chat reply is running (and no worker is): cancel it like Esc, keeping the prompt draft and the session. Otherwise quit like Ctrl+Q |

## What Alfredo leaves behind

Everything is under the state directory except a few Git entries in your repository's
`.git`, none of which touch your branch, index or working files:

- **Run worktrees.** Each worker run checks out the whole repository into
  `<state>/rust-tasks-v1/<id>/runs/<run>/worktree` (one full checkout per run, so disk
  use grows with every run) and registers it, so `git worktree list` in your repository
  shows it.
- **`refs/alfredo/candidates/<sha>`** keep each reviewed result reachable, and
  **`refs/alfredo/bases/<sha>`** pin a composed baseline for dependent tasks. Hidden
  from `git branch`; see them with `git for-each-ref refs/alfredo`. They are small.
- **`alfredo/go-<id>`** branches are the autopilot results you merge or delete yourself.

Cleanup is manual and explicit. `/clean` lists the worktrees it would remove with their
total size; `/clean yes` removes those of finished runs (accepted, rejected, failed or
cancelled) and runs `git worktree prune` (skipped if you have stale worktree entries of
your own). It keeps runs that are active, interrupted or
awaiting review, any run whose saved evidence or candidate ref is missing, and runs whose partial work
was not captured in the evidence (interrupted and recovered). The
evidence (F3), candidate refs, `/branch` and your `alfredo/go-*` branches are untouched,
and it is refused while autopilot or a worker is active. Failed runs have no candidate
to save; their diff is kept in the run's evidence. See
[the reference](alfredo-tui/docs/reference.md#clean-finished-run-worktrees).

## Flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `--model NAME` | `qwen2.5-coder:14b` (env `ALFREDO_MODEL`) | Model for new conversations (a restored conversation keeps its own in the terminal). With `--headless` and `--doctor`, an explicit `--model` is used for that run even over a saved model, checked first (exit 2 names it) and never saved; the headless JSON reports it as `model` |
| `--endpoint URL` | `http://127.0.0.1:11434` (env `OLLAMA_HOST`) | Ollama HTTP origin |
| `--go GOAL` | off | Start autopilot on launch |
| `--headless` | off | With `--go GOAL`: no terminal needed. One stderr line per autopilot state change, then one JSON object on stdout. Exit 0 all accepted, 1 partial/failed/interrupted, 2 setup or usage error (including Ollama unreachable or model missing); waits for and reports an `--on-finish` hook |
| `--chat PROMPT` | – | With `--headless`: run one Chat Agent turn (tools, approvals) without a terminal and print one JSON result. Mutating calls are denied unless `--approve` allows them. PROMPT `-` reads the prompt from stdin to EOF (UTF-8, at most 256 KiB, nonempty; otherwise exit 2), e.g. `git diff \| alfredo-tui --chat - --headless` |
| `--resume` | – | Only with `--headless`: continue the mission's paused run (as `/resume` does) with the same stderr lines, JSON and exit codes as `--go`; exit 2 when there is nothing to resume. `--go GOAL --headless` over a paused run exits 2 naming it and plans nothing; use `--conversation NAME` for a separate run |
| `--approve workspace-edits` | deny all | Only with `--chat … --headless`: approve `edit_file`/`write_file` inside the repository, deny everything else (outcomes logged to `chat-approvals.jsonl`). Interactive chat always asks |
| `--max-repairs N` | 3 | Auto-repairs per task, 0–16 (0 disables) |
| `--on-finish PROGRAM` | off (env `ALFREDO_ON_FINISH`) | Absolute path to your own executable, run when an autopilot run ends `done`, `partial` or `failed` (see [reference](alfredo-tui/docs/reference.md#completion-hook)) |
| `--workspace DIR` | current repo | Open the repository containing `DIR` |
| `--mission NAME` / `--new-mission NAME` | `default` | Resume / create a named mission in the repository of the current directory or of `--workspace`; outside a repository `--workspace DIR` is required |
| `--select` | off | Always show the repository/mission selector |
| `--conversation NAME` | `default` | Named conversation set (one terminal owns a set) |
| `--state-dir DIR` | `~/.local/state/alfredo` (env `ALFREDO_STATE_DIR`) | State location; must be outside the repo |
| `--keep-alive VALUE` | `30m` (env `ALFREDO_KEEP_ALIVE`) | Ollama `keep_alive`: `30m`, `300`, `-1`, or `default` |
| `--num-ctx N` | unset (env `ALFREDO_NUM_CTX`) | Ollama input window in tokens, 2048–262144, sent on every request; reply length is clamped to the room left, and prompts that leave under 256 tokens are refused, not truncated. Unset: server default |
| `--max-turns N` | `8` (env `ALFREDO_MAX_TURNS`) | Tool rounds one chat turn may run, 1–32, interactive and `--chat --headless`; a model that asks for more is stopped with a notice naming N. Out of range or non-numeric exits 2 |
| `--connect-retries N` | 3 | Auto-retry (1 s, 2 s, 4 s…) before any reply text, 0–10 |
| `--parallel-models N` | 2 | Concurrent model requests per endpoint across terminals, 1–8 |
| `--structured-thinking auto\|on\|off` | `off` | Thinking mode for structured (plan/worker) requests |
| `--worker-format blocks\|json` | `blocks` | Worker answer format: plain-text FILE blocks, or legacy schema-constrained JSON (see [reference](alfredo-tui/docs/reference.md#worker-answer-format)) |
| `--icons nerd\|unicode\|ascii` | `unicode` (env `ALFREDO_ICONS`) | Record icons in the side pane (task, repair, agent) |
| `--no-motion` | off (env `ALFREDO_NO_MOTION=1`) | Static `▶` instead of the working spinner |
| `--doctor` | | Check storage, model, repo and worker tools; no TTY; exit 2 on failure |
| `--qualify-inference REPORT` | | Opt-in model diagnostic run (see [reference](alfredo-tui/docs/reference.md#run-an-explicit-inference-diagnostic)) |
| `--qualification-repetitions N` | 3 | Repetitions for `--qualify-inference`, 1–3 |
| `--inspect-qualification REPORT` | | Summarize a saved diagnostic report |
| `--help`, `--version` | | |

## Troubleshooting

- **Header shows `ollama ✗ retrying`, or doctor says "Cannot reach Ollama"**: start
  Ollama (`ollama serve` or your system service) and check `--endpoint`. Alfredo
  recovers automatically once the server is back; no restart needed.
- **`OLLAMA_HOST`**: Ollama's own forms (`host:port`, `0.0.0.0`) are accepted;
  `0.0.0.0` connects to `127.0.0.1`. `--endpoint` overrides it.
- **Model not installed** (doctor: "is not listed"): `ollama pull MODEL`, or use
  `/model NAME` inside the terminal (it checks installed models first), or `/models`
  then Up/Down and Enter.
- **Doctor: `FAIL installed worker tool: /usr/bin/bwrap`** (or prlimit/git): install
  `bubblewrap` / `util-linux` / `git`. The paths are fixed. Without bubblewrap, `/go`
  and `/run` refuse with the same fix instead of spending repairs.
- **Worker fails at sandbox start** (e.g. "Permission denied" setting up namespaces):
  your distro restricts unprivileged user namespaces (Ubuntu 24.04+ AppArmor). Test
  with `bwrap --ro-bind / / --unshare-user --unshare-pid --unshare-net true` and allow
  bwrap per your distro's policy.
- **"Interactive terminal required"**: run in a real terminal, or use `--headless --go GOAL` for scripts.
- **"Another alfredo-tui already owns conversation 'default' for this repo"**: another
  terminal owns that conversation set; close it or relaunch with
  `alfredo-tui --conversation other` (both terminals share the task queue; a terminal showing its side pane re-reads it every
  second, so the other terminal's `/go` tasks appear without a keypress).

### WSL notes

- Run Alfredo and your repositories inside the Linux filesystem (`~/...`), not
  `/mnt/c`; worktrees and checks are much slower on the Windows mount.
- If Ollama runs on Windows rather than inside WSL, `127.0.0.1` may not reach it
  unless WSL mirrored networking is on. Either run Ollama inside WSL, enable mirrored
  networking, or pass `--endpoint http://<windows-host-ip>:11434` with Ollama
  listening on that interface.
- bubblewrap works on WSL2 with the default kernel.

## Limitations

- Linux x86-64 only. No macOS or Windows-native build; release archives are built
  and tested on one glibc host (see `BUILD.json`).
- Local-model quality limits unattended runs. Plans can be wrong, checks can be weak,
  repairs can fail; expect to review, re-plan or finish some tasks by hand.
- Auto-accept trusts the approved check. If the check passes, the task is accepted;
  a weak check means weak acceptance. Risk-flagged and human-hold reviews still wait
  for you.
- Checks run in a sandbox with read-only system tools only; toolchains installed in
  your home directory (e.g. `~/.cargo`, `~/.nvm`) are not visible to checks.
- Work is based on committed HEAD; uncommitted changes are not seen by workers.
- Autopilot never pushes, never opens PRs and never moves your branch.

## Documentation

- [alfredo-tui/README.md](alfredo-tui/README.md): day-to-day use, agent view, build,
  test, package, source layout.
- [alfredo-tui/docs/reference.md](alfredo-tui/docs/reference.md): detailed behavior
  (tasks, review, repair, storage, scope, diagnostics).
- [CHANGELOG.md](CHANGELOG.md): what changed, following Keep a Changelog.
- [CONTRIBUTING.md](CONTRIBUTING.md): set up, test-first workflow, CI gates.
- [.agent/Tasks/STATUS.md](.agent/Tasks/STATUS.md): current project and release status.
- [CONTEXT.md](CONTEXT.md): domain vocabulary. [docs/](docs/README.md): index of the rest.

## Project status

Version 0.1.0, early. The terminal, autopilot, agent view and release packaging are
in place; a live end-to-end run on a real model and the first tagged release are still
open (see [STATUS](.agent/Tasks/STATUS.md)). Bug reports and reproductions are welcome
as [issues](https://github.com/EricleungDK/Alfredo/issues).

## License

MIT; see [LICENSE](LICENSE). Release archives include `THIRD_PARTY_NOTICES.txt` and
`DEPENDENCIES.json` for dependencies, which keep their own licenses.
