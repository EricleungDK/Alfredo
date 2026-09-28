# Changelog

All notable changes to `alfredo-tui` are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-09-28

First release of the native terminal. It replaces the React/Tauri desktop app and
Python orchestrator as the primary product; those remain in the repository as
legacy (see `docs/legacy.md`).

### Added

- Native Rust (ratatui) terminal, `alfredo-tui`, for Linux x86-64.
- Zero-typing startup: inside a Git repository it opens the repository root with
  mission `default`. `--select`, `--workspace`, `--mission` and `--new-mission`
  choose explicitly; `/workspace` switches without restarting.
- Concurrent streaming Ollama conversations (up to 8) with cancel (Esc), retry
  (Ctrl+R), prompt history and restart-safe transcripts and drafts.
- Server health in the header (polled `/api/ps`), model preload and `keep_alive`
  (`--keep-alive`), bounded automatic reconnect before first content
  (`--connect-retries`), recovery after an Ollama restart.
- Shared per-endpoint inference capacity across terminals (`--parallel-models`),
  with foreground priority over background workers.
- Durable task queue with dependencies, explicit file/check policy (`/permit`),
  approval, receipts, activity view (F4) and Mission Work tree (F2).
- Planner (`/plan`, `/plan-revise`, `/plan-save`) using bounded committed repository
  context, and per-task model assignment (`/assign`).
- Coding workers in isolated Git worktrees; approved checks run in a bubblewrap
  sandbox with no network; verified evidence (F3, `/evidence`), criterion review,
  linked repairs, candidate commits, dependency composition and local review
  branches (`/branch`).
- Autopilot: `/go GOAL` or `--go GOAL` plans, approves and dispatches, auto-accepts
  tasks whose approved check passes, repairs failures up to `--max-repairs`
  (default 2), and composes accepted work onto one local `alfredo/go-<id>` branch.
  F5, `/pause`, `/resume`, `/stop`, `/autopilot`. Restores paused after restart;
  never pushes or moves your branch.
- `--doctor` preflight for storage, model catalog, repository and worker tools.
- Opt-in inference diagnostics (`--qualify-inference`, `--inspect-qualification`).
- Reproducible release archive with `BUILD.json` provenance, `DEPENDENCIES.json`,
  `THIRD_PARTY_NOTICES.txt`, MIT `LICENSE`, and an installed-binary smoke.
- CI for fmt, clippy, tests, PTY smokes, dependency audit and packaging; tag-driven
  draft release workflow.

### Known limitations

- Linux x86-64 only; tested on a single glibc host.
- Unattended quality depends on the local model; auto-accept trusts the approved
  check.
- Checks see only read-only system tools; toolchains in the home directory are not
  available inside the sandbox.

[Unreleased]: https://github.com/EricleungDK/Alfredo/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/EricleungDK/Alfredo/releases/tag/v0.1.0
