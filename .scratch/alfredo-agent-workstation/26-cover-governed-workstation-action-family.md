# Cover the Governed Workstation Action Family

Status: ready-for-agent
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Extend the proven consequential-action path to the rest of Alfredo's governed workstation actions. Launch, retry, cancel, path grants, evidence acceptance, repair requests, human-review escalations, model assignment changes, Mission Draft confirmation, and Conversation Scope changes that affect the next model turn should all use typed Orchestrator requests and produce visible prompt/orchestrator turns.

## Acceptance criteria

- [ ] Launch, retry, cancel, approval, rejection, deferral, and path grant decisions cannot bypass Orchestrator validation or expected-revision checks.
- [ ] Evidence acceptance, repair requests, human-review escalation, model assignment changes, and Mission Draft confirmation create typed requests and visible human-readable prompt/orchestrator turns.
- [ ] Conversation Scope changes that affect the next model turn are visible near the composer and reconstructable from durable prompt or Activity Journal records.
- [ ] Stale actions explain the current state and recovery path, and disabled actions explain why they cannot be used.
- [ ] Governance tests cover accepted and rejected paths for each consequential action class.

## Blocked by

- `25-route-first-consequential-workstation-action.md`

