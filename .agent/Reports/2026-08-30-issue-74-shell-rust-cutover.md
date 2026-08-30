# Issue #74 — Shell Rust host-effect cutover

**Implemented:** 2026-08-30

**Issue:** [GitHub #74](https://github.com/EricleungDK/Alfredo/issues/74)

**Parent PRD:** [GitHub #56](https://github.com/EricleungDK/Alfredo/issues/56)

**Baseline:** `83aceb6` on `main`

## Outcome

Governed Shell Terminal effects can now use the qualified, integrity-bound packaged Rust execution provider. Python remains the sole authority for command classification, Frontier or Mission Commander approval, Additional Path Grants, canonical path and environment preparation, the Bubblewrap/resource boundary, durable request claim, exact correlation replay, Shell Terminal and audit state, and uncertainty reconciliation. Rust receives only the already-authorized `ExecutionRequest` and performs the prepared external process effect.

The cutover has an independent `ALFREDO_RUST_SHELL_ENABLED` switch under `ALFREDO_RUST_CANDIDATE_ENABLED`. Packaged desktop launches enable both with the exact provider path and SHA-256. Setting either switch to `0` selects the packaged Python provider before request validation or journal claim. Setting the global candidate plus both effect switches to `0` also makes desktop launch independent of the Rust artifact, so a damaged candidate cannot block the all-Python rollback.

## Safety contract

- `ExecutionCoordinator` records the selected provider with the exact request and preserves it through executing, completed, cancelled, start-failed, and outcome-unknown receipts. Exact terminal replay returns the original receipt without invoking either provider; reuse of the correlation for a changed command boundary fails closed.
- The shared JSONL transport streams the Rust effect child's PID and start identity before completion. Python binds that child—not the adapter process—to the per-Mission execution journal.
- Cancellation is polled while the Rust effect is live and signalled to the provider, which supervises and cleans up the process group before returning the canonical cancellation receipt.
- Provider crash, timeout, malformed or duplicate streaming events, invalid terminal evidence, or a disagreement between the bound child and receipt becomes provider-preserving `outcome-unknown`. It never retries through Python and retains Mission Commander reconciliation.
- Rust selection rechecks the exact regular, executable, non-symlink provider and its SHA-256, then repeats Python's prepared Bubblewrap, mount, sanitized-environment, resource, and argv validation before the durable claim. Invalid selected-Rust flags or artifacts fail without an external effect or automatic fallback.
- The immediately previous one-response JSON provider remains readable. Existing providerless receipts continue to decode as Python receipts; no journal, Shell state, audit, or correlation identity is converted during cutover or rollback.

## Public transports and package boundary

The one-process CLI, persistent JSONL server, Tauri commands, `WorkspaceClient`, and React Shell Terminal keep their existing typed request and result shapes. A public-seam regression submits through the CLI, replays through the persistent transport, and rejects a changed request while the Rust provider runs exactly once. Existing Tauri client coverage transports the same correlation unchanged, and React's lost-response path deliberately retries that exact correlation before reloading canonical state.

The npm backend inventory includes the shared provider cutover adapter. The installed desktop adapter validates the provider only when at least one Rust effect is selected; enabled launches require exact package containment, regular non-symlink executable mode, and the `desktop.json` digest. Shell, Local Agent, and global flags remain independent, so Shell can return to Python without changing Local Agent state or rewriting stored receipts.

## Verification

- `python3 -m unittest -q tests.test_execution_cutover tests.test_execution tests.test_execution_shadow tests.test_execution_integration tests.test_local_agent_execution_cutover` — 73 passed with one optional previous-package fixture skipped, including real built-Rust Shell completion and cancellation.
- `npm test -- --run tests/build-npm-release.test.js tests/desktop-adapter.test.js` — 15 passed, including all-Python rollback with unusable Rust bytes and default-launch tamper rejection.
- `npm test -- --run src/workspace-client.test.ts src/App.test.tsx` — WorkspaceClient passed 45/45 and all Shell Terminal React cases passed. Two unrelated App cases timed out in the combined run and passed 2/2 in isolation.
- `npm run typecheck` — passed.
- `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` — passed.
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features execution -- --nocapture` — 14 passed.
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features` — 58/59 passed; the sole failure is the pre-existing retirement-review fixture, outside the execution cutover.
- Full Python discovery reached 781 tests with 29 failures, 30 errors, and 2 skips across pre-existing retirement/capability fixture drift plus subprocess copies that could not import the concurrent still-untracked cutover modules. The focused 73-test execution integration matrix is green, with one optional previous-package fixture skipped.

Issue #75 owns the integrated packaged-release verification of both cutovers and does not change the authority boundary described here.

## Principal files

- `albert_mvp/workspace.py` — Shell provider selection after Python policy and sandbox preparation.
- `albert_mvp/execution.py` — provider-neutral coordination, durable provider identity, replay, and reconciliation.
- `albert_mvp/execution_cutover.py` — integrity-bound Rust adapter and Shell-specific selection.
- `albert_mvp/execution_shadow.py` — bounded current/previous JSONL transport with live effect binding and cancellation.
- `mission-control/src-tauri/src/bin/alfredo-execution-provider.rs` — streamed process-binding events and live cancellation control.
- `mission-control/bin/desktop-adapter.js` — packaged provider verification and independent rollback flags.
- `tests/test_execution_cutover.py` — Shell public-seam selection, fallback, replay, conflict, uncertainty, transport, completion, and cancellation coverage.
