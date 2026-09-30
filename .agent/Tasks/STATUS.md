# Current status (2026-09-30)

- **Product**: `alfredo-tui/` — native Rust ratatui terminal orchestrating local Ollama
  coding agents (autopilot `/go`). The earlier desktop app and Python orchestrator were removed. `context.md` is a
  historical log.
- **Branch**: the product is on `main` (`feat/rust-tui` merged; side pane and worker
  context merged as PR #98, #99; success-rate fixes #100). Version 0.1.0 tagged
  `v0.1.0` at `8226708` (after release fix #104); draft GitHub release created, not
  published. Later changes are under `[Unreleased]` in `CHANGELOG.md`.
- **Build**: Rust 1.96.0, `cargo build --release --locked --manifest-path alfredo-tui/Cargo.toml`.
- **Test**: see `alfredo-tui/README.md#test` (fmt, clippy, `cargo test --locked`, 6 PTY
  smokes, Python unittests incl. `test_docs.py` doc-drift guards, `cargo audit`). CI:
  `.github/workflows/rust-terminal.yml`; tag `v*` -> `release.yml` draft release.
- **Package**: `python3 alfredo-tui/scripts/package_release.py --output NEWDIR` then
  `python3 alfredo-tui/tests/release_smoke.py NEWDIR/*.tar.gz`.
- **Docs map**: [root README](../../README.md) (users) ·
  [CONTRIBUTING](../../CONTRIBUTING.md) (developers) ·
  [alfredo-tui/README.md](../../alfredo-tui/README.md) (daily use, build, layout) ·
  [alfredo-tui/docs/reference.md](../../alfredo-tui/docs/reference.md) (detailed behavior).

## Launch acceptance ([launch-acceptance.md](launch-acceptance.md))

| Item | State |
| --- | --- |
| A1-A3 zero-ceremony start | Done (in-repo auto-open; `--select`; flags kept) |
| B1-B6 autopilot loop | Done; pause key is **F5** (spec said `p`), `/stop` cancels |
| C1-C4 dashboard UI | Shipped: side pane (missions, work tree), agent view, grouped F1 help, receipt IDs kept to F3/F4 ([tui-side-pane.md](tui-side-pane.md)); layout tests in `alfredo-tui/tests/side_pane.rs`, `dashboard.rs` |
| C5 input latency < 50 ms | Not re-measured since the side pane landed |
| D1-D4 connection/speed | Done (health header, preload + keep_alive, retry, restart recovery) |
| E1 fmt/clippy/test | Done 2026-09-28 (439 tests, 11 ignored live); rerun in CI on each push |
| E2 PTY smokes on release binary | Done 2026-09-28 (5 smokes on installed archive); the agent-view smoke was added since |
| E3 live 2-task goal, real model | Open: not verified |
| F1 README | Done (root README leads with TUI) |
| F2 CHANGELOG/LICENSE/notices/CI | Done locally; `main` is pushed, CI run result not recorded here |
| F3 archive + draft release | Done 2026-09-30: `release.yml` run 36746677600 built the archive, passed the installed smoke, created the draft; owner publishes |

## Open decisions for owner
- Publish the `v0.1.0` draft release after review.
- Refresh `docs/assets/demo.gif` and the two screenshots when the UI changes (recorded 2026-09-29 from a real run).

## Open follow-ups (2026-09-30)
- UI follow-ups from #99 (Manual tasks grouping, steered output, fences, agent
  drafts, adopt during integration, other-mission counts): fixed on
  `feat/ui-followups`, PR pending merge.
- Unexplained one-off instant chat failure after warm-up (2026-09-28): left open by
  owner decision (not reproduced).
