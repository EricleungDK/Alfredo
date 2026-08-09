# Issue #68 Retirement Storage and Blocked Outcomes

**Date:** 2026-08-09

**Status:** Implemented and proportionally validated locally; independent review pending

**Issue:** [GitHub #68](https://github.com/EricleungDK/Alfredo/issues/68)

**Parent:** [PRD #56](https://github.com/EricleungDK/Alfredo/issues/56)

## Outcome

Snapshot Payloads now have bounded retention and aggregate-capacity policy without weakening the Preservation Budget, Runner Quiescence, Worktree Identity, or Coding Workspace boundaries established by Issues #66 and #67. Retirement Records remain durable after payload reclamation, `/status` and `/storage` expose deterministic storage truth, and Retirement Blocked units have exact correlated inspect, retry, export, and Retained Worktree Discard actions.

## Storage Contract

- The default Snapshot Payload retention is 30 days and the default Snapshot Storage Budget is 5 GiB; both are configurable through the common one-process and persistent CLI boundary.
- Admission counts all retained Snapshot Payload bytes plus every still-bound 32 MiB Preservation Budget before reserving another Retirement Unit. A unit does not start when the aggregate cannot fit.
- Capacity pressure reclaims only expired, unpinned payloads whose Retirement Unit is already `retired`. Ordering is deterministic by creation time and session id. Grace, preservation/retirement-blocked, pinned, and other protected records remain untouched.
- Reclamation persists an exact per-session intent before deleting the deterministic app-local payload directory. Restart after deletion but before record commit proves absence and completes the same intent. The compact record retains Mission/session/outcome identity, Worktree Identity, sizes, manifest hash, expiry, and reclaimed disposition.
- Protected or pinned exhaustion persists `snapshot-storage-exhausted` attention and blocks new Retirement Unit admission. Storage inspection reports policy, payload/reserved/committed/available bytes, counts, expiry order, largest retained payloads, bounded reclamation history, and blockers.
- Verified snapshots created before Issue #68 remain readable. Their storage metadata is derived conservatively from the verified Preservation Budget timestamp or immutable manifest time when policy first needs it, then becomes durable on mutation.

## Blocked Actions and Replay

`retirement-inspect` exposes the exact unit revision, phase, terminal outcome, blocker, Worktree Identity, runner boundary, Preservation Budget, compact Retirement Record, and action availability. `retirement-pin` changes only retained payload policy. `retirement-retry` persists intent before resetting one blocked unit for a fresh bounded preservation or retirement attempt.

`retirement-export` persists the exact correlation, revision, destination, and manifest digest before reconstructing verified retained material. A crash after materialization but before the marker or canonical receipt is recoverable: retry accepts the destination only after byte/mode/symlink comparison with a fresh reconstruction and exact baseline/status proof for Git. Any changed export remains blocked and is never overwritten.

Retained Worktree Discard requires the exact session id as confirmation, an explicit reason, a terminal blocked unit, fresh independent Runner Quiescence, the deterministic managed worktree path, and Coding Workspace exclusion. It persists intent before moving/removing the exact path, proves final path and applicable Git-registration absence, records the irreversible outcome, and releases a still-bound Preservation Budget only after deletion. Exact correlation replay never repeats an effect or overwrites newer canonical state; changed-boundary reuse fails closed.

## Public Seams

- `AlbertMission.retirement_storage_inspection()` and `inspect_retirement_unit()` provide deterministic read-only projections.
- `set_retirement_snapshot_pin()`, `retry_retirement_unit()`, `export_retirement_unit()`, and `discard_retained_worktree()` are expected-revision and correlation guarded.
- CLI and persistent transport share `retirement-storage`, `retirement-inspect`, `retirement-pin`, `retirement-retry`, `retirement-export`, and `retirement-discard`.
- Agent Console `/status` includes a compact Snapshot Payload summary and `/storage` renders the full deterministic inspection without controller inference.

## Verification

- Retirement preservation/lifecycle/storage coverage: 65/65 passing.
- Affected Agent Console help/status/storage seam: 1/1 passing.
- Python compilation, TypeScript typecheck, Vite production build, Cargo formatting, localhost gateway 23/23, functional browser 1/1, and persistent-container responsive layout 4/4 pass.
- Scoped Ruff fatal checks pass for the changed core, CLI, retirement, and retirement-test files. Including the complete modified workspace file reports the pre-existing undefined `EvidencePackage` annotation at `albert_mvp/workspace.py:9601`, outside the Issue #68 hunk.
- The one-time complete workspace run executed 265 tests with 20 failures, 58 errors, and 6 skips. The output reproduces the documented host baseline: trusted Bubblewrap is unavailable and Darwin canonicalizes `/var` temporary paths to `/private/var`; the focused Issue #68 workspace test is green.
- The complete Python run executed 568 tests with 35 failures, 172 errors, and 8 skips. The failures reproduce those same Bubblewrap and Darwin path-alias constraints; no Issue #68 retirement assertion failed.
- The complete frontend run passed 288/313 tests. Its 25 failures are the existing Darwin `/var` alias and unsupported Darwin/arm64 Ubuntu-release-fixture assumptions; the 142-test App suite and all Issue #68-independent frontend surfaces passed.
- Cargo tests passed 43/54. Its 11 failures reproduce the same missing-Bubblewrap, path-alias, and downstream scope-startup constraints; the Rust candidate contains no Issue #68 source change.
- Documentation audit reports six heuristic findings: three broad recent-feature connection ratios and three pre-existing orphan reports. Standards validation remains Grade A at 98.0%, with those same three low-severity report traceability findings.

## Remaining Gate

Create the scoped candidate commit, then complete independent parallel Standards, Spec, and Correctness/Risk review against fixed point `aa92ab36ec92280b696e758f60e6da07362e54b5`. Resolve every supported finding before marking this report complete. No push, pull request, or GitHub tracker mutation is part of this implementation run.
