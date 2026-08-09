# Issue #67 Retirement Lifecycle

**Date:** 2026-08-09

**Status:** Implemented locally; GitHub tracker unchanged

**Issue:** [GitHub #67](https://github.com/EricleungDK/Alfredo/issues/67)

**Parent:** [PRD #56](https://github.com/EricleungDK/Alfredo/issues/56)

## Outcome

Retirement is now a canonical, restart-safe lifecycle rather than a manual preservation-only step. Accepted evidence and completed cancellation preserve and retire automatically after exact Runner Quiescence. Failed or rejected work receives passive configurable grace, defaulting to 72 hours, without automatic investigation or repair. Needs-human-review remains nonterminal and retained.

## Durable Contract

Retirement Units persist `preserving` and `retiring` as pre-effect intents. `preserved`, `grace`, `retired`, `preservation-blocked`, and `retirement-blocked` are durable receipts after verification, policy transition, completion, or a blocked state requiring Mission Commander attention. Preservation intent makes an interrupted publication recoverable. Never-started cancellation records exact durable no-runner proof and reconciles inline; running cancellation waits for the runner-finalization reconciliation path. Failure transitions enter grace in the same persistent process. Retirement records an initial token-free attempt, one automatic short-backoff retry, and one later reconciliation/startup retry. A per-unit lock serializes preservation and retirement effects across processes.

Git-backed worktrees preserve ignored as well as ordinary untracked material. Before cleanup, Git and managed-directory worktrees move atomically to a deterministic app-local removal-effect path. Validation and deletion operate only there: new bytes written through the original path remain intact and block retirement; same-user open descriptors, process cwd/root references, and writable shared mappings are proven absent before deletion; and a crash after isolation resumes from the same effect path. Exact patch bytes, file bytes, symlink targets, and modes must match the snapshot or its provably partial per-path cleanup state before exact non-force `git worktree remove`; split move back-pointers and partial marker loss repair only one exact matching Git administration entry before resuming non-force removal. If an interrupted recursive removal also unlinks tracked content, recovery accepts only missing working-tree entries whose index identity still exactly matches the verified preserved or baseline index, reconstructs them through Git, and retries non-force removal. An absent managed path with a lingering exact Git registration uses registration-only non-force removal rather than retiring as pure absence. Completion proves the original path, effect path, and Git registration are absent, while unavailable registration or process-reference inspection fails durably closed. Non-Git units must originate at the exact deterministic app-managed directory and may resume only from an exact subset of its preserved bytes/modes. Terminal sessions with no materialized worktree preserve verified deterministic absence. The Coding Workspace is never a retirement target, post-preservation new bytes block removal, corrupt snapshots become a durable `retirement-blocked` state requiring attention, and overlapping reconcilers cannot execute concurrent preservation or deletion.

## Repair Lineage

A repair request verifies and materializes its predecessor's preserved snapshot into an app-local clean room, stages that material with its manifest digest, retires the predecessor, and only then persists one separately authorized queued repair unit. Repair no longer depends on a mutable predecessor worktree remaining available.

## Verification

Focused retirement coverage currently passes 47/47 and proves automatic accepted/cancelled retirement, same-process never-started cancellation and pre-worktree-failure policy, production no-runner proof, live-cancellation deferral without preservation races, failed/rejected grace, retained human review, CLI-configurable grace, serialized publication recovery, exact ignored-file preservation, original-path/open-descriptor/cwd/shared-mapping retention, partial Git/directory cleanup recovery, isolated-effect/split-move/partial-marker-and-tracked-content crash recovery, registration-only managed-absence cleanup, serialized reconcilers, three-attempt exhaustion across crash cuts, corrupt-snapshot blocking, fail-closed unavailable registration inspection, verified repair lineage, and contained deletion. Runner-supervision integration passes 17/17; Python compilation, focused Ruff fatal checks, diff checks, and TypeScript typecheck pass. The final repository-wide results and re-review are added before handoff.

## Scope Boundary

Issue #68 still owns aggregate snapshot retention, reclamation, pinning, export, and explicit retained-worktree discard. This slice does not push commits, publish a release, or mutate the GitHub tracker.
