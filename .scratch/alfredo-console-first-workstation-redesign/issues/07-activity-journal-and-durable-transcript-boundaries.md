Status: complete
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Keep the Agent Console transcript and Activity Journal meaningful after the console-first redesign. Prompts, assistant/controller responses, consequential actions, command/grant approval decisions, assignments, outcomes, and evidence links should be reconstructable, while routine UI navigation and raw telemetry remain outside durable records unless summarized as evidence.

## Acceptance criteria

- [x] Durable Agent Console history includes prompts, assistant/controller responses, consequential actions, command/grant approval decisions, and outcomes.
- [x] Routine issue-board navigation, card selection, filtering, sorting, expansion, and evidence viewing stay out of the prompt transcript unless they trigger a consequential action.
- [x] Activity Journal records meaningful attributed command, grant, assignment, approval, outcome, and evidence events.
- [x] Raw token streams and terminal bytes are excluded from durable prompt history and Activity Journal entries unless summarized as evidence.
- [x] Journal entries remain searchable or inspectable enough to reconstruct relevant command, grant, assignment, approval, and outcome decisions.
- [x] Transcript and Activity Journal tests cover inclusion boundaries, exclusion boundaries, attribution, and evidence-link behavior.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/02-inline-command-execution-cards.md
- .scratch/alfredo-console-first-workstation-redesign/issues/03-inline-approvals-and-contextual-path-grants.md
- .scratch/alfredo-console-first-workstation-redesign/issues/06-governed-issue-assignment-and-launch-actions.md

## Comments

- Implemented the remaining Shell Terminal durable-record boundary: command approval requests, denied commands, completed/failed command outcomes, and Additional Path Grant creation now append concise Agent Console transcript turns and attributed Activity Journal entries. Raw stdout/stderr remains transient and is not copied into transcript or journal summaries.
- Existing release transcript coverage already proves prompt/model commentary/workstation action inclusion, routine navigation exclusion, transient telemetry rejection, assignment attribution, and evidence links. Added backend coverage for command/grant inclusion boundaries and raw terminal byte exclusion.
- Verification: `python3 -m unittest tests.test_workspace_snapshot` passes 101 tests; `python3 -m unittest discover -s tests` passes 182 tests with 1 skip; `npm test -- --run` passes 125 frontend tests; `npm run typecheck` passes; `npm run build` passes; `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` passes; and `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` passes 27 Rust bridge tests.
