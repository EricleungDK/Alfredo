# Project Live Agent Workstation Cards

Status: ready-for-agent
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Feed the Agent Workstations side pane from typed Orchestrator/session events and canonical snapshots, then render compact live workstation cards for agents and subagents. Cards should make live supervision readable at a glance without inventing accepted state before Orchestrator acknowledgement.

## Acceptance criteria

- [ ] Each live card shows agent or subagent name, actual model, role, current task, status, phase or progress, last activity, approval blockers, files touched count, latest command or test summary, and next action.
- [ ] Card status covers thinking, running, waiting approval, blocked, reviewing, review-ready, done, and failed states from canonical Orchestrator/session state.
- [ ] Blocked and waiting-approval cards sort above other live work, active work is grouped separately from done or historical sessions, and optional mission or scope grouping is available when many sessions exist.
- [ ] The UI may show pending intent immediately, but accepted state appears only after Orchestrator acknowledgement with the expected revision.
- [ ] Projection tests prove cards update from canonical state and do not derive accepted state from frontend-only assumptions.

## Blocked by

- `22-build-prompt-dominant-workstation-shell.md`

