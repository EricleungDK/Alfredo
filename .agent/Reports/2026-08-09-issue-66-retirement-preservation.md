# Issue #66 — Retirement Unit preservation proof

**Date:** 2026-08-09
**Status:** Implemented, verified, and independently reviewed locally
**Issue:** [GitHub #66 — Reserve and prove preservation for every Retirement Unit](https://github.com/EricleungDK/Alfredo/issues/66)

## Overview

Issue #66 establishes the preservation authority required before any later worktree deletion can be considered. Each worktree-owning Local Agent session now begins with a durable, bound Preservation Budget and independent Retirement Unit state. Preservation claims the exact session revision, proves Worktree Identity and Runner Quiescence, captures an app-local Retirement Snapshot, and verifies it through manifest readback, per-file hashes, and disposable clean-room reconstruction.

This implementation deliberately stops before physical retirement. Terminal-outcome automation, grace periods, durable retirement retry phases, exact removal, aggregate storage retention/reclamation, pinning, and Retained Worktree Discard remain Issues #67 and #68.

## Implemented contract

- Session creation reserves a fixed 32 MiB preservation allowance before execution; the reservation remains bound after a failed proof and becomes verified/unbound only after successful snapshot publication.
- The existing monotonic session revision is the one lifecycle lock. Execution, cancellation, review, repair launch, and preservation reject stale revisions or a non-active Retirement Unit; preservation persists `preserving` before filesystem effects.
- Worktree Identity now requires agreement among the stored path, deterministic managed path, canonical filesystem path, `.git` administrative back-pointer, and the exact worktree listed by the expected repository. The Coding Workspace remains ineligible.
- Runner Quiescence requires independent absence of the exact per-operation supervising-runner lease and spawned process group. A terminal label does not substitute for either result; process identity/token ambiguity remains fail-closed.
- Git snapshots retain a self-contained baseline bundle, the exact baseline commit and metadata, staged and unstaged binary patches, exact porcelain-v2 state digest, untracked non-ignored files, and all registered session artifacts. Unregistered directories are not retirement-eligible.
- The manifest records Mission/session/revision authority, terminal state, Worktree Identity, baseline metadata and digest, evidence identity, payload/manifest/total snapshot sizes, and SHA-256 hashes. The total retained bytes, including `manifest.json`, must fit the budget. Publication and later verification each reconstruct a disposable clean room solely from preserved bytes and compare exact Git-visible state. A successful correlation receipt makes lost-response retries exact; a late publication race is quarantined and the unit becomes durably blocked.
- `retirement-preserve` and `retirement-verify` expose the same authority through the one-process CLI and persistent CLI transport. Neither command removes a worktree.

## Public seams and tests

The TDD seam is `AlbertMission` session launch/run/review/cancel/repair plus `preserve_retirement_unit()` and `verify_retirement_snapshot()`, with the CLI as the compatibility boundary. `tests/test_retirement_preservation.py` covers:

1. reservation before execution and release only after verified preservation;
2. one-winner same-revision preservation/review concurrency and stale execution/cancel/repair/duplicate rejection;
3. stored/managed/canonical/Git Worktree Identity agreement and Coding Workspace exclusion;
4. terminal status with contradictory process-group liveness failing closed;
5. staged, unstaged, untracked, evidence, hash, readback, clean-room, and tamper behavior;
6. self-contained clean-room reconstruction after the source repository is moved, plus exact authority tamper rejection;
7. late lifecycle-race quarantine and fail-closed state; and
8. one-process CLI preservation followed by persistent-transport re-verification of the same snapshot.

## Verification

- Focused retirement preservation: 8/8 pass.
- Retirement preservation plus deterministic runner supervision: 24/24 pass.
- Nine focused evidence/review/repair/TUI/workspace lifecycle regressions: 9/9 pass with only the host test process's documented missing-Bubblewrap isolation boundary replaced.
- Focused frontend compatibility: 220/220 pass under Node 25's required `--no-experimental-webstorage` test setting.
- Localhost gateway: 23/23 pass; TypeScript typecheck, Vite production build, Cargo formatting, Python compile, scoped Ruff, and diff checks pass.
- Documentation validation: Grade A (98.5%). The audit retains five pre-existing findings: three broad recent-feature connection heuristics and two orphaned FirstMate reports.
- The required full Python run executed once: 507 tests, with 32 failures, 170 errors, and 8 skips. The red cases are dominated by the documented macOS absence of trusted Bubblewrap plus `/var` versus `/private/var` fixture assumptions; the focused retirement tests are green.
- The broad Vitest and Rust runs were also executed once. Vitest remains host-red on Node 25 Web Storage defaults and Linux release fixtures; the Rust suite passes 43/54 and retains 11 host-bound failures from missing Bubblewrap, macOS path aliases, and existing conversation-scope fixtures. No failing broad test exercises the new Retirement Snapshot contract.

## Independent review

Standards and Spec reviews ran in parallel against fixed point `8704280`. Review corrections made expected session revisions real at external lifecycle/evidence boundaries; replaced terminal-derived owner claims with an independently observable runner-owner lease; made the baseline bundle self-contained; rejected unregistered directories; validated complete record/manifest/evidence/budget authority; included final manifest bytes in the budget; added exact correlation-idempotent replay; and quarantined late publication races. Final Spec review reports no findings. Final Standards review reports zero hard findings and retains only a non-blocking Primitive Obsession/Data Clump judgement about the nested retirement dictionaries as a future typed-state refactor.

## Known boundaries

- macOS does not expose Linux `/proc` process-start identities. The production probe uses the exact owner-release receipt and POSIX process-group absence where available, and otherwise fails closed. The host currently cannot start the persistent Apple container (`Operation not permitted`), so no manual browser evidence was possible.
- Snapshot reconstruction uses only the preserved baseline bundle, patches, and files. Aggregate payload retention policy remains Issue #68; preservation itself no longer depends on the source repository retaining the baseline object.
- A failed proof enters `preservation-blocked` with its budget still bound. Retry, export, discard, attention projection, and automatic retirement policy remain downstream work.
