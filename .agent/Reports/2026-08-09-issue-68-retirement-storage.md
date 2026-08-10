# Issue #68 Retirement Storage and Blocked Outcomes

**Date:** 2026-08-09

**Status:** Implemented locally; first review corrections validated and clean re-review pending

**Issue:** [GitHub #68](https://github.com/EricleungDK/Alfredo/issues/68)

**Parent:** [PRD #56](https://github.com/EricleungDK/Alfredo/issues/56)

## Outcome

Snapshot Payloads now have bounded retention and aggregate-capacity policy without weakening the Preservation Budget, Runner Quiescence, Worktree Identity, or Coding Workspace boundaries established by Issues #66 and #67. Retirement Records remain durable after payload reclamation, `/status` and `/storage` expose deterministic storage truth, and Retirement Blocked units have exact correlated inspect, retry, export, and Retained Worktree Discard actions.

## Storage Contract

- The default Snapshot Payload retention is 30 days and the default Snapshot Storage Budget is 5 GiB; both are configurable through the common one-process and persistent CLI boundary.
- Admission counts all retained Snapshot Payload bytes plus every still-bound 32 MiB Preservation Budget before reserving another Retirement Unit. One shared launch lock makes that proof, reservation, and session persistence atomic across Issue Slice, Ad Hoc, and headless admissions. A unit does not start when the aggregate cannot fit.
- Capacity pressure reclaims only expired, unpinned payloads whose Retirement Unit is already `retired`. Ordering is deterministic by creation time and session id. Startup and storage inspection sweep all such expired payloads even without new admission pressure. Grace, preservation/retirement-blocked, pinned, and other protected records remain untouched.
- Reclamation persists an exact per-session intent before deleting the deterministic app-local payload directory. Restart after deletion but before record commit proves absence and completes the same intent. The compact record retains Mission/session/outcome identity, Worktree Identity, sizes, manifest hash, expiry, and reclaimed disposition.
- Protected or pinned exhaustion persists `snapshot-storage-exhausted` attention and blocks new Retirement Unit admission. The blocker appears prominently in Mission Work and resolves after an unpin or policy change makes reclamation possible. Active-Mission storage inspection reports policy, payload/reserved/committed/available bytes, counts, expiry order, largest retained payloads, bounded reclamation history, and blockers.
- Verified snapshots created before Issue #68 remain readable. Their storage metadata is derived conservatively from the verified Preservation Budget timestamp or immutable manifest time when policy first needs it, then becomes durable on mutation.

## Blocked Actions and Replay

`retirement-inspect` exposes the exact unit revision, phase, terminal outcome, blocker, Worktree Identity, runner boundary, Preservation Budget, compact Retirement Record, and only actions that can actually run. A `preservation-blocked` unit without a verified Snapshot Payload therefore does not advertise export. `retirement-pin` changes only retained payload policy. `retirement-retry` persists intent before resetting one blocked unit for a fresh bounded preservation or retirement attempt.

`retirement-export` persists the exact correlation, revision, destination, and manifest digest before reconstructing verified retained material. A crash after materialization but before the marker or canonical receipt is recoverable: retry re-verifies the repository even when the marker already exists, using exact byte/mode/symlink comparison and baseline/status proof for Git. Any changed export remains blocked and is never overwritten.

Retained Worktree Discard requires the exact session id as confirmation, an explicit reason, a terminal blocked unit, fresh independent Runner Quiescence and open-handle absence, the deterministic managed worktree path, exact retained-content identity, and Coding Workspace exclusion. It persists intent before moving/removing the exact path with non-force Git cleanup, proves final path and applicable Git-registration absence, records the irreversible outcome, and releases a still-bound Preservation Budget only after deletion. A replacement directory is rejected without deleting its bytes. Pin and discard effects share the per-session effect lock, so a concurrent pin cannot strand an already completed deletion. Exact correlation replay never repeats an effect or overwrites newer canonical state; changed-boundary reuse fails closed.

Every persisted action receipt is validated against its action-specific shape during runtime load. The four meaningful mutations also pass through typed Workstation authority in Mission Work and the desktop bridge, use the session lifecycle revision, and append one Mission/session-linked Activity Journal entry. If the core effect commits before the Workstation receipt, exact correlation replay recovers the canonical effect and completes the missing audit side effects once.

## Public Seams

- `AlbertMission.retirement_storage_inspection()` and `inspect_retirement_unit()` provide deterministic read-only projections.
- `set_retirement_snapshot_pin()`, `retry_retirement_unit()`, `export_retirement_unit()`, and `discard_retained_worktree()` are expected-revision and correlation guarded.
- CLI and persistent transport share `retirement-storage`, `retirement-inspect`, `retirement-pin`, `retirement-retry`, `retirement-export`, and `retirement-discard`.
- The typed Workstation action family and desktop bridge expose pin/unpin, retry, export destination, and exact-session discard confirmation from the Mission Execution Tree and card detail.
- Agent Console `/status` includes a compact Snapshot Payload summary and Active-Mission `/storage` renders the full deterministic inspection without controller inference.

## Verification

- Retirement preservation/lifecycle/storage coverage: 72/72 passing, including every first-review executable reproduction and direct typed CLI action dispatch.
- Focused workspace coverage passes for the four audited retirement Workstation actions, exact replay-safe Activity Journal entries, blocked Retirement Record projection, and Active-Mission storage resolution.
- Workstation projection passes 32/32 and App passes 143/143, including the Retirement Record inspector plus reason/exact-session discard gate. TypeScript typecheck and the Vite production build pass.
- Python compilation, scoped Ruff fatal checks, Cargo formatting, and Cargo check pass. Including F82 for the complete modified workspace file reports the pre-existing undefined `EvidencePackage` annotation now at `albert_mvp/workspace.py:9782`, outside the Issue #68 hunk.
- The complete Python run continues to reproduce the documented host constraint that trusted Bubblewrap is unavailable; focused Issue #68 suites are green. The earlier counted broad run was 568 tests with 35 failures, 172 errors, and 8 skips from Bubblewrap and Darwin `/var` aliasing.
- The complete frontend run passes 267/292. Its 25 failures remain the existing unsupported Darwin/arm64 Ubuntu-release fixtures, Darwin `/var` aliasing, missing Bubblewrap, and the downstream real-backend release seam. The transient generic-CLI parser regression discovered by this gate was fixed and regression-tested before this recorded rerun.
- Cargo tests pass 43/54. Its 11 failures reproduce the same missing-Bubblewrap, path-alias, and downstream scope-startup constraints; the typed retirement bridge compiles and formats cleanly.
- Documentation audit reports six heuristic findings: three broad recent-feature connection ratios and three pre-existing orphan reports. Standards validation remains Grade A at 98.0%, with those same three low-severity report traceability findings.

## Review Corrections

The first required review against `aa92ab36ec92280b696e758f60e6da07362e54b5...6940130` found three Standards gaps, five Spec gaps plus one admission race, and six executable Correctness/Risk defects. The correction set covers every supported finding: strict receipts, Activity Journal integration, Active-Mission storage, typed Mission Work surfaces, accurate action availability, prominent/resolving attention, proactive expiry, atomic admission, recovered-export verification, pin/discard serialization, Git open-handle protection, and replacement-data safety.

## Remaining Gate

Commit the validated correction set, then repeat independent parallel Standards, Spec, and Correctness/Risk review against fixed point `aa92ab36ec92280b696e758f60e6da07362e54b5`. Resolve every supported finding before marking this report complete. No push, pull request, or GitHub tracker mutation is part of this implementation run.
