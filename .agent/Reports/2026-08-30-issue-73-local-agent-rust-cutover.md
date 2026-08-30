# Issue #73 — Local Agent Rust host-effect cutover

**Implemented:** 2026-08-30

**Issue:** [GitHub #73](https://github.com/EricleungDK/Alfredo/issues/73)

**Parent PRD:** [GitHub #56](https://github.com/EricleungDK/Alfredo/issues/56)

**Baseline:** `83aceb6` on `main`

## Outcome

Authorized Local Agent host effects can now use the qualified, integrity-bound packaged Rust execution provider. Python still authenticates the Mission/session/revision/runner/worktree boundary, constructs and validates the exact sandboxed request, durably claims the request, owns canonical Mission and Local Agent session state, reconciles the typed receipt, and decides whether recovery is safe. Rust receives only the already-authorized `ExecutionRequest` and performs the external process effect.

The cutover has an independent `ALFREDO_RUST_LOCAL_AGENT_ENABLED` switch under the qualified-candidate gate. Packaged desktop and headless launches enable it with the exact provider path and SHA-256. Setting either Local Agent or candidate flag to `0` selects the packaged Python provider before request validation or journal claim, including when the optional native package is unavailable. Artifact preflight may also fall back only at that same pre-claim boundary, where no canonical write, external effect, or unresolved receipt exists.

## Safety contract

- `ExecutionCoordinator` durably records the selected provider and exact request before invoking it. Exact terminal or uncertain replay returns the original receipt without crossing providers.
- The shared JSONL transport streams the Rust effect child's PID and start identity before completion. Python binds that child—not the adapter process—to the executing journal and Local Agent supervision record.
- Cancellation is polled while the Rust effect is live and signalled to the provider, which performs identity-aware process-group cleanup and returns a typed cancellation receipt.
- Provider crash, timeout, malformed streaming events, invalid terminal evidence, or a disagreement between the bound child and receipt becomes Rust `outcome-unknown`. It is never automatically retried through Python and keeps Mission Commander reconciliation visible.
- Streaming negotiation uses an internal environment flag. The immediately previous packaged provider ignores it and retains its one-response JSON behavior, so current and previous provider transports remain readable.
- Existing providerless receipts decode as Python receipts without rewriting the journal. The schema-versioned request, receipt, and per-Mission journal remain Python-owned and are not destructively converted.

## Package boundary

The npm backend inventory now includes both the shared provider cutover adapter and the Local-Agent-specific selector. Installed desktop and headless launch paths verify the exact packaged provider as a contained, executable, non-symlink regular file whose SHA-256 matches `desktop.json`, then export the candidate, Shell, and Local Agent flags independently. A Local Agent rollback therefore does not silently change Shell routing. Issue #72's shadow eligibility store remains a qualification-time authority rather than a runtime dependency: published-package cutover trusts only the exact artifact admitted by the release gate, while local/source execution requires the explicit candidate flag and still verifies its exact path/hash before claim.

## Verification

- `python3 -m unittest -q tests.test_local_agent_execution_cutover tests.test_execution_cutover tests.test_execution tests.test_execution_integration tests.test_execution_shadow` — 81 ran, 80 passed, 1 packaged-prior-binary check skipped after the package fixture cleanup; the synthetic immediately previous transport remained covered, and the prior packaged binary passed earlier in the same run before cleanup.
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features execution -- --nocapture` — 14 passed, including real completion, cancellation, timeout, output, and process-cleanup behavior.
- `cargo build --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features --bin alfredo-execution-provider` — passed.
- `npm test -- --run tests/desktop-adapter.test.js` — 7 passed.
- `npm run typecheck` — passed.
- `npm run build` — passed.
- `npm run release:verify` — reached the optimized Rust/AppImage bundle step, then the environment's `linuxdeploy` invocation failed; no publishable package claim is made from this run.

The repository-wide suites retain existing lifecycle/UI debt outside this slice: Python ran 782 tests with 21 failures, 28 errors, and 3 skips; Vitest passed 296 of 298 tests with two UI timeouts plus one Node-environment setup failure; full Rust passed 58 of 59 tests with one retirement-review integration failure. The focused cutover matrices above remain green. Independent Standards/Spec review outcomes are recorded in the active orchestration context and final implementation handoff.

## Principal files

- `albert_mvp/core.py` — Local Agent provider selection at the existing governed effect seam.
- `albert_mvp/execution.py` — provider-neutral coordination, durable provider identity, replay, and reconciliation.
- `albert_mvp/execution_cutover.py` — integrity-bound shared Rust provider adapter.
- `albert_mvp/local_agent_execution_cutover.py` — Local-Agent-specific selection and proof-gated Python fallback.
- `albert_mvp/execution_shadow.py` — bounded current/previous JSONL transport with live child-binding and cancellation callbacks.
- `mission-control/src-tauri/src/bin/alfredo-execution-provider.rs` — streamed process-binding events and live cancellation control.
- `mission-control/bin/desktop-adapter.js` — exact packaged provider verification and effect-specific launch flags.
- `tests/test_local_agent_execution_cutover.py` — public-seam cutover, fallback, restart, crash, replay, compatibility, completion, and cancellation coverage.
