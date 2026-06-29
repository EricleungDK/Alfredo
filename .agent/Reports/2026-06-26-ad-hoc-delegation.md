# Ad Hoc Delegation Approval and Supervision

Command Deck Issue 10 is complete. Conversational intent can now become a proposed Ad Hoc Delegation, enter Workspace Queue for explicit approval, launch as bounded `ADHOC-*` Local Agent work, and complete through Evidence Package review without becoming an Issue Slice.

## Implemented

- `WorkspaceQueueService` supports `ad-hoc-delegation` proposals with accepted Conversation Scope, acceptance criteria, allowed paths, command policy, proposed Local Agent, and originating Agent Console message id.
- Rejection preserves no-launch state; approval creates a bounded `LocalAgentSession` and never adds an `ADHOC-*` entry to `mission.issues`.
- Approval denies launch when the accepted command policy contains a non-`auto-allowed` command.
- Workspace Session mission summaries expose ad hoc session status plus role/provider/model provenance.
- Review Workspace projects ad hoc Evidence Packages and accepts valid evidence as complete bounded work without mutating Issue Slice lifecycle.
- CLI command `ad-hoc-delegation-proposal`, Tauri command `ad_hoc_delegation_proposal`, and React Workspace Queue proposal form expose the proposal path from the latest Agent Console message.

## Verification

- `python3 -m unittest tests.test_workspace_snapshot tests.test_albert_mvp` - 126 tests passed.
- `npm test -- --run App.test.tsx workspace-client.test.ts workspace-sync.test.ts` - 47 tests passed.
- `npm run typecheck` - passed.
- `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` - passed.
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` - 15 tests passed.

## Notes

Ad Hoc Delegations remain distinct from Issue Slices throughout their lifecycle. They use Workspace Queue as the approval surface and Review Workspace as the evidence decision surface, preserving the existing Command Deck boundary: Python owns accepted state, Tauri transports typed JSON, and React keeps pending UI state local until acknowledgement/reload.
