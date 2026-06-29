# Approve and Supervise an Ad Hoc Delegation

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Allow conversational intent to become a proposed Ad Hoc Delegation, then require explicit Workspace Queue approval before a Local Agent can execute it. Carry the bounded work through execution visibility, evidence, and review without turning it into an Issue Slice.

## Acceptance criteria

- [x] A proposed Ad Hoc Delegation records scope, acceptance criteria, permissions, proposed Local Agent, and its originating conversation context.
- [x] The delegation appears in Workspace Queue and cannot launch until explicitly approved by the Mission Commander.
- [x] Approval launches only within the accepted boundaries and exposes Local Agent session status and model provenance in the Workspace Session.
- [x] Completion requires an Evidence Package and review equivalent to other bounded Local Agent work.
- [x] The delegation remains distinguishable from an Issue Slice throughout its lifecycle.
- [x] End-to-end tests cover proposal, rejection, approval, bounded launch, evidence validation, review, and permission denial.

## Progress

- 2026-06-26: Backend TDD coverage is green for proposal, rejection, approval, bounded `ADHOC-*` session launch, command-policy denial, session provenance, Evidence Package validation, and Review Workspace acceptance without Issue Slice creation.
- 2026-06-26: CLI/Tauri/React proposal path is green: latest Agent Console context can become a pending Ad Hoc Delegation proposal in Workspace Queue.
- 2026-06-26: Issue 10 closeout verification is green: 126 Python tests, 47 focused frontend/client/sync tests, TypeScript typecheck, Rust formatting, and 15 Rust bridge tests pass.

## Blocked by

- `07-ollama-provider-neutral-model-assignments.md`
- `09-resolve-change-proposals-and-frontier-confirmations.md`
