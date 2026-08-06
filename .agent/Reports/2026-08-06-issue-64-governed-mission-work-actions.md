# Issue #64 Governed Mission Work actions

**Date:** 2026-08-06
**Scope:** Expose cancellation-adjacent retention, blocker resolution guidance, evidence review/repair continuity, and completed-history archival as consequence-explicit Mission Work actions without changing Python's canonical authority.

## Implemented contract

- `runtime.json` now retains a validated `archived_issue_ids` set. `issue-archive` and `issue-restore` are correlated, expected-revision Workstation actions that atomically record their marker and timeline event. Archive accepts evidence-complete/pr-ready work and tracker-merged completed history. They never delete the Issue Slice, nested Local Agent sessions, Evidence Packages, Activity Journal links, or inspection identity.
- The Mission Execution Tree groups archived completed subtrees beneath `Archived work`. The original Issue Slice remains inspectable there and exposes only its typed restore action; active completed work exposes its typed archive action. Both controls state the exact retained-history consequence before submit and show a success only after the acknowledgement.
- Repair is still one canonical Python action. Its Mission Work control previews the inherited goal, acceptance, paths, command policy, evidence requirements, assigned Local Agent, and review reason before launch, without creating another task packet or session.
- Blocker recommendations project a dependency's rationale, proposed accepted boundary, actor, and the exact fail-closed consequence. Core dependency truth now requires a `pr-ready` or complete reviewed blocker; approval or follow-up creation alone never unblocks the original Issue Slice.
- Archive writes use the runtime lock, and generic persistence adopts the latest authoritative archive set. A stale mission instance therefore cannot recreate an archive entry after a separate restore.

## Verification evidence

- Focused Python tests: archive/restore plus stale persistence, tracker-merged archive, CLI archive contract, repair preview, and approved-follow-up dependency regression — **5 passed**.
- Persistent Apple container: `src/workstation-projection.test.ts` plus `src/App.test.tsx` — **172 passed**; targeted visual/projection checks — **34 passed**; `npm run test:gateway` with the container launch-location override removed — **23 passed**; `npm run typecheck` and `npm run build` — passed.
- Persistent Apple container `./scripts/apple-container-dev test-layout` — **4/4** production Chromium viewports passed.
- Host `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` — passed.

The broad Python and Rust bridge suites remain blocked by the existing host/container sandbox limits: macOS lacks Bubblewrap and the nested Apple container cannot mount `/proc` through Bubblewrap. Broad frontend/release fixtures are likewise platform-bound (the persistent guest is Linux arm64 while the release fixture supports Linux x64; the host is macOS and cannot build an AppImage). These are not Issue #64 failures. The targeted canonical, projection, rendered, and four-viewport production checks above cover the new behavior.

## Review closure

Independent Standards and Spec review initially found three gaps: tracker-merged archive eligibility, imprecise repair consequence copy, and missing rendered repair-preview/restore coverage. The implementation now accepts a tracker-merged archive at the same canonical boundary, promises one inherited canonical repair rather than an ambiguous retry, and proves the rendered repair packet plus restore request/acknowledgement. The reviewers' follow-up confirmation is recorded in the active orchestration context with this handoff.
