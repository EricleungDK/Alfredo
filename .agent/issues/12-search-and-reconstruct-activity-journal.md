# Search and Reconstruct Work through the Activity Journal

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Deliver Activity as the searchable chronological view of the append-only Activity Journal. Preserve attribution and links for meaningful actions while snapshots reconstruct current state and transient output remains outside the journal.

## Acceptance criteria

- [x] Meaningful Mission Commander, Orchestrator, Frontier Model, and Local Agent actions are recorded with time, actor, action type, affected entities, and available evidence links.
- [x] Activity displays entries chronologically and supports search plus mission, actor, action-type, and time filtering.
- [x] Entries link to affected Missions, Issue Slices, sessions, Evidence Packages, and queue decisions where available.
- [x] Model token streams, terminal bytes, and failed or unacknowledged actions are not promoted into accepted Activity Journal entries.
- [x] Restart reconstruction produces the same canonical current state from snapshots while retaining journal order and attribution separately.
- [x] Persistence and interaction tests cover recording, filtering, links, exclusions, failed writes, and restart reconstruction.

## Blocked by

- `02-synchronize-live-state-and-recover-from-disconnection.md`
- `08-review-evidence-packages.md`
- `09-resolve-change-proposals-and-frontier-confirmations.md`
- `10-approve-and-supervise-ad-hoc-delegation.md`
