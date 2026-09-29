# Current status (2026-09-29)

- **Product**: `alfredo-tui/` — native Rust ratatui terminal orchestrating local Ollama
  coding agents (autopilot `/go`). Python `albert_mvp/` and Tauri/React
  `mission-control/` are legacy (`docs/legacy.md`). `context.md` is a historical log.
- **Branch**: the product is on `main` (`feat/rust-tui` merged; side pane and worker
  context merged as PR #98, #99). Version 0.1.0 (`alfredo-tui/Cargo.toml`); everything
  since is under `[Unreleased]` in `CHANGELOG.md`. No `v*` tag exists yet.
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
| E4 legacy gates | Run 2026-09-28; 1 legacy defect (Python retirement /proc scan, 51 fails), not TUI; see `.agent/Reports/2026-09-28-launch-regression.md` |
| F1 README | Done (root README leads with TUI) |
| F2 CHANGELOG/LICENSE/notices/CI | Done locally; `main` is pushed, CI run result not recorded here |
| F3 archive + draft release | Archive + installed smoke done locally; draft release needs the `v0.1.0` tag push by owner |

## Open decisions for owner
- Push tag `v0.1.0` to create the draft release; publish manually after review.
- Move `[Unreleased]` in `CHANGELOG.md` under `0.1.0` (or `0.2.0`) when tagging.
- Refresh `docs/assets/demo.gif` and the two screenshots when the UI changes (recorded 2026-09-29 from a real run).

## Open follow-ups (2026-09-29)
- UI (#99): follow-ups grouped under Manual tasks; steered runs keep no partial
  output; code fences raw in agent view; agent-view draft lost on quit; follow-up
  adopted during integration may be missed; other missions show state not done/total.
- Unexplained one-off instant chat failure after warm-up (2026-09-28).
- Legacy Python suite /proc retirement failures: owner decision pending.
