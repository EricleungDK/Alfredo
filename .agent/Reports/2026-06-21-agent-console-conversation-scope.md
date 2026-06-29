# Agent Console and Explicit Conversation Scope — Completion Evidence

**Issue:** `.agent/issues/03-agent-console-with-explicit-conversation-scope.md`

**Status:** Complete

## Delivered behavior

- Agent Console history is append-only and atomically persisted with stable sequence, role, content, source, outcome, and original Conversation Scope.
- Working directory, Mission, and Issue Slice scope changes are deliberate expected-revision actions validated by the Orchestrator.
- Public message submission requires the current acknowledged revision and the exact displayed scope; mismatches cannot append history.
- React owns only draft presentation. History, draft, and scope survive Operations navigation, reconnect, and Active Mission replacement without silent retargeting.
- Proposed, pending, acknowledged, rejected, and model-commentary records remain visibly distinct and sourced.
- Scope changes have no launch, approval, review, assignment, path-access, permission, or locked-contract mutation surface.

## Acceptance evidence

- Continuity: user-level tests retain persisted history and an unfinished draft across Operations navigation and retain history/draft/ISS-01 scope when a fresh canonical snapshot changes Active Mission.
- Deliberate targets: parameterized UI tests cover Working directory and Issue Slice targets; a separate test covers Mission scope. Each request contains the expected revision and explicit target.
- No silent retargeting: changing the selector alone leaves acknowledged scope unchanged, and submission after mission replacement still carries the displayed ISS-01 scope.
- Authorization boundary: Python integration compares issue files plus review, runtime, assignment, and session state before and after scope change. The scope command exposes no authority-bearing fields.
- Outcome semantics: history validation and UI records cover all five allowed outcomes; model commentary retains its distinct source/outcome.
- Persistence and malformed input: service restart restores the original scoped record; malformed roles, outcomes, sequences, and scopes are rejected as persistence failures.

## Integration evidence — 2026-06-21

- `python3 -m unittest discover -s tests -v`: 86 passed.
- `npm test -- --run`: 26 passed.
- `npm run typecheck`: passed.
- `npm run build`: passed.
- `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check`: passed.
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml`: 9 passed.
- Fresh-runtime CLI smoke: initial revision 1 changed deliberately to ISS-03 at revision 2; a proposed user message was appended with that exact scope; separate history and snapshot processes restored sequence 1, revision 2, and ISS-03 unchanged.

## Known limits

- Full history is preserved for the human surface, but bounded model-input assembly and source curation belong to Issue 04.
- Mission creation and switching controls belong to Issue 05; Issue 03 guarantees that an externally changed canonical Active Mission does not implicitly retarget an existing Agent Console draft.
