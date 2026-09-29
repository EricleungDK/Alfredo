# Changelog

All notable changes to `alfredo-tui` are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Coding workers now answer with plain-text FILE blocks
  (`=== FILE: path ===` … `=== END FILE ===`) instead of schema-constrained JSON,
  so code is written verbatim with no quote/newline escaping. The worker request
  sends no schema; `think: false` and repair sampling are kept. A reply cut off
  inside a block fails with `Model output ended inside FILE block for PATH
  (truncated)`, and the next repair says so. Legacy JSON answers are still
  accepted. `--worker-format json` restores the constrained JSON request;
  evidence records the requested format. Qualification pins `json`.
- Repair prompts show prior evidence as plain text (patch and check output
  unescaped), a fresh repair gets the previous attempt's files as FILE blocks,
  and retained legacy JSON answers are replayed as FILE blocks.
- The running task pane shows streamed FILE blocks as code: `▸ path` headings,
  marker lines hidden.

- Autopilot repair budget default is now 3 (`--max-repairs` still overrides).
- Repair prompts open with a short "What is still failing" section: failing test
  names, assertion/error lines and `-`/`+` diff lines (max 30 lines / 2 KiB),
  before the full prior evidence.
- Repair sampling temperature rises within a lineage (0 → 0.3 → 0.6, cap 0.8)
  after two or more failed attempts, one step more after no progress. Evidence
  records the requested temperature.
- A reply cut off by the token limit now fails with `Model output hit the
  4096-token limit`; the next repair requests 8192 tokens.

### Fixed

- A failed repair that returns byte-identical files is recorded as `No change from
  previous attempt`, and the next repair is told so explicitly and starts a fresh
  Local Agent conversation instead of replaying the repeated answer.
- Work status row pluralizes counts (`2 repairs`, `2 recorded runs`).

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
