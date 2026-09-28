# Legacy regression run: launch check

- Date: 2026-09-28
- Commit: `d2d77cd` (branch `agent/launch`, clean tree)
- Scope: legacy Python `albert_mvp/` + `tests/`, Tauri/React `mission-control/`. The Rust terminal (`alfredo-tui/`) was not run here.
- Environment: WSL2 (kernel 6.6.114.1-microsoft-standard-WSL2), Ubuntu 24.04.3, systemd 255 (user manager running), Python 3.12.3, Node v24.11.0 / npm 11.12.1, rustc/cargo 1.96.0. No `rust-toolchain` file. The Apple-container SOP pins Rust 1.88.0, but that pin does not apply here, so 1.96.0 was used.
- Worktree sits on `/mnt/c` (NTFS through 9p), so file I/O is slow.
- `npm ci` ran first: exit 0, 57 s.
- Cargo target: `CARGO_TARGET_DIR=/home/ericl/.cache/alfredo-target/legacy`.
- Playwright 1.61.1 needs chromium v1228, and the global cache only has v1243. v1228 was installed to `PLAYWRIGHT_BROWSERS_PATH=/tmp/alfredo-playwright-browsers`, the documented method. No sudo or system deps were needed.

## Suites

| Suite | Command | Pass | Fail | Skip | Duration | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| Python unittest | `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests` | 735 | 50 (23 fail + 27 error) | 12 | 311 s | All 50 have one root cause (F1) |
| Python rerun, private PID ns | `cd tests && PYTHONPATH=.. unshare -c -p -f --mount-proc python3 -m unittest test_retirement_preservation test_workspace_snapshot test_albert_mvp -k repair -k retire -k reclam -k discard -k concurrent -k expired -k storage -k restart -k accepted -k cancellation` | 198 | 0 | 1 | 138 s | Covers all 50 failing tests. All pass once `systemd --user` is out of view |
| Vitest unit | `npx vitest run` (= `npm test -- --run`) | 324 | 1 | 0 | 673 s | `src/alfredo-release-seam.test.tsx` fails. Root cause F1 (backend retirement) |
| Vitest rerun, private PID ns | `unshare -c -p -f --mount-proc npx vitest run src/alfredo-release-seam.test.tsx` | 4 | 0 | 0 | 55 s | Passes, which confirms F1 |
| Vitest gateway | `npm run test:gateway` | 23 | 0 | 0 | 4 s | |
| Node performance | `npm run test:performance` | 30 | 0 | 0 | 2 s | |
| TypeScript | `npm run typecheck` | ok | 0 | – | 7 s | `tsc -b` exit 0 |
| Rust src-tauri | `cargo test` (in `mission-control/src-tauri`) | 77 | 0 | 1 ignored | 116 s wall, incl. build | lib 71, localhost-bridge 6 (1 ignored) |
| Rust fmt | `cargo fmt --check` | ok | 0 | – | <5 s | |
| Playwright layout | `npm run test:layout` | 4 | 0 | 0 | 16 s wall (5.5 s tests) | Includes prod build |
| Playwright localhost functional | `npm run test:browser` | 1 | 0 | 0 | 59 s | Real Vite bridge + Python authority |
| Playwright prototype journey | `npm run test:prototype-journey` | 4 | 0 | 0 | 29 s | |

Not run: `npm run release:verify` and `release:check`. They need AppImage packaging, a local registry and verified output, and are outside this regression scope.

## Failure triage

| ID | Failing tests | Symptom | Root cause | Class | Affects alfredo-tui? |
| --- | --- | --- | --- | --- | --- |
| F1 | Python: 22 in `test_retirement_preservation`, 10 in `test_albert_mvp` (repair/concurrent launch), 3 in `test_workspace_snapshot` (repair reload, retirement actions). Vitest: `alfredo-release-seam.test.tsx` (1) | `AlbertError: Open-handle inspection was unavailable before retirement: [Errno 13] Permission denied: '/proc/408/cwd' (Name: systemd ...)`. Downstream effects: `LaunchBlockedError`, `'retiring' != 'retired'`, storage budget exhausted, stale lifecycle revision | The Linux open-handle scan in `albert_mvp/core.py` (~L11729-11874) walks every same-uid `/proc/<pid>`. `systemd --user` (pid 408, uid 1000) holds `CapEff=CAP_WAKE_ALARM`, so the kernel denies `readlink /proc/408/cwd` to an unprivileged same-uid caller. The fallback `process_is_no_longer_same_live_owner` returns False because the process is live and same-uid, so retirement fails closed | Real legacy defect, triggered by the environment. It hits any Ubuntu 24.04 / systemd 255 host with a user manager, not only WSL. Every retirement, and every repair launch gated on retirement, is blocked | No. alfredo-tui has no procfs open-handle scan. Its `/proc` uses are bubblewrap `--proc /proc` mounts (`run_boundary.rs`, `worker.rs`) |

Confirmed by rerunning the same tests in a private PID namespace (`unshare -c -p -f --mount-proc`, same uid mapping): all pass.

## Fixes

None. F1 sits in fail-closed safety logic. The correct behavior (skip same-uid processes holding extra capabilities, or treat EACCES from a live process that cannot hold user files as non-blocking) is a design decision, not a cheap fix. No assertions were changed.

## Rust terminal gates (E1/E2/F3), same day, `6cecbd6`

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | ok |
| `cargo clippy --locked --all-targets -D warnings` | ok (16 s) |
| `cargo test --locked` | 439 pass, 0 fail, 11 ignored (live-model) (47 s) |
| PTY smokes on debug binary: terminal, autopilot, inference, recovery, qualification CLI | 5/5 pass |
| `python3 -m unittest discover -s alfredo-tui/tests -p 'test_*.py'` | 9 pass |
| `cargo audit --file alfredo-tui/Cargo.lock --deny warnings` (0.22.2, 1273 advisories, 213 crates) | clean |
| `package_release.py` + `release_smoke.py` (installed binary, 5 PTY smokes) | pass; clean source, sha256 `b5afbe9337002509f70d7dc1ec68bde9f656f571a893143582e74b8e44a88331` |
