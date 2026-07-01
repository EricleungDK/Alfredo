# Governed Shell Terminal Completion Report

Date completed: 2026-07-01  
Issue: `.agent/issues/13-governed-shell-terminal-and-path-grants.md`

## Outcome

Issue 13 is complete. The production Command Deck now implements the approved Variant A relationship: Agent Console and Shell Terminal are distinct left-lane modes, Agent Console remains the startup default, and Terminal replaces Conversation Scope with an explicit execution-boundary summary.

## Acceptance evidence

| Acceptance criterion | Evidence |
| --- | --- |
| Distinct left-lane state | React tests `switches distinct left-lane modes without mixing console and terminal drafts` and `submits auto-allowed terminal command and keeps output local to the session` prove independent drafts/history and removal of the inactive panel from the document and accessibility tree. |
| Three command classes and approval gates | Python auto-allowed, Frontier approval, and human denial tests; React human approval and Frontier-boundary tests; Rust real-backend approval bridge test. |
| Actionable denial and path failure | Python denial/expired/no-write-grant coverage plus React `keeps terminal inputs and shows actionable path rejection without false success` prove no execution or false completed state. |
| Bounded Additional Path Grants and expiry | Python bounded external-read and expiry tests, TypeScript grant transport test, Rust real-backend grant creation, and React creation/expired-history tests. |
| No self-expansion | Python `test_local_agent_cannot_expand_or_renew_additional_path_grant` and React immutable-history assertions exclude edit, renew, broaden, agent, and skill controls. |
| Terminal-byte separation | Python canonical snapshot and Activity Journal invariants, CLI metadata-restoration test, Rust metadata-only projection test, and React current-session transcript test. |
| Constrained desktop interaction | React arrow-key tab test at 700 px, named region/tabpanel semantics, visible textual governance states, and constrained reflow styles. |

## Changed boundaries

- `mission-control/src/use-shell-terminal.ts` owns transient React controller state and typed client orchestration.
- `mission-control/src/ShellTerminalPanel.tsx` renders governed execution, pending decisions, current-session output, and immutable Additional Path Grants.
- `mission-control/src/App.tsx` owns startup-default lane selection and accessible tab/tabpanel switching.
- `mission-control/src/styles.css` implements the industrial Command Deck terminal layout and constrained reflow.
- Existing Python, TypeScript client, and Rust/Tauri boundaries remain authoritative for classification, execution, persistence, and transport.

## Fresh verification

- `python3 -m unittest discover -s tests` — 157 passed.
- `npm test -- --run` — 69 passed across 3 files.
- `npm run typecheck` — passed.
- `npm run build` — passed; Vite production bundle generated.
- `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` — passed.
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` — 24 passed.

## Remaining risk

Human confirmation of the overall Command Deck hierarchy, contrast, zoom/reflow, and assistive-technology behavior belongs to Issue 14. Automated Issue 13 coverage proves the Shell Terminal contract but does not substitute for that human review.
