# Operate the Agent Console with Explicit Conversation Scope

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Deliver the persistent Agent Console as the Mission Commander's unified conversational lane. Messages target an explicitly selected Working directory, Mission, or Issue Slice Conversation Scope, and that target remains stable while the user navigates or switches missions.

## Acceptance criteria

- [x] Agent Console history remains continuous while Operations Workspace views and Active Mission change.
- [x] The composer displays and deliberately changes Conversation Scope among Working directory, Mission, and Issue Slice targets.
- [x] Navigating, selecting an Issue Slice, or switching Active Mission cannot silently change the target of a composed or submitted message.
- [x] Changing Conversation Scope influences Working Context only and cannot launch work, expand permissions, grant path access, approve decisions, or mutate locked state.
- [x] Messages and sourced narration distinguish proposed, pending, acknowledged, rejected, and model-commentary outcomes.
- [x] User-level tests verify continuous history, all scope targets, navigation stability, and authorization boundaries.

## Blocked by

- `01-open-and-restore-command-deck-workspace-session.md`
- `02-synchronize-live-state-and-recover-from-disconnection.md`

## Progress

- 2026-06-20: Issues 01-02 dependencies completed. Wrote the TDD implementation plan and began the deliberate scope-change public-boundary tracer bullet.
- 2026-06-20: First RED→GREEN cycle complete: `workspace-scope` validates and persists deliberate Mission scope at an expected revision while preserving Operations view and Issue Slice contract state.
- 2026-06-21: Backend/transport checkpoint complete: scoped append-only history, all five outcomes, malformed-history rejection, authorization invariants, JSON CLI commands, and real Tauri→Python scope/message/restart restoration are implemented and focused tests pass.
- 2026-06-21: Completed the React Agent Console projection and deliberate scope controls. User tests prove history/draft/scope continuity across Operations navigation and Active Mission replacement, explicit submission scope, all three targets, and all five sourced outcomes.
- 2026-06-21: Ticket integration passed: 86 Python tests, 26 frontend tests, TypeScript typecheck, production build, Rust formatting, 9 Rust tests, and a fresh-runtime scope→message→restart CLI smoke restoring revision 2 and the original ISS-03 message scope.
