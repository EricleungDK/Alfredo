# Switch Active Missions while Background Missions Continue

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Deliver compact mission switching within one Workspace Session. Changing the Active Mission updates the Operations Workspace while approved bounded work continues in Background Missions, Conversation Scope remains independent, and new governance needs route to Workspace Queue attention.

## Acceptance criteria

- [x] A compact mission selector changes the Active Mission and refreshes mission-specific operational state.
- [x] Switching missions preserves Agent Console history, the Workspace Session, and the current Conversation Scope.
- [x] Approved Local Agent sessions continue in Background Missions and remain inspectable after switching away.
- [x] Background approvals or clarifications create a compact attention indication linked to the relevant Workspace Queue item.
- [x] Tests run background work through mission switches and verify continuity, stable scope, and attention routing.

## Blocked by

- `02-synchronize-live-state-and-recover-from-disconnection.md`
- `03-agent-console-with-explicit-conversation-scope.md`

## Progress

- 2026-06-21: Dependencies complete. Defined the TDD architecture for a workspace-level mission registry, expected-revision switching, independent mission runtimes, stable scope/history, background session summaries, and compact Workspace Queue attention links.
- 2026-06-25: Completed with TDD. Python owns multi-mission workspace snapshots, expected-revision mission switching, background session summaries, and delegation/clarification queue attention. React exposes the compact Active Mission selector and mission catalog without retargeting Agent Console state. Tauri transports `workspace_mission_switch` through the Python CLI and restores the switched canonical snapshot.
- 2026-06-25: Verification passed: 3 focused Python Issue 05 tests, 30 focused frontend App/client tests, TypeScript typecheck, and 2 focused Rust bridge mission-switch tests.
