Status: complete
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Move risky command approvals and Additional Path Grant requests into contextual Agent Console turns. Alfredo should ask for authority only when it is needed, show exactly what is being requested, and keep all accepted or denied decisions auditable without presenting grants as a standing configuration form.

## Acceptance criteria

- [x] Commands that require approval produce inline approval prompts before execution can proceed.
- [x] Additional Path Grant requests are raised contextually when a blocked command or agent action needs out-of-workspace access.
- [x] Path-grant prompts include path, access level, duration, reason, and affected action.
- [x] Agents and skills cannot create or expand grants without explicit Mission Commander authority.
- [x] Active, denied, and expired grants are visible when inspecting relevant command, approval, Activity Journal, or detail surfaces.
- [x] Tests cover command approval prompts, contextual grant prompts, grant decision outcomes, and absence of a standing default grant form.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/02-inline-command-execution-cards.md

## Comments

- Implemented inline Agent Console approval prompts for human-required Shell Terminal commands, while Frontier-approvable commands continue to show the Frontier approval boundary without Mission Commander approval controls.
- Blocked out-of-workspace command submissions now raise contextual Additional Path Grant prompts in the Agent Console with path, access, duration, reason, and affected action. Accepted and denied grant decisions append visible workstation/orchestrator turns.
- The Command Audit drill-down now lists grant history and explains that new grant authority is requested inline; the standing default grant creation form is removed.
- Verification: `python3 -m unittest discover -s tests` passes 180 tests with 1 skip; `npm test -- --run App.test.tsx` passes 57 tests; `npm test -- --run` passes 116 frontend tests; `npm run typecheck` passes; `npm run build` passes; `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` passes; and `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` passes 27 Rust tests.
