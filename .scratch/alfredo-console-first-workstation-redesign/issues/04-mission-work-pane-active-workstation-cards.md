Status: ready-for-agent
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Create the Mission Work side pane with Active Workstations as the top region and primary live-supervision surface. The Mission Commander should be able to see active, blocked, waiting-approval, reviewing, review-ready, failed, done, and idle workstation state while continuing to prompt in the Agent Console.

## Acceptance criteria

- [ ] The side pane is labeled and structured as Mission Work rather than a tab switcher between Workstations and Shell Terminal.
- [ ] Active Workstations are the top region and remain visible while the Mission Commander prompts.
- [ ] Workstation cards show agent or subagent identity, Issue Slice, actual model, role, current state, last meaningful activity, latest command/test summary, blocker or next action, and review/evidence affordances when present.
- [ ] Blocked and waiting-approval cards sort above routine active work while other states remain visually distinct.
- [ ] Workstation cards remain the primary live-supervision model and are not replaced by a dense table.
- [ ] Tests verify card projection, sorting priority, state distinctions, and visibility alongside the Agent Console.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/01-console-first-workstation-layout.md
