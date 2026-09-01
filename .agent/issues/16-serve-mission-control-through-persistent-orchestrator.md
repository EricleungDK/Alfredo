# Serve Mission Control Through a Persistent Orchestrator

Status: complete
Type: AFK

## What to build

Replace per-request Python launches with one supervised long-running Orchestrator while preserving typed contracts, canonical state, ordered updates, crash recovery, and restart behavior.

## Acceptance criteria

- [x] Warm snapshot and no-change update requests meet a local p95 budget of 150 ms.
- [x] Integration tests cover reuse, ordering, malformed responses, crash recovery, restart, and shutdown.

## Blocked by

- `15-launch-command-deck-reliably-across-supported-platforms.md`
