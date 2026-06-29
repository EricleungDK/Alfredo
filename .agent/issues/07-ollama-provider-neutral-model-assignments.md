# Run Ollama through Provider-Neutral Model Assignments

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Expose model assignment, availability, streaming status, and provenance through provider-neutral contracts while shipping Ollama as the first adapter. Let the Mission Commander identify who performed work and understand provider failures without confusing infrastructure state with mission lifecycle.

## Acceptance criteria

- [x] Model assignments expose stable Albert role, provider, model identity, and availability without requiring provider-specific UI logic.
- [x] The Ollama adapter implements the same assignment, availability, streaming, and failure contract exercised by a provider-neutral fake adapter.
- [x] Issue Slice and session surfaces display role/provider/model provenance from authoritative state.
- [x] An unavailable assigned model is visible before launch and blocks or fails only the affected operation.
- [x] Availability, streaming, and provider failures do not mutate accepted mission state or Issue Slice lifecycle incorrectly.
- [x] Contract and interaction tests cover available, unavailable, disconnected, streaming, and failed Ollama states.

## Blocked by

- `02-synchronize-live-state-and-recover-from-disconnection.md`
- `06-navigate-issue-graph-and-inspect-issue-slice.md`

## Verification

- `python3 -m unittest tests.test_workspace_snapshot tests.test_albert_mvp` passes 105 tests.
- `npm test -- --run App.test.tsx workspace-client.test.ts workspace-sync.test.ts` passes 34 tests.
- `npm run typecheck` passes.
- `cargo test` passes 12 Rust bridge tests.
