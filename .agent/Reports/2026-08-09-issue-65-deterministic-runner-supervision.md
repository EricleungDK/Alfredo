# Issue #65 — Deterministic Runner Supervision

**Date:** 2026-08-09
**Status:** Implemented, verified, and independently reviewed
**Authoritative requirement:** [GitHub Issue #65](https://github.com/EricleungDK/Alfredo/issues/65)

## Overview

Issue #65 replaces startup-only fixed-count abandoned-runner requeue with a durable, token-free supervision protocol. Python remains the only canonical authority. Advisory observations cannot mutate Mission state directly; an actionable observation first commits its Local Agent Attention Record, pending intent, semantic receipt, and observer cursor together, after which the Orchestrator independently proves every recovery boundary.

## Implementation

- `albert_mvp/core.py` persists observer cursors, attentions, intents, receipts, session revisions, exact process identities, runner-operation ids, Worktree Identities, recovery count, and typed result candidates.
- Healthy exact observations return `no-change` without a model turn, Mission/session mutation, attention, timeline entry, or Agent Console message.
- Exact runner and process-group absence with no result may queue one fresh runner in the same logical session and worktree. A valid late result is reconciled instead of rerun.
- Reused identities, unavailable probes (including an unobservable process-token scan), contradictory evidence, malformed state, revision/operation/worktree drift, or a failed single recovery stop automation and create one receipt-bound Mission Commander decision.
- `runner-observe` exposes the same contract through the one-process CLI and persistent JSONL transport. Workspace snapshots project receipt/outcome/recovery count; Agent Console reconciles only actionable receipts, while Mission Work links decision attention to the exact session and offers the existing typed manual Retry action with a required reason after the one automatic recovery fails.

## Verification

Focused red/green coverage is in `tests/test_runner_supervision.py`, with existing restart behavior corrected in `tests/test_albert_mvp.py`. It covers healthy runtime and startup silence, durable delivery before cursor acknowledgement, crash recovery, semantic replay across restart and observer source, exact one-runner recovery, late-result reconciliation, reused/unavailable/contradictory/mismatched fail-closed paths, persistent transport parity, real runner boundary persistence, and failed-recovery escalation. Frontend regressions cover the session receipt/budget inspector and Mission Work decision projection.

Verification evidence:

- Product-focused Python supervision/workspace suite: 19/19 passed, including the 16 new supervision cases and the corrected legacy restart contract.
- Python compilation and diff whitespace checks: passed.
- Canonical Apple-container App/projection suite: 173/173 passed; TypeScript typecheck and production Vite build passed.
- Localhost gateway: 23/23 passed; production four-viewport Playwright layout gate: 4/4 passed after restarting `alfredo-dev`.
- Cargo formatting and documentation standards validation passed; documentation remains Grade A (98.5%).

The requested broad runs were also executed once. The host Python discovery run completed 496 tests (8 skipped) but cannot be a green release gate on macOS because trusted Bubblewrap is unavailable and historical fixtures distinguish `/var` from `/private/var`. The Apple guest frontend discovery ran 313 tests with 288 passing; its remaining 25 are the documented persistent launch-environment, nested-Bubblewrap, and unsupported Linux-arm64 release-fixture constraints. None is in the Issue #65 supervision seam; the focused canonical suites above are green.

## Boundaries and Follow-up

The recovery is a new runner in the same logical session/worktree; it does not claim exact-token model continuation. Retirement, storage, Rust host-effect authority, and local-inference scheduling remain downstream Issue Slices. The implementation itself did not push, create a PR, release, or publish anything. In a separately authorized follow-up, Issue #65 received its verification evidence and was closed as completed; the local commit remains unpushed.

## Independent Review Corrections

The parallel Standards/Spec review and rechecks found and closed every hard/spec issue. The corrected implementation treats an unobservable process-token scan as unavailable rather than absent; rejects malformed revision, recovery-count, result, receipt, attention, intent, and observer authority; proves typed-result Mission/session/operation/worktree/digest identity and ledger Mission/session/effect consistency; links direct and recheck-blocked decision receipts to the affected session; removes residual Workspace Queue and approval semantics; and turns terminal failed-recovery attention into a typed manual Retry-with-reason decision proven through the production action service. Final rechecks report no Spec findings and no hard Standards findings. The Standards reviewer retains only a non-blocking Divergent Change judgement about the size of the linear supervision handlers; their lock-contained causal ordering is kept for this slice, with effect-specific handler extraction left as a future deepening opportunity.

## Traceability

- Architecture: [project_architecture.md](../System/project_architecture.md)
- Persistence: [database_schema.md](../System/database_schema.md)
- Command contract: [api_endpoints.md](../System/api_endpoints.md)
- UI semantics: [ux_guidelines.md](../System/ux_guidelines.md)
- Active orchestration: [context.md](../Tasks/context.md)
