# Alfredo

Alfredo is a native terminal (`alfredo-tui`) that runs local Ollama coding agents
on your Git repository. Give it a goal; it plans tasks, runs each one in an isolated
worktree and sandbox, checks the result, repairs failures, and leaves one local
branch for you to review. Nothing leaves your machine and nothing is pushed.

## Requirements

- Linux x86-64 (WSL2 works; see [WSL notes](#wsl-notes)).
- Git at `/usr/bin/git`, and a repository with at least one commit.
- [Ollama](https://ollama.com) running locally, plus a model, e.g.
  `ollama pull qwen2.5-coder:14b`.
- For coding workers: bubblewrap (`/usr/bin/bwrap`) and prlimit (`/usr/bin/prlimit`,
  from util-linux). Debian/Ubuntu: `sudo apt install git bubblewrap util-linux`.
- To build from source: Rust 1.96 (`rustup toolchain install 1.96.0`).

Chat works without the worker tools; only coding tasks need them.

## Install

From source (needs the full checkout; the build includes
`mission-control/src-tauri/src/execution.rs`):

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
Manual control is always available (`/plan`, `/task`, `/approve`, `/run`,
`/evidence`, `/accept`, …); press F1 on an empty prompt for the command list.

## Keys

| Key | Action |
| --- | --- |
| Enter | Send prompt or command |
| Shift+Enter | New line (if the terminal reports it; paste also works) |
| F1 | Command picker (empty prompt); Tab after `/prefix` completes |
| F2 | Right pane: task detail / conversation |
| F3 | Evidence for the selected task (task detail) |
| F4 | Activity (saved task receipts) |
| F5 | Pause / resume autopilot |
| F6 | Focus the side pane (overlay below 88 columns); again or Esc returns to the prompt |
| Up / Down | Prompt history; in task detail, select task; in the side pane, move |
| Tab (side pane) | Switch between missions and work |
| Enter (side pane) | Open the row: a task's or the architect's agent view, group detail, chat, or switch mission |
| Enter (agent view) | Instruct that agent: steer, queued note, repair, follow-up or plan revision |
| Esc (agent view) | Back to the previous pane; the agent's unsent draft is kept |
| Ctrl+O (agent view) | Expand / collapse the full instruction text |
| Alt+Left / Alt+Right | Collapse / expand a task branch |
| PageUp / PageDown | Scroll transcript, agent view, details or evidence |
| Tab / Shift+Tab | Next / previous conversation |
| Ctrl+N | New conversation (max 8) |
| Esc | Cancel the current model request (keeps partial reply); close pickers |
| Ctrl+R | Retry a failed or cancelled turn |
| Ctrl+W / Ctrl+U | Delete word / clear prompt |
| Ctrl+Q / Ctrl+C | Quit (cancels workers and waits for their results) |

## Flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `--model NAME` | `qwen3:14b` (env `ALFREDO_MODEL`) | Model for new conversations |
| `--endpoint URL` | `http://127.0.0.1:11434` (env `OLLAMA_HOST`) | Ollama HTTP origin |
| `--go GOAL` | off | Start autopilot on launch |
| `--max-repairs N` | 3 | Auto-repairs per task, 0–16 (0 disables) |
| `--workspace DIR` | current repo | Open the repository containing `DIR` |
| `--mission NAME` / `--new-mission NAME` | `default` | Resume / create a named mission (with `--workspace`) |
| `--select` | off | Always show the repository/mission selector |
| `--conversation NAME` | `default` | Named conversation set (one terminal owns a set) |
| `--state-dir DIR` | `~/.local/state/alfredo` (env `ALFREDO_STATE_DIR`) | State location; must be outside the repo |
| `--keep-alive VALUE` | `30m` (env `ALFREDO_KEEP_ALIVE`) | Ollama `keep_alive`: `30m`, `300`, `-1`, or `default` |
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
  `/models` then `/model NAME` inside the terminal.
- **Doctor: `FAIL installed worker tool: /usr/bin/bwrap`** (or prlimit/git): install
  `bubblewrap` / `util-linux` / `git`. The paths are fixed.
- **Worker fails at sandbox start** (e.g. "Permission denied" setting up namespaces):
  your distro restricts unprivileged user namespaces (Ubuntu 24.04+ AppArmor). Test
  with `bwrap --ro-bind / / --unshare-user --unshare-pid --unshare-net true` and allow
  bwrap per your distro's policy.
- **"Interactive terminal required"**: run in a real terminal, not piped.
- **"Conversation namespace is in use"**: another terminal owns that conversation
  set; use `--conversation other` (both terminals share the task queue).

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

- [alfredo-tui/README.md](alfredo-tui/README.md) — build, test and package from source.
- [alfredo-tui/docs/reference.md](alfredo-tui/docs/reference.md) — detailed behavior
  (tasks, review, repair, storage, scope, diagnostics).
- [CHANGELOG.md](CHANGELOG.md)
- [.agent/Tasks/STATUS.md](.agent/Tasks/STATUS.md) — current project status.

## Legacy

This repository also contains an earlier desktop workstation (React/Tauri
`mission-control/` with a Python orchestrator `albert_mvp/`). It is no longer the
primary product. Its documentation is in [docs/legacy.md](docs/legacy.md).

## License

MIT; see [LICENSE](LICENSE). Release archives include `THIRD_PARTY_NOTICES.txt` and
`DEPENDENCIES.json` for dependencies, which keep their own licenses.
