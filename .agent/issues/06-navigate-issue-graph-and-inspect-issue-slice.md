# Navigate the Issue Graph and Inspect an Issue Slice

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Make Mission Board the concise answer to what is happening in the Active Mission. Render the Issue Graph, progress, blockers, and lifecycle semantics, then provide on-demand Issue Slice inspection with accepted boundaries, sessions, provenance, evidence, and context.

## Acceptance criteria

- [x] Mission Board renders the Issue Graph, blocker relationships, concise mission progress, and launch eligibility from authoritative state.
- [x] Ready is displayed only for approved, unblocked, launch-eligible Issue Slices; Complete means evidence-accepted and PR-ready and remains distinct from merged.
- [x] Selecting an Issue Slice opens an inspector without changing Conversation Scope.
- [x] The inspector shows accepted boundary, blockers, lifecycle, attached sessions, role/provider/model provenance, Evidence Package summary, and Working Context sources.
- [x] Individual session detail is available through drill-down, including stale or disconnected status where applicable.
- [x] Interaction tests cover graph navigation, blocker state, lifecycle terminology, inspector detail, and scope stability.

## Blocked by

- `02-synchronize-live-state-and-recover-from-disconnection.md`
- `05-switch-active-and-background-missions.md`

## Verification

- `python3 -m unittest tests.test_workspace_snapshot tests.test_albert_mvp` passes 99 tests.
- `npm test -- --run App.test.tsx workspace-client.test.ts workspace-sync.test.ts` passes 34 tests.
- `npm run typecheck` passes.
- `cargo test` passes 12 Rust bridge tests.
