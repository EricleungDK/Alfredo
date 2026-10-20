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
- **Eval**: `alfredo-tui/eval/` (issue #176) runs 6 fixed tasks through Alfredo (fixture or
  local Ollama) or Claude Code and emits schema-valid metrics JSON (M1-M16); fixture mode
  is smoked in CI by `tests/eval_smoke.py`. See `alfredo-tui/eval/README.md`. `--probes` (issue #190, `eval/probes.py`) fills M8/M9/M10/M14.
- **Headless** (issue #183): `--go GOAL --headless` (`src/headless.rs`) drives autopilot without a TTY: stderr progress lines, one JSON result on stdout, exit 0/1/2; smoked by `tests/headless_smoke.py`. Follow-ups: migrate `eval/run_eval.py`, `--doctor --json`, goal from stdin, headless `--resume`.
- **Compaction** (issue #187, owner decision: summarise, not trim): near 96 KiB (less with `--num-ctx`) the oldest chat turns are summarised by the conversation's model into a `compaction` note stored in place of them (`src/compaction.rs`, child module of `model`); `/compact` and `/clear`; smoked by `tests/compaction_terminal_smoke.py`. Reference: `docs/reference.md#long-conversations-compaction-and-clear`.
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

## Security hardening (2026-10-19)

- Worker sandbox also unshares IPC, UTS and cgroup namespaces (`SANDBOX_ISOLATION_FLAGS`); eval grading runs `check.sh` under bwrap and needs bubblewrap (`--unsafe-unsandboxed-grading` opts out); host git is hardened against repo-local fsmonitor, hooks and filter drivers, and `/go` start reports uncommitted changes as unchecked when local filters or includes are configured.
