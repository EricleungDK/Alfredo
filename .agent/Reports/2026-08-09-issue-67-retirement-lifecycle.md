# Issue #67 Retirement Lifecycle

**Date:** 2026-08-09

**Status:** Implemented locally; GitHub tracker unchanged

**Issue:** [GitHub #67](https://github.com/EricleungDK/Alfredo/issues/67)

**Parent:** [PRD #56](https://github.com/EricleungDK/Alfredo/issues/56)

## Outcome

Retirement is now a canonical, restart-safe lifecycle rather than a manual preservation-only step. Accepted evidence and completed cancellation preserve and retire automatically after exact Runner Quiescence. Failed or rejected work receives passive configurable grace, defaulting to 72 hours, without automatic investigation or repair. Needs-human-review remains nonterminal and retained.

## Durable Contract

Retirement Units persist `preserving` and `retiring` as pre-effect intents. `preserved`, `grace`, `retired`, `preservation-blocked`, and `retirement-blocked` are durable receipts after verification, policy transition, completion, or a blocked state requiring Mission Commander attention. Preservation intent makes an interrupted publication recoverable. Never-started cancellation records exact durable no-runner proof instead of relying on an external probe. Retirement records an initial token-free attempt, one automatic short-backoff retry, and one later reconciliation/startup retry. A per-unit lock serializes preservation and retirement effects across processes.

Git-backed worktrees preserve ignored as well as ordinary untracked material. Before cleanup, Git and managed-directory worktrees move atomically to a deterministic app-local removal-effect path. Validation and deletion operate only there: new bytes written through the original path remain intact and block retirement, same-user open descriptors are proven absent before deletion, and a crash after isolation resumes from the same effect path. Exact patch bytes, file bytes, symlink targets, and modes must match the snapshot or its provably partial per-path cleanup state before exact non-force `git worktree remove`; partial Git removal repairs only the exact registered marker before resuming non-force removal. An absent managed path with a lingering exact Git registration uses registration-only non-force removal rather than retiring as pure absence. Completion proves the original path, effect path, and Git registration are absent, while unavailable registration or open-handle inspection fails durably closed. Non-Git units must originate at the exact deterministic app-managed directory and may resume only from an exact subset of its preserved bytes/modes. Terminal sessions with no materialized worktree preserve verified deterministic absence. The Coding Workspace is never a retirement target, post-preservation new bytes block removal, corrupt snapshots become a durable `retirement-blocked` state requiring attention, and overlapping reconcilers cannot execute concurrent preservation or deletion.

## Repair Lineage

A repair request verifies and materializes its predecessor's preserved snapshot into an app-local clean room, stages that material with its manifest digest, retires the predecessor, and only then persists one separately authorized queued repair unit. Repair no longer depends on a mutable predecessor worktree remaining available.

## Verification

Focused retirement coverage currently passes 44/44 and proves automatic accepted/cancelled retirement, production never-started quiescence proof, queued-cancellation absence, pre-worktree-failure grace, live-cancellation deferral, failed/rejected grace, retained human review, CLI-configurable grace, serialized publication recovery, exact ignored-file preservation, original-path and open-descriptor boundary-write retention, partial Git/directory cleanup recovery, isolated-effect and partial-Git-removal crash recovery, registration-only managed-absence cleanup, serialized reconcilers, three-attempt exhaustion across crash cuts, corrupt-snapshot blocking, fail-closed unavailable registration inspection, verified repair lineage, and contained deletion. Runner-supervision integration passes 17/17; Python compilation, focused Ruff fatal checks, diff checks, and TypeScript typecheck pass. The final repository-wide results and re-review are added before handoff.

## Scope Boundary

Issue #68 still owns aggregate snapshot retention, reclamation, pinning, export, and explicit retained-worktree discard. This slice does not push commits, publish a release, or mutate the GitHub tracker.
