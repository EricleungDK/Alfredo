# Resolve Issue Change Proposals and Frontier Confirmations

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Deliver Workspace Queue as the sole governance inbox for Issue Change Proposals and Frontier Confirmations. Questionable actions remain pending, explain their source and consequence, and can be approved, rejected, or deferred without duplicating decision controls elsewhere.

## Acceptance criteria

- [x] Editing a governed field on a locked Issue Slice creates an Issue Change Proposal instead of mutating accepted state.
- [x] Ambiguous, risky, irreversible, or launch-boundary-changing Frontier Model actions create Frontier Confirmations and remain pending.
- [x] Workspace Queue groups and filters items by type and mission and shows source, requested action, affected boundary, and consequence.
- [x] Approve, reject, and defer outcomes are validated by the Orchestrator and reflected consistently across all views.
- [x] Mission Board, inspector, Agent Console, and background attention provide compact links to queue items without duplicate decision controls.
- [x] Governance tests prove locked state cannot change before approval and stale or rejected decisions preserve authoritative state.

## Blocked by

- `02-synchronize-live-state-and-recover-from-disconnection.md`
- `05-switch-active-and-background-missions.md`
- `06-navigate-issue-graph-and-inspect-issue-slice.md`

## Completion evidence

- Python Workspace Queue service persists Issue Change Proposals and Frontier Confirmations in `workspace-queue.json` without mutating accepted state before approval.
- Queue decisions support approve, reject, and defer with queue-revision stale checks; rejected and stale decisions preserve authoritative Mission state.
- CLI, Tauri, TypeScript, and React expose Workspace Queue projection and decisions.
- React Workspace Queue renders grouped governance items, source, requested action, affected boundary, consequence, proposed changes, and queue-only decision controls.
- Mission summaries expose pending queue item attention links as `workspace-queue#<item-id>`.
- Verification passed on 2026-06-26:
  - `python3 -m unittest tests.test_workspace_snapshot tests.test_albert_mvp` - 119 tests.
  - `npm test -- --run App.test.tsx workspace-client.test.ts workspace-sync.test.ts` - 46 tests.
  - `npm run typecheck`.
  - `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` - 14 Rust bridge tests.
  - `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check`.
