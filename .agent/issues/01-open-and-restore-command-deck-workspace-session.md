# Open and Restore the Command Deck Workspace Session

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Deliver the first complete desktop path from launching the Tauri Mission Control App through connecting to the long-running Orchestrator, receiving a canonical state snapshot, and rendering a restored Workspace Session in the approved Command Deck layout. The React interface is a projection of authoritative backend state and must not claim accepted changes that the Orchestrator has not acknowledged.

## Acceptance criteria

- [x] Launching the app connects to the local Orchestrator and renders a versioned canonical snapshot for the open workspace.
- [x] Reopening an existing workspace restores its current Workspace Session, Active Mission, Conversation Scope, and sensible operational view from authoritative state.
- [x] The visible shell has the persistent left lane and focused Operations Workspace responsibilities established by the Command Deck direction.
- [x] Loading, empty, backend-startup failure, and persistence-read failure states are visible and actionable without displaying fabricated accepted state.
- [x] End-to-end tests exercise launch, first snapshot, restart, and restoration through the desktop-to-backend boundary.

## Blocked by

None - can start immediately

## Progress

- 2026-06-19: Added and verified the versioned Python Workspace Session snapshot boundary, atomic preference restoration, empty state, and structured persistence-read failure (4 focused tests).
- 2026-06-19: Added the React/TypeScript Command Deck projection with restored mission, scope, operational view, loading, empty, startup-failure, retry, and persistence-failure behavior (5 interaction tests; typecheck and production build pass).
- 2026-06-20: Added the typed Tauri command boundary and production `TauriWorkspaceClient`; Rust tests cover successful decoding, startup/contract/persistence failures, repository configuration, and real Python launch plus restart restoration.
- 2026-06-20: Ticket integration passed: 72 Python tests, 7 React tests, TypeScript typecheck, Vite production build, 6 default-feature Rust tests, and a real repository `workspace-snapshot` smoke command.
