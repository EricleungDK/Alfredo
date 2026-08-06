# Issue #63 Mission Execution Tree review fixes

**Date:** 2026-08-06
**Scope:** Repair the confirmed output, review-action identity, disclosure, Repair semantics, constrained-inspector, and responsive-gate defects in the Mission Execution Tree. This report does not add Issue #64 archival behavior or Issue #65 supervision behavior.

## Implemented contract

- Runner output uses pipe `read1` where available so a flushed short record is observable before child exit. Recorder startup is inside the durable runner failure path; startup failures clear runner identity, record a terminal failure, and close any partial recorder.
- Per-session output stays app-local and transient. It retains at most 128,000 journal bytes, splits valid UTF-8 content into ordered events of at most 16,000 bytes, and pages exact Mission/session reads at 256 events. `complete` is true only after a terminal runner's final retained page. Python, Rust, and TypeScript require the same output phase and preserve exact identity checks; React retains and renders each event as an exact chunk, without manufacturing a newline at an event boundary.
- The WorkspaceClient reports subscription success only after a valid exact response. Typed failures are observable, auto-retry only a bounded number of recoverable times, expose inline retry, retain prior output, and unsubscribe on inspector close/change/unmount/Mission switch.
- Review actions use one exported canonical Mission/session/decision state identity, with visible state keyed by that identity, so concurrent decisions remain independently pending and only the matching decision receives its resulting feedback.
- Parent work has labelled, separate disclosure buttons for pointer/touch expansion while treeitem rows retain keyboard navigation and inspection. Repair lineage now has explicit text plus a distinct non-color shape.
- At 680 px and below the inspector is a focus-contained modal dialog with initial Close focus and Escape/Close restoration. Desktop keeps it inline in Mission Work.
- The production four-viewport Playwright gate now drives the canonical tree, disclosure, exact inspector, review consequence, Evidence Package, constrained dialog, focus return, reduced motion, and page-overflow assertions; retired Workstation Card selectors were removed.

## Evidence recorded during implementation

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_workspace_snapshot -k session_output` — 7 passed, including terminal paging, stale cursors with and without a journal, UTF-8 splitting, live flushed capture, and output-recorder startup failure.
- `cargo fmt -- --check && cargo test session_output` in `mission-control/src-tauri` — 2 passed.
- `npm run typecheck` in `mission-control` — passed.
- `NODE_OPTIONS=--localstorage-file=/private/tmp/alfredo-issue63-fix-localstorage npm test -- --run src/App.test.tsx src/workstation-projection.test.ts src/workspace-client.test.ts` — 212 passed. The added coverage asserts all 257 rendered output events in exact order; session-scoped pending/acknowledged review decisions; stale, rejected, and persistence-reload feedback; and Tab/Shift+Tab containment. Two pre-existing React `act(...)` warnings remain in unrelated App tests.
- `./scripts/apple-container-dev test-layout` — passed all four production Chromium scenarios (desktop, compact desktop, tablet, mobile) after the final frontend corrections. It reuses the persistent guest and cached guest browser, so it bypasses the host macOS Mach-bootstrap restriction without stopping the canonical workstation.
- `bash -n scripts/apple-container-dev` and `git diff --check` — passed.

The broad host suites remain environment-limited rather than product-failing: the Python discovery suite encounters the absent host Bubblewrap binary plus Darwin `/var` versus `/private/var` aliases, and the full Vitest/Rust suites additionally contain Linux-only release fixtures. The focused Issue #63 seams, production build inside the guest layout run, and real four-viewport Chromium gate are green. The persistent `alfredo-dev` bridge remains ready on `127.0.0.1:1420`.

## Review closure

Independent Standards and Spec review identified and the implementation corrected output replay/drop and cross-event newline injection, decision-state leakage including concurrent pending actions, pointer-interactive mobile modal underlay, evidence focus outside an active constrained inspector, competing constrained-dialog scroll regions, and an inaccessible test helper selector. The final targeted review found no actionable findings and confirms the repaired behavior is covered at the production seams.
