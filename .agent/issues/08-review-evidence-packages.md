# Review Evidence Packages in Review Workspace

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Deliver Review Workspace as the exclusive evidence-decision surface. A Mission Commander can inspect a complete Evidence Package, understand limitations and consequences, and submit accept, repair, or human-escalation decisions to the authoritative Orchestrator.

## Acceptance criteria

- [x] Review Workspace lists work awaiting review and displays changed files, diff summary, commands, test results, risks, proposed context updates, and visibility limitations.
- [x] Missing required evidence blocks acceptance and explains what is incomplete.
- [x] Accepting valid evidence makes the Issue Slice Complete and PR-ready without presenting it as merged.
- [x] Repair requests require a reason and expose the resulting next action; human escalation records an explicit needs-human-review outcome.
- [x] Consequential review actions explain their effect before submission and update UI state only after acknowledgement.
- [x] End-to-end tests cover complete and incomplete evidence, accept, repair, escalation, stale decisions, limitations, and backend rejection.

## Blocked by

- `02-synchronize-live-state-and-recover-from-disconnection.md`
- `06-navigate-issue-graph-and-inspect-issue-slice.md`

## Verification

- `python3 -m unittest tests.test_workspace_snapshot tests.test_albert_mvp` passes 112 tests.
- `npm test -- --run App.test.tsx workspace-client.test.ts workspace-sync.test.ts` passes 43 tests.
- `npm run typecheck` passes.
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` passes 13 Rust bridge tests.
- `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` passes.
