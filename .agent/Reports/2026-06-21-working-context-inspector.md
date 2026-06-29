# Working Context Inspector — Completion Evidence

**Issue:** `.agent/issues/04-inspect-and-curate-working-context.md`

**Status:** Complete

## Delivered behavior

- `WorkingContextService` reconstructs typed Workspace Session, Shared Context, unresolved-item, recent-conversation, and deliberate-reference sources for the acknowledged Conversation Scope.
- Recent scoped input is limited to six messages and total included content to 4,000 characters; full Agent Console history is stored and rendered independently.
- Eligible sources support included, pinned, and excluded dispositions in separately revisioned atomic persistence.
- Pinned messages remain available as deliberate references after leaving the recent window.
- Governed Workspace Session and Shared Context sources are required, ineligible, and rejected by the backend with `context-source-ineligible` if a caller bypasses the UI.
- Tauri transports typed inspect/curate contracts. React shows the bounded projection and changes it only after acknowledgement and authoritative reload.

## Acceptance evidence

- Source reconstruction: Python tests assert all five categories, deterministic source ids, governance flags, dispositions, six-message recent window, scope filtering, and character budget.
- Eligible curation: service/CLI tests pin a message and exclude an unresolved Issue Slice, then reconstruct the acknowledged projection after a fresh service process.
- Governance: backend tests reject governed, unknown, and stale curation without changing workspace revision, issue markdown, or history; React exposes no controls for governed sources.
- Bounded/full separation: user-level tests show all eight Agent Console messages while Context Inspector contains only messages 3-8.
- Acknowledgement: a deferred React test keeps the visible source included while Pending and shows excluded only after the acknowledgement triggers reload.
- Restart: the real Rust bridge pins a source, restarts the Python process, restores revision 2, and preserves structured governed rejection.

## Integration evidence — 2026-06-21

- `python3 -m unittest discover -s tests -v`: 94 passed.
- `npm test -- --run`: 30 passed.
- `npm run typecheck`: passed.
- `npm run build`: passed.
- `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check`: passed.
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml`: 10 passed.
- Fresh-runtime smoke: ISS-04 scope acknowledged at workspace revision 2; message source appended; pin and exclude acknowledgements advanced context revision to 3; a separate process restored the pinned message, excluded issue, required governed sources, unchanged workspace revision, and full history.
- Governance smoke: excluding `shared-context:issue-slice:ISS-04` returned structured `context-source-ineligible` and left context revision 3 unchanged.

## Integration defect found and fixed

Parallel smoke reads exposed `AlbertMission._persist` truncating `runtime.json` in place during every load. A concurrent history process observed an empty document. Runtime persistence now writes a unique sibling temporary file and atomically replaces the canonical file. A deterministic paused-write regression passes, the failed history read succeeds, and eight concurrent CLI read processes complete without corruption.

## Known limits

- The source catalog is intentionally initial-release scale: one required Workspace source, one scope-specific Shared Context source, unresolved Issue Slices, scoped conversation messages, and explicit pins.
- Cross-mission source projection will expand with the multi-mission registry delivered by Issue 05; curation governance and bounded-input contracts remain unchanged.
