# Changelog

All notable changes to `alfredo-tui` are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Pasting into the prompt no longer deletes tabs (each becomes four spaces) and now
  shows `Paste truncated to 16 KiB` when the draft limit cuts the paste.
- In a repository with no commits, `/go` now stops at once (no planning retries) and
  says to make an initial commit; the launch footer and `--doctor` give the same fix.
  Git error text no longer glues lines together (`tree.Use`).

### Changed

- Side pane: other missions show `done/total` and their state (`4/7   running`).
- `execution.rs` moved into `alfredo-tui/src`; the crate builds without the rest of
  the repository.

### Removed

- Legacy desktop app (React/Tauri `mission-control/`) and Python orchestrator
  (`albert_mvp/`), with their docs, tests, scripts and the npm publish workflow.

### Fixed

- Model switching: picking another model now abandons the previous model's preload.
  Every model picked used to stay queued in Ollama and load in full, one after
  another, so a chat could wait minutes behind models nobody wanted.
- Chat waits two minutes for a model to load, separate from the sixty-second idle
  timeout. Expiry is final instead of retried: a retry closed the connection, which
  aborted the load and started it again.
- `/models`: Up/Down and Enter now pick a model; arrows used to fall through to
  prompt history, so only `/model NAME` could switch.
- Chat knew nothing of the harness: plain chat turns sent only conversation text, so
  "what did you remove?" got "I'm an AI language model". Turns now carry a bounded
  system message with Alfredo's role and the mission's tasks and verified patches.
- Shared inference: a chat whose `--parallel-models` differs from another live
  Alfredo process waits for it to drain instead of failing with "capacity conflict".
  No HTTP is sent and Esc cancels; a cat indicator shows the wait.
  `--qualify-inference` still refuses.
- Ollama `{"error":...}` frame before any output is retried like other connection
  failures; "not found" and errors after output stay final.
- Release workflow fetches crates before offline packaging.
- Work tree: `/after` follow-ups appear in their plan group, not "Manual tasks".
- Agent view: a steered run keeps its streamed partial output, shown with a dim
  `— steered at Ns · output cut` line; repairs never reuse the partial answer.
- Agent view and live output hide markdown code fence lines.
- Agent view: unsent drafts survive quit, crash and mission switch, per task and
  architect; the chat draft is never replaced by an agent's note.
- Autopilot: a follow-up adopted while the integration branch is built is no longer
  lost; the run resumes and integrates again on the next `-N` branch.

## [0.1.0] - 2026-09-30

First release of the native terminal. It replaces the earlier desktop app and orchestrator.

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

- Agent view: Enter on a task or the architect in the side pane (or `/watch
  ID|architect`) shows that agent's transcript in the right pane: instruction
  (Ctrl+O expands the full request), read-only references, the answer as code per
  file, the check with its output tail, the outcome, then each repair and your
  notes. Follows the tail while live; PageUp/PageDown keep the reading position.
- Owner instructions: while an agent view is open the prompt reads
  `To worker #N · Enter send · Esc back` and a note steers a generating worker
  (cancel, rerun as a repair outside the autopilot budget), is queued during its
  check (repair reason on failure, `Note not needed: check passed` on pass),
  repairs a failed/rejected/review-ready result, adds a follow-up to accepted work
  (same files and check), or revises the architect's draft. Held reviews refuse.
  `/tell ID|architect TEXT` from anywhere. Every step goes through the existing
  commands and receipts; notes lead the next worker request as `OWNER INSTRUCTION`.
- Autopilot holds a family while you instruct it and resumes afterwards; follow-ups
  join the run and integrate on `alfredo/go-ID-2`.

- Side pane on every view: **missions** of this repository (current first; others
  show their saved autopilot phase, `idle` or `?`, read without locks every 2 s)
  and **work**: the architect while planning or holding a draft, plan groups with
  tasks and repairs, then chats. Width is a quarter of the terminal (28–44
  columns); below 88 columns it is one summary row and F6 opens it as an overlay.
- F6 focuses the side pane: Up/Down, Tab (missions/work), Enter opens (task or
  group detail, plan draft, chat, or switch mission under the `/workspace` rules),
  Alt+Left/Right fold, Esc returns to the prompt with the draft kept.
- Record icons (`▤` task, `⑂` repair, `◈` agent) and status glyphs; a braille
  spinner (100 ms) for working rows, redrawn only while something works. Running
  rows get a dim second line: stage, model, elapsed.
- `--icons nerd|unicode|ascii` (env `ALFREDO_ICONS`), `--no-motion` (env
  `ALFREDO_NO_MOTION=1`); truecolor palette when `COLORTERM` is `truecolor`/`24bit`,
  16 colours otherwise, `NO_COLOR` respected.

### Changed

- Enter on a task row opens its agent view instead of the task detail (F2 and Esc
  still reach the detail).
- Autopilot panel: `✓ Autopilot done`, the goal on one line, `Tasks`, `Repairs`,
  `Branch`, one line per task, `Review` and `Merge`; the footer result reads
  `Autopilot done   1/1 accepted   git switch alfredo/go-…`.
- Chat: autopilot's consecutive steps for one task collapse to one line
  (`✓ #1 planned → approved → started → check passed → accepted`); detail stays in F4.

- Header is two rows at most: mission and repository name, attention items only
  when non-zero (`2 running`, `1 review`, `1 decision`, `dispatch on`), health with
  the short model name; the autopilot row has no goal and no ` · ` chains.
  `dispatch off`, `Work 0 local` and `no pending review` are gone.
- Task detail is labeled sections (`Files`, `Check`, `Depends`, `State`, `Next`,
  `Result`, then the diff) with hanging-indent wrapping; group detail is goal,
  progress and tasks. Key help, counts and the filter line left the detail panes;
  an active filter is named in the detail title.
- One column of padding inside panes; chat speaker labels are dim on their own line.
- Footer: one line of at most eight hints for the focused area.
- Completed plan groups collapse while another group has open work.

### Fixed

- Work group titles showed planner retry text (`… | The previous plan was rejected
  by validation: …`); the title is the user's goal, also for older saved plans.

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

- A failed repair that returns byte-identical files is recorded as `No change from
  previous attempt`, and the next repair is told so explicitly and starts a fresh
  Local Agent conversation instead of replaying the repeated answer.
- Work status row pluralizes counts (`2 repairs`, `2 recorded runs`).
- Plan lint rejects untracked runtime data files (e.g. `todo.json`, `app.sqlite`)
  in task files and tells the planner to use temp paths; a later check rewriting
  them was refused as modifying files outside approved paths.
- A worker answer that is one bare code fence is taken as the file when the task
  allows exactly one file (never when FILE blocks or JSON were attempted).
- Repairs of tasks with accepted dependencies get the dependencies' files as
  read-only references and are told the accepted implementation is authoritative.
- A contended scope lock is a retryable busy refusal, not a final denial;
  autopilot spends no attempt on it.

### Known limitations

- Linux x86-64 only; tested on a single glibc host.
- Unattended quality depends on the local model; auto-accept trusts the approved
  check.
- Checks see only read-only system tools; toolchains in the home directory are not
  available inside the sandbox.

[Unreleased]: https://github.com/EricleungDK/Alfredo/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/EricleungDK/Alfredo/releases/tag/v0.1.0
