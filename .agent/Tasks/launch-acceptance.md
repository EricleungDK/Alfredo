# Alfredo TUI launch acceptance

Owner: orchestrator verifies each item by running it, not by reading reports.
Scope rule: no new governance/receipt/schema slices unless an item below needs one.

## A. Zero-ceremony start
- A1 `alfredo-tui` inside a Git repo opens the main screen directly: workspace =
  repo root, mission `default` resumed or created. No typed input required.
- A2 Outside a repo, or with `--select`, the existing selector appears.
- A3 `--workspace/--mission/--new-mission` keep current behavior.

## B. Autonomous loop (ralph-style)
- B1 `/go GOAL` (and CLI `--go "GOAL"`) plans, saves, approves and dispatches
  tasks without further commands.
- B2 Passing check + complete evidence => task auto-accepted; dependents start.
- B3 Failed task => automatic repair, bounded (default 2); then task is held and
  the loop continues with independent tasks.
- B4 F5 (or `/pause`, `/resume`) pauses/resumes the loop; `/stop` cancels; state survives restart
  with the loop paused (never auto-replays).
- B5 On completion, accepted work lands on one local integration branch; user's
  HEAD/working tree untouched. Summary shows branch and per-task outcome.
- B6 Risk/human-hold reviews still stop for the user. Manual commands still work.

## C. Dashboard UI
- C1 Left: task list with status glyphs + progress count. Right: live output of
  selected agent/task. Header: loop state, done/total, elapsed, model, server.
- C2 Default view hides receipt IDs/revisions; detail view (F3) keeps them.
- C3 Correct at 80x24, 100x30, 140x40 and degrades without overlap at 40x12.
- C4 F1 help is grouped and short; common path first.
- C5 Input latency: keypress-to-redraw under 50 ms with 4 streaming workers.

## D. Connection and speed
- D1 Server health visible in header (up/down/loading model), polled.
- D2 Model preloaded at start and kept warm (`keep_alive`), configurable.
- D3 Connection failure before any content: automatic bounded retry with
  backoff, visible countdown. After content: partial kept, manual retry.
- D4 Server restart mid-session recovers without restarting the TUI.

## E. Regression
- E1 `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test --locked` pass.
- E2 PTY smokes pass on the release binary; new PTY journey covers A1 + B1-B5
  with a fake Ollama server.
- E3 Live journey with a real local model completes a 2-task goal end to end.
- E4 Legacy gates (Python, frontend unit, Playwright) run once; results recorded;
  failures triaged as legacy-only or fixed.

## F. Launch
- F1 Root README leads with the TUI: install, 60-second quickstart, keys.
- F2 CHANGELOG, LICENSE, third-party notices, CI workflow green on GitHub.
- F3 Release archive built + installed smoke; GitHub release drafted (publish
  only after user confirms).
