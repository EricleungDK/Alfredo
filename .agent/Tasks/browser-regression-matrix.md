# Browser regression coverage for the Rust migration

Date: 2026-09-13. Current user authorization explicitly includes browser Playwright
regressions for all issues and functions. This matrix supplements the
[81-issue acceptance inventory](rust-terminal-regression-inventory.md); it does not
replace its 297 source criteria or infer completion from closed issues.

## Execution surfaces

| Suite | What executes | Covered behavior | Limits |
| --- | --- | --- | --- |
| `mission-control/e2e/responsive-layout.pw.ts` | Production React bundle in real Chromium, fixture Tauri IPC | Desktop, compact desktop, tablet and mobile geometry; command palette; review controls; operational detail; inline evidence | Fixture data does not prove backend execution, failure recovery or native Rust UI behavior |
| `mission-control/e2e/localhost-functional.pw.ts` | Real Vite localhost bridge and Python authority in isolated temporary roots | No Tauri global; keyboard repository creation and new-mission entry; page-reload workspace/mission continuity; canonical Agent Console; reduced-motion behavior; page-error check | Does not yet exercise process restart, conversation restoration, inference, scope, plan approval, workers, review or retirement |
| `mission-control/e2e/workspace-mission-journey-entry.prototype.ts` | Throwaway journey prototype in Chromium | Entry, status-label containment, transcript follow, history/draft restoration and command/capability completion | Prototype behavior is not production behavior |
| `alfredo-tui/tests/terminal_smoke.py` | Native Rust process in a real PTY, reconstructed terminal screen | Workspace/mission entry, scope gate, chat interruption, task/worker/plan/review/branch flow, dispatch, saved-state restart | Chromium has no DOM for the native terminal; native coverage remains PTY plus Rust tests |

## Functional acceptance still needed

- Extend page-reload continuity to process restart, mission resume and preserved drafts.
- Browser evidence for request interruption, model disconnect, retry and concurrent work.
- Browser scope/formation and action-receipt behavior across accepted/rejected requests.
- Browser policy/approval, worker results, review/repair, dependencies and retirement.
- Browser-visible timing and responsiveness measurements with qualified real-model cohorts.
- Requirement-by-requirement links to all relevant issue acceptance criteria, including
  failure paths, not merely one passing flow per issue.

These checks remain open. Existing Rust and backend tests provide separate evidence;
none of these browser suites alone proves all functions or launch readiness.

## Current environment

The installed Playwright CLI requested Chromium/headless-shell revision 1228, while
the default cache held revision 1243. Initial browser launch failed before product
assertions. The matching revision was installed in `/tmp/alfredo-playwright-browsers`
without changing the package lock or global browser cache. Use:

```bash
cd mission-control
PLAYWRIGHT_BROWSERS_PATH=/tmp/alfredo-playwright-browsers npm run test:layout
PLAYWRIGHT_BROWSERS_PATH=/tmp/alfredo-playwright-browsers npm run test:browser
PLAYWRIGHT_BROWSERS_PATH=/tmp/alfredo-playwright-browsers npm run test:prototype-journey
```

The initial `test:layout` already completed the production build. Its browser rerun
used the unchanged built bundle directly. Chromium layout: four passed in 6.2 seconds.
The original real-localhost entry test passed in 29.7 seconds. It is now extended to
reload and verify the acknowledged workspace path and mission identity. The first
new assertion incorrectly expected the entry screen's main landmark after reload;
the captured restored screen proved canonical state under `Prompt Workstation` with
an `Agent Console` region. The test now asserts those landmarks plus workspace value
and mission tree identity. Its final rerun passed in 38.4 seconds, including restored workspace value and mission tree identity.

Prototype baseline: three passed, one status-label check timed out at 30 seconds.
An unchanged focused traced rerun passed in 24.4 seconds. Trace shows navigation took
20.182 seconds, including roughly 11.5-second development dependency requests. This
is evidence of a slow test environment, not proof of a fixed layout defect or a stable
full prototype suite. The subsequent full suite passed all four tests in 56.4 seconds. No timeout or geometry assertion was relaxed.


Final evidence: production layout 4/4 (6.2 s), real localhost creation/reload 1/1
(38.4 s), prototype 4/4 (56.4 s). Production build and TypeScript check passed.
Logs: `/tmp/alfredo-browser-layout-verified.log`,
`/tmp/alfredo-browser-reload-final.log`, `/tmp/alfredo-browser-prototype-final.log`,
`/tmp/alfredo-browser-typecheck.log`. The focused prototype trace is retained under
`/tmp/alfredo-prototype-status-trace`. All processes completed. The initial prototype
timeout remains recorded; passing reruns do not prove sustained timing stability.
