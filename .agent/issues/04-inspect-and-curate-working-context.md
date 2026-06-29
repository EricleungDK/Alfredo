# Inspect and Curate Working Context

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Let the Mission Commander examine how Working Context was assembled for the current Conversation Scope and influence eligible source selection without bypassing Shared Context governance. Preserve the full Agent Console history while keeping model input bounded and reconstructable.

## Acceptance criteria

- [x] Context Inspector lists the Workspace Session, Shared Context, unresolved-item, recent-conversation, and deliberately referenced sources used for Working Context.
- [x] Eligible sources can be pinned or excluded and the resulting Working Context is visibly updated after Orchestrator acknowledgement.
- [x] Ineligible or governed Shared Context cannot be rewritten, suppressed, or promoted through Context Inspector controls.
- [x] Working Context remains bounded while full Agent Console history stays available for human inspection.
- [x] Tests verify source reconstruction, eligible pin/exclude behavior, rejected governance bypasses, and persistence across restart.

## Blocked by

- `03-agent-console-with-explicit-conversation-scope.md`

## Progress

- 2026-06-21: Issue 03 dependency completed. Defined the TDD architecture for a bounded typed source projection, separately revisioned eligible curation, governed required sources, scoped recent history, and persistent deliberate references.
- 2026-06-21: Backend/CLI checkpoint green: all five source categories reconstruct deterministically, recent scoped history is limited to six while full history remains intact, pin/exclude state restores at its own revision, malformed persistence fails closed, and governed/stale curation leaves workspace and issue state unchanged.
- 2026-06-21: Completed typed Tauri inspect/curate transport and the React Context Inspector. User tests prove governed controls are unavailable, full history remains visible beside a six-message model window, and eligible projection changes only after acknowledgement and reload.
- 2026-06-21: Ticket integration passed: 94 Python tests, 30 frontend tests, TypeScript typecheck, production build, Rust formatting, 10 Rust tests, and a fresh-runtime inspect→pin→exclude→restart smoke. The smoke also exposed and fixed non-atomic core runtime persistence; a deterministic regression and eight concurrent process reads now pass.
