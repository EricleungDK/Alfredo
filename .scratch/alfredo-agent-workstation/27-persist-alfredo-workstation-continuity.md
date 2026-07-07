# Persist Alfredo Workstation Continuity

Status: ready-for-agent
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Persist and restore the meaningful Alfredo workstation state needed for continuity after backend restart, app relaunch, or desktop refresh. Restoration should focus on accepted and meaningful state, not exact replay of transient streaming animation or raw command bytes.

## Acceptance criteria

- [ ] Selected workspace, recent workspaces, selected controller/model, prompt transcript, active sessions, approvals, evidence links, card state, and side-pane selection restore after restart.
- [ ] Persisted Activity Journal records preserve meaningful attributed actions separately from canonical current-state snapshots.
- [ ] Ephemeral token streaming, loading animation, and raw transient terminal output are not required to replay exactly after restart.
- [ ] Restore behavior distinguishes backend runtime location from selected coding workspace.
- [ ] Persistence tests restart the app/backend and verify transcript, Agent Workstations state, approvals, evidence links, recent workspaces, and selected workspace continuity.

## Blocked by

- `23-project-live-agent-workstation-cards.md`
- `25-route-first-consequential-workstation-action.md`

