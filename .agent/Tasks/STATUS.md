# Current status (2026-09-28)

- **Product**: `alfredo-tui/` — native Rust ratatui terminal orchestrating local Ollama
  coding agents (autopilot `/go`). Python `albert_mvp/` and Tauri/React
  `mission-control/` are legacy (`docs/legacy.md`). `context.md` is a historical log.
- **Branch**: `feat/rust-tui` holds the product (`main` does not yet). Launch docs/CI
  work: `agent/launch`. Version 0.1.0 (`alfredo-tui/Cargo.toml`, `CHANGELOG.md`).
- **Build**: Rust 1.96.0, `cargo build --release --locked --manifest-path alfredo-tui/Cargo.toml`.
- **Test**: see `alfredo-tui/README.md#test` (fmt, clippy, `cargo test --locked`, 5 PTY
  smokes, notice unittest, `cargo audit`). CI: `.github/workflows/rust-terminal.yml`;
  tag `v*` -> `release.yml` draft release.
- **Package**: `python3 alfredo-tui/scripts/package_release.py --output NEWDIR` then
  `python3 alfredo-tui/tests/release_smoke.py NEWDIR/*.tar.gz`.

## Launch acceptance ([launch-acceptance.md](launch-acceptance.md))

| Item | State |
| --- | --- |
| A1-A3 zero-ceremony start | Done (in-repo auto-open; `--select`; flags kept) |
| B1-B6 autopilot loop | Done; pause key is **F5** (spec said `p`), `/stop` cancels |
| C1-C5 dashboard UI | Open: UI work in progress (separate agent) |
| D1-D4 connection/speed | Done (health header, preload + keep_alive, retry, restart recovery) |
| E1 fmt/clippy/test | Done 2026-09-28: fmt, clippy, 439 tests pass (11 ignored live) |
| E2 PTY smokes on release binary | Done: 5 smokes incl. autopilot pass on installed archive |
| E3 live 2-task goal, real model | Open: not verified in this pass |
| E4 legacy gates | Run; 1 legacy defect (Python retirement /proc scan, 51 fails), not TUI; see `.agent/Reports/2026-09-28-launch-regression.md` |
| F1 README | Done (root README leads with TUI) |
| F2 CHANGELOG/LICENSE/notices/CI | Local done; CI green on GitHub not yet observed (not pushed) |
| F3 archive + draft release | Archive + installed smoke done locally; draft release needs tag push by owner |

## Open decisions for owner
- Merge `feat/rust-tui` -> `main` (root README clone instructions assume it).
- Push tag `v0.1.0` to create the draft release; publish manually after review.
