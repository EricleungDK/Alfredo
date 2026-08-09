# Issue #67 Retirement Lifecycle

**Date:** 2026-08-09

**Status:** Implemented locally; GitHub tracker unchanged

**Issue:** [GitHub #67](https://github.com/EricleungDK/Alfredo/issues/67)

**Parent:** [PRD #56](https://github.com/EricleungDK/Alfredo/issues/56)

## Outcome

Retirement is now a canonical, restart-safe lifecycle rather than a manual preservation-only step. Accepted evidence and completed cancellation preserve and retire automatically after exact Runner Quiescence. Failed or rejected work receives passive configurable grace, defaulting to 72 hours, without automatic investigation or repair. Needs-human-review remains nonterminal and retained.

## Durable Contract

Retirement Units persist `preserving`, `preserved`, `grace`, `retiring`, `retired`, `preservation-blocked`, and `retirement-blocked` before their corresponding effects. Preservation intent makes an interrupted publication recoverable. Retirement records at most three token-free effect attempts; startup reconciliation resumes intermediate phases, and an effect-before-receipt crash finalizes from independently verified absence without repeating deletion.

Git-backed worktrees must still match the verified snapshot, are restored only through exact tracked/untracked operations, and are removed with exact non-force `git worktree remove`. Completion proves both path and registration absence. Non-Git units must be the exact deterministic app-managed directory, must still match their snapshot bytes and modes, and are removed only at that validated boundary. The Coding Workspace is never a retirement target.

## Repair Lineage

A repair request verifies and materializes its predecessor's preserved snapshot into an app-local clean room, stages that material with its manifest digest, retires the predecessor, and only then persists one separately authorized queued repair unit. Repair no longer depends on a mutable predecessor worktree remaining available.

## Verification

Focused retirement coverage proves automatic accepted/cancelled retirement, failed and rejected grace, retained human review, configurable zero grace, publication recovery, startup recovery at retirement boundaries, three-attempt exhaustion, crash-after-removal finalization, post-preservation change blocking, verified repair lineage, exact Git registration absence, and contained non-Git deletion. The final repository-wide results and independent Standards/Spec review are recorded in `.agent/Tasks/context.md` after completion.

## Scope Boundary

Issue #68 still owns aggregate snapshot retention, reclamation, pinning, export, and explicit retained-worktree discard. This slice does not push commits, publish a release, or mutate the GitHub tracker.
