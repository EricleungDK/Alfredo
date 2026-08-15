# Issue #72 — Shadow Rust execution receipts against Python

**Date:** 2026-08-15
**Status:** Implemented locally; Rust remains non-eligible pending external packaging and release gates
**Source:** [GitHub Issue #72](https://github.com/EricleungDK/Alfredo/issues/72)
**Parent:** [PRD #56](https://github.com/EricleungDK/Alfredo/issues/56)
**Depends on:** [Issue #71](https://github.com/EricleungDK/Alfredo/issues/71), [Issue #65](https://github.com/EricleungDK/Alfredo/issues/65)

## Outcome

Issue #72 now has a production-shaped Rust host-effects candidate behind the Issue #71 request/receipt boundary. Python remains the authorization, persistence, replay, reconciliation, and canonical-write authority. The candidate is invoked only by an explicit Python shadow adapter; it is not wired into the Mission coordinator or any product cutover path.

The Rust provider validates the versioned `ExecutionRequest`, requires a trusted prepared Bubblewrap argv with an exact namespace/mount/resource shape, launches with a cleared environment and bounded input, supervises a process group with identity-checked cleanup, captures bounded output, and returns typed receipts for completion, failure, cancellation, timeout, output-limit, start failure, and outcome-unknown paths. The no-desktop `alfredo-execution-provider` binary exposes one bounded JSONL request/response boundary for shadow samples.

## Python-owned shadow controls

- `CanonicalStoreHashGuard` hashes every declared canonical store before and after a sample. Any unapproved byte change disables the sample; approved observation records are explicit and remain visible in the result.
- `normalize_execution_receipt` and `compare_execution_receipts` remove provider-generated identity/timestamp noise while retaining request identity, effect, status, exit outcome, output bytes/digests, reconciliation state, visible projections, process outcomes, and structured error fields.
- `ShadowCohortDefinition` recomputes immutable fixture/source/artifact digests, admits only measured production stages, and rejects reducer, sidecar, and microbenchmark evidence. Every sample binds the cohort, fixture root, artifact command, and stage marks.
- `RustShadowProvider` uses an allowlisted environment and bounded stdout/stderr readers; malformed output, non-zero provider exits, process crashes, start failures, timeout, invalid typed receipts, and structured provider failures are non-authoritative outcomes.
- `RustEligibilityStore` persists a separate, locked, atomic, directory-durable, fail-closed decision. Contract parity, canonical-store integrity, crash-cut, state-version, packaging, release-gate, production-equivalent cohort, and complete stage evidence are all required; package/release gates additionally require recomputed packaged-artifact evidence.

## Acceptance evidence

| Issue requirement | Evidence | Result |
|---|---|---|
| Shared contract behavior | Rust request-digest fixture, strict schema/sandbox/resource validation, typed start/status tests, process-level completion/timeout/output/cancellation tests, JSONL contract-failure round-trip, and Python Issue #71 replay/crash tests | Passes on the local host; the real Bubblewrap cross-provider success sample and Linux-only resource behavior still need Linux evidence |
| Canonical store integrity | Python hash guard and mutation/approved-observation tests | Passes |
| Normalized receipt and structured failure parity | Normalized receipt, process-outcome, visible-projection, structured-failure projections, non-zero-exit crash cut, and JSONL contract-failure tests | Passes locally; real Bubblewrap success parity is unavailable on this host |
| Production-equivalent shadow cohorts | Recomputed fixture/source/provider digests, bound roots/commands, measured S/R stages, and forbidden reducer/sidecar/microbenchmark tests | Passes |
| Eligibility and fallback | All-gates-required, artifact-bound release evidence, crash-cut/state-version/store-integrity, atomic persistence, and malformed-state fail-closed tests | Passes; no cutover is enabled by this implementation |

No Rust reducer, sidecar, or microbenchmark result is presented as a product speed claim.

## Verification

Passed:

- `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check`
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features execution -- --nocapture` — 9 passed
- `cargo build --manifest-path mission-control/src-tauri/Cargo.toml --no-default-features --bin alfredo-execution-provider`
- `PYTHONDONTWRITEBYTECODE=1 uv run --with pytest python -m pytest -q tests/test_execution.py tests/test_execution_integration.py tests/test_execution_shadow.py` — 25 passed, 1 Bubblewrap-dependent sample skipped on Darwin
- `cargo check --manifest-path mission-control/src-tauri/Cargo.toml`
- `npm run typecheck` and `npm run build` in `mission-control/`
- `uv run --with ruff ruff check albert_mvp/execution_shadow.py tests/test_execution_shadow.py`
- `git diff --check`

The real JSONL success-parity test is skipped on this Darwin host because no trusted Bubblewrap executable is installed. Test-only Rust process fixtures likewise skip without trusted Bubblewrap; they do not admit fixture-local shims into the production validator. The provider's Linux resource path and packaged-artifact gate therefore remain unverified here, and macOS deliberately makes no Linux eligibility claim.

## Files

- `albert_mvp/execution_shadow.py` — Python shadow runner, store guard, parity projection, provider adapter, cohort and eligibility gates.
- `tests/test_execution_shadow.py` — Python shadow, parity, crash, structured-failure, store-integrity, cohort, and eligibility tests.
- `mission-control/src-tauri/src/execution.rs` — Rust request/receipt model, provider, supervision, cancellation, output/resource bounds, and tests.
- `mission-control/src-tauri/src/bin/alfredo-execution-provider.rs` — no-desktop JSONL provider process.
- `mission-control/src-tauri/Cargo.toml` and `Cargo.lock` — direct `libc` and `sha2` dependencies.

## Remaining gates and rollback

Rust eligibility must remain disabled until the production-equivalent packaged binary is exercised on the supported Linux/Bubblewrap environment, crash-cut and state-version evidence is collected there, and the release gate accepts the packaged artifact. Any parity, store-integrity, crash, protocol, package, or release failure records a disabled reason and leaves Python authority unchanged.

Related architecture and protocol guidance is recorded in [project architecture](../System/project_architecture.md), [API/command boundaries](../System/api_endpoints.md), and the living [agent context](../Tasks/context.md).
