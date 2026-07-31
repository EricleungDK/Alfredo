# Cover the Governed Workstation Action Family

Status: complete
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Extend the proven consequential-action path to the rest of Alfredo's governed workstation actions. Launch, retry, cancel, path grants, evidence acceptance, repair requests, human-review escalations, model assignment changes, Mission Draft confirmation, and Conversation Scope changes that affect the next model turn should all use typed Orchestrator requests and produce visible prompt/orchestrator turns.

## Acceptance criteria

- [x] Launch, retry, cancel, approval, rejection, deferral, and path grant decisions cannot bypass Orchestrator validation or expected-revision checks.
- [x] Evidence acceptance, repair requests, human-review escalation, model assignment changes, and Mission Draft confirmation create typed requests and visible human-readable prompt/orchestrator turns.
- [x] Conversation Scope changes that affect the next model turn are visible near the composer and reconstructable from durable prompt or Activity Journal records.
- [x] Stale actions explain the current state and recovery path, and disabled actions explain why they cannot be used.
- [x] Governance tests cover accepted and rejected paths for each consequential action class.

## Blocked by

- `25-route-first-consequential-workstation-action.md`

## Comments

### 2026-07-12 — completed

- The action family now includes exact typed contextual path-grant requests, durable rejected Workstation request/rejection phases, stale recovery guidance, and fail-closed local worker/controller authority.
- Fresh merged evidence: 416 Python tests (one optional skip), 215 frontend tests, TypeScript/build, Rust formatting, and 36 Tauri/Python bridge tests pass.
