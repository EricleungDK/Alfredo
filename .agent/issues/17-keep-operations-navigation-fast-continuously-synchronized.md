# Keep Operations Navigation Fast and Continuously Synchronized

Status: complete
Type: AFK

## What to build

Keep every Mission Control lane responsive and continuously synchronized without duplicate projection loads or invented accepted state.

## Acceptance criteria

- [x] Browser journeys cover tabs, retry, outcomes, reconnect, keyboard switching, and constrained-width reflow.
- [x] Warm interactions issue no duplicate projection request per acknowledged transition.

## Blocked by

- `16-serve-mission-control-through-persistent-orchestrator.md`
