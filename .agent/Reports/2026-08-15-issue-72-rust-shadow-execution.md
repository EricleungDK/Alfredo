# Issue #72 — Shadow Rust execution receipts against Python

**Date:** 2026-08-15
**Status:** Implemented locally; Rust remains non-eligible pending external packaging and release gates
**Source:** [GitHub Issue #72](https://github.com/EricleungDK/Alfredo/issues/72)
**Parent:** [PRD #56](https://github.com/EricleungDK/Alfredo/issues/56)
**Depends on:** [Issue #71](https://github.com/EricleungDK/Alfredo/issues/71), [Issue #65](https://github.com/EricleungDK/Alfredo/issues/65)

## Outcome

Issue #72 now has a production-shaped Rust host-effects candidate behind the Issue #71 request/receipt boundary. Python remains the authorization, persistence, replay, reconciliation, and canonical-write authority. The candidate is invoked only by an explicit Python shadow adapter; it is not wired into the Mission coordinator or any product cutover path.

The Rust provider validates the versioned `ExecutionRequest`, requires a prepared Bubblewrap argv, launches with a cleared environment and bounded input, supervises a process group, captures bounded output, applies supported resource limits, and returns typed receipts for completion, failure, cancellation, timeout, output-limit, start failure, and outcome-unknown paths. The no-desktop `alfredo-execution-provider` binary exposes one JSONL request/response boundary for shadow samples.

## Python-owned shadow controls

- `CanonicalStoreHashGuard` hashes every declared canonical store before and after a sample. Any unapproved byte change disables the sample; approved observation records are explicit and remain visible in the result.
- `normalize_execution_receipt` and `compare_execution_receipts` remove provider-generated identity/timestamp noise while retaining request identity, effect, status, exit outcome, output bytes/digests, reconciliation state, and structured error fields.
- `ShadowCohortDefinition` admits only production-equivalent fixture/stage evidence and rejects reducer, sidecar, and microbenchmark evidence. Fixture, source, artifact, cohort, and stage identities are bound to each sample.
- `RustShadowProvider` treats malformed output, process crashes, start failures, timeout, invalid typed receipts, and structured provider failures as non-authoritative outcomes.
- `RustEligibilityStore` persists a separate, locked, atomic, fail-closed decision. Contract parity, canonical-store integrity, crash-cut, state-version, packaging, release-gate, production-equivalent cohort, and complete stage evidence are all required before eligibility can become true.

## Acceptance evidence

| Issue requirement | Evidence | Result |
|---|---|---|
| Shared contract behavior | Rust request-digest fixture, typed validation/start/status tests, process-level completion/timeout/output/cancellation tests, and Python Issue #71 replay/crash tests | Passes on the local host; Linux-only resource behavior still needs Linux evidence |
| Canonical store integrity | Python hash guard and mutation/approved-observation tests | Passes |
| Normalized receipt and structured failure parity | Actual Rust JSONL binary round-trip against the Python fixture, normalized projections, crash-to-unknown and structured-failure tests | Passes |
| Production-equivalent shadow cohorts | Cohort identity/stage/source/artifact checks; forbidden reducer/sidecar/microbenchmark tests | Passes |
| Eligibility and fallback | All-gates-required and crash-cut/state-version/packaging/release fail-closed persistence tests | Passes; no cutover is enabled by this implementation |

No Rust reducer, sidecar, or microbenchmark result is presented as a product speed claim.

## Verification

Passed:

- `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check`
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features execution -- --nocapture` — 8 passed
- `cargo build --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features --bin alfredo-execution-provider`
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_execution tests.test_execution_integration tests.test_execution_shadow -v` — 23 passed
- `git diff --check`

The actual JSONL parity test uses a fixture-local executable named `bwrap` on macOS because the host does not provide Bubblewrap. The provider's Linux `RLIMIT_AS` path is retained for Linux; Darwin rejects that primitive, so macOS deliberately does not claim that resource gate.

## Files

- `albert_mvp/execution_shadow.py` — Python shadow runner, store guard, parity projection, provider adapter, cohort and eligibility gates.
- `tests/test_execution_shadow.py` — Python shadow, parity, crash, structured-failure, store-integrity, cohort, and eligibility tests.
- `mission-control/src-tauri/src/execution.rs` — Rust request/receipt model, provider, supervision, cancellation, output/resource bounds, and tests.
- `mission-control/src-tauri/src/bin/alfredo-execution-provider.rs` — no-desktop JSONL provider process.
- `mission-control/src-tauri/Cargo.toml` and `Cargo.lock` — direct `libc` and `sha2` dependencies.

## Remaining gates and rollback

Rust eligibility must remain disabled until the production-equivalent packaged binary is exercised on the supported Linux/Bubblewrap environment, crash-cut and state-version evidence is collected there, and the release gate accepts the packaged artifact. Any parity, store-integrity, crash, protocol, package, or release failure records a disabled reason and leaves Python authority unchanged.

Related architecture and protocol guidance is recorded in [project architecture](../System/project_architecture.md), [API/command boundaries](../System/api_endpoints.md), and the living [agent context](../Tasks/context.md).
