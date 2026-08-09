# Issue #67 Retirement Lifecycle

**Date:** 2026-08-09

**Status:** Implemented locally; GitHub tracker unchanged

**Issue:** [GitHub #67](https://github.com/EricleungDK/Alfredo/issues/67)

**Parent:** [PRD #56](https://github.com/EricleungDK/Alfredo/issues/56)

## Outcome

Retirement is now a canonical, restart-safe lifecycle rather than a manual preservation-only step. Accepted evidence and completed cancellation preserve and retire automatically after exact Runner Quiescence. Failed or rejected work receives passive configurable grace, defaulting to 72 hours, without automatic investigation or repair. Needs-human-review remains nonterminal and retained.

## Durable Contract

Retirement Units persist `preserving` and `retiring` as pre-effect intents. `preserved`, `grace`, `retired`, `preservation-blocked`, and `retirement-blocked` are durable receipts after verification, policy transition, completion, or a blocked state requiring Mission Commander attention. Preservation intent makes an interrupted publication recoverable. Retirement records an initial token-free attempt, one automatic short-backoff retry, and one later reconciliation/startup retry. A per-unit lock serializes preservation and retirement effects across processes.

Git-backed worktrees preserve ignored as well as ordinary untracked material. Exact patch bytes, file bytes, symlink targets, and modes must match the snapshot or its provably partial per-path cleanup state before exact non-force `git worktree remove`; a boundary write blocks cleanup, and an absent path with lingering registration resumes the same command. Completion proves both path and registration absence, while unavailable registration inspection fails durably closed. Non-Git units must be the exact deterministic app-managed directory and may resume only from an exact subset of its preserved bytes/modes. Terminal sessions with no materialized worktree preserve verified deterministic absence. The Coding Workspace is never a retirement target, post-preservation new bytes block removal, corrupt snapshots become a durable `retirement-blocked` state requiring attention, and overlapping reconcilers cannot execute concurrent preservation or deletion.

## Repair Lineage

A repair request verifies and materializes its predecessor's preserved snapshot into an app-local clean room, stages that material with its manifest digest, retires the predecessor, and only then persists one separately authorized queued repair unit. Repair no longer depends on a mutable predecessor worktree remaining available.

## Verification

Focused retirement coverage currently passes 36/36 and proves automatic accepted/cancelled retirement, queued-cancellation absence, pre-worktree-failure grace, live-cancellation deferral, failed/rejected grace, retained human review, CLI-configurable grace, serialized publication recovery, exact ignored-file preservation, same-path and cleanup-boundary write blocking, partial Git/directory cleanup recovery, serialized reconcilers, three-attempt exhaustion across crash cuts, corrupt-snapshot blocking, absent-path registration recovery, fail-closed unavailable registration inspection, verified repair lineage, and contained deletion. Runner-supervision integration passes 17/17; Python compilation, focused Ruff fatal checks, diff checks, and TypeScript typecheck pass. The final repository-wide results and re-review are added before handoff.

## Scope Boundary

Issue #68 still owns aggregate snapshot retention, reclamation, pinning, export, and explicit retained-worktree discard. This slice does not push commits, publish a release, or mutate the GitHub tracker.
