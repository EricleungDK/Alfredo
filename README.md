# local-coding-agent - Albert MVP

**Last updated:** 2026-06-20

Albert is a tested local coding-agent orchestrator MVP. It reads local markdown PRDs and Issue Slices, renders a textual mission-control TUI, routes approved work through Qwen, launches local model agents in isolated worktrees, validates evidence, supports repair relaunch, and prepares PR-ready summaries without auto-merging.

## Start Here

- [Architecture and design](docs/albert-architecture.md)
- [MVP mapping and status](docs/albert-mvp-status.md)
- [Usage guide](docs/albert-usage.md)
- [Documentation index](docs/README.md)

## Quick Start

```bash
ollama list
python3 -m unittest discover -s tests
python3 -m albert_mvp tui \
  --target-repo . \
  --tracker-dir .scratch/local-coding-agent-mvp-development \
  --runtime-root /tmp/albert-runtime \
  --mission-id local-coding-agent-mvp-development \
  --agent-config .albert/agents.json
```

## Start the Command Deck

Prerequisites: Python 3, Node.js/npm, Rust/Cargo, Ollama, and the [Tauri 2 Linux system dependencies](https://v2.tauri.app/start/prerequisites/) for your distribution.

```bash
git clone <repository-url> local-coding-agent
cd local-coding-agent
ollama list
python3 -m unittest discover -s tests -v
cd mission-control
npm install
npm run desktop
```

The desktop shell starts the local Python Orchestrator through the Tauri bridge. Set `ALBERT_PYTHON` if Python is not available as `python3`, or `ALBERT_BACKEND_ROOT` when launching outside the repository layout.

## Current Model Registry

- `qwen3.6-27b` -> `ollama:qwen3.6:27b`, frontier/router
- `gemma4-12b` -> `ollama:gemma4:12b`, normal local worker
- `gemma4-26b` -> `ollama:gemma4:26b`, normal local worker
- `qwen2.5-coder-14b` -> `ollama:qwen2.5-coder:14b`, delegate-only coder
- `deepseek-r1-14b` -> `ollama:deepseek-r1:14b`, delegate-only reasoning/architecture worker

Qwen chooses when approved work stays on Gemma or escalates to a delegate-only model.

## Status

- Both local tracker backlogs are complete under `.agent/issues/`.
- Issue 01's integration gates passed on 2026-06-20: 72 Python tests, 7 frontend tests, typecheck, production build, and 6 Rust desktop bridge tests.
- Qwen3.6-27B, Gemma4-12B, and Gemma4-26B have live prototype or repair-loop verification.
- Local Qwen2.5-Coder 14B and DeepSeek-R1 14B replaced subscription-gated Ollama Cloud delegates.

## Common Commands

Use `python3 -m albert_mvp <command>` with `--target-repo`, `--tracker-dir`, `--runtime-root`, `--mission-id`, and `--agent-config`.

- `agents` lists configured model agents.
- `tui` renders mission control.
- `show ISSUE_ID` inspects one slice.
- `approve ISSUE_ID` locks a slice for launch.
- `route ISSUE_ID` asks Qwen to choose a worker or delegate.
- `launch ISSUE_ID` starts an approved unblocked session.
- `review SESSION_ID --outcome OUTCOME --reason REASON` records Frontier review.
- `pr ISSUE_ID` prepares PR-ready instructions.
