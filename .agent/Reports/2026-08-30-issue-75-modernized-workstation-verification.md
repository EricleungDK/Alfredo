# Issue #75 — Modernized workstation and fallback verification

**Implemented:** 2026-08-30

**Issue:** [GitHub #75](https://github.com/EricleungDK/Alfredo/issues/75)

**Parent PRD:** [GitHub #56](https://github.com/EricleungDK/Alfredo/issues/56)

**Baseline:** `8e82d87` on `main`

## Outcome

The integrated public-seam verification now exercises Alfredo from an unbound Starting Location through explicit Coding Workspace selection, explicit Mission resume, Wayfinder and Shared Understanding Gate confirmation, governed tracked and Ad Hoc dispatch, Mission Execution Tree inspection, evidence review, supervision/recovery evidence, retained Retirement Record inspection, Activity Journal chronology, and process restart. The tracer uses the real Python CLI/backend rather than a frontend-only fake and proves exact Mission-choice replay before the workstation becomes ready.

The installed Linux release also passes its production package boundary outside the managed filesystem sandbox. It builds the optimized Tauri/AppImage artifact, packs both npm packages, installs only the meta package through an isolated local registry, probes the installed launcher and native version, loads the installed frontend and backend into the selection-required state, and validates the exact artifact hashes. The resulting verified manifest is publishable, but this run did not publish it.

This report does not claim public release completion or production latency. Authenticated public-registry publication, registry-only reinstall, human real-display/accessibility acceptance, and repeated production performance cohorts were not authorized or performed.

## Production-equivalent workstation journey

`mission-control/src/alfredo-release-seam.test.tsx` creates a distinct Starting Location and committed Git Coding Workspace, writes a real tracker, and drives the supported public clients against the Python CLI. It proves:

1. launch begins at `selection-required` with null Coding Workspace and Active Mission;
2. Coding Workspace selection acknowledges only the exact repository;
3. Mission options are discovered, the known Mission is explicitly resumed, and exact correlation replay returns the same receipt;
4. the ready workspace enters Wayfinder Chart mode and opens the pending Shared Understanding Gate;
5. `/wayfinder confirm` opens the gate before governed tracked and Ad Hoc work is dispatched;
6. the Mission Execution Tree and Activity Journal expose durable session/evidence chronology;
7. tracked and Ad Hoc evidence are independently loaded and accepted through the real review transport;
8. a controlled pre-effect crash cut records exact dead owner/process identities without a host-effect receipt, the public `runner-observe` seam recovers the same session once and replays the same receipt, the remounted workstation reruns it, and accepted evidence retires it;
9. tracked, Ad Hoc, and recovered sessions all reach `retired` with retained Retirement Records, and restart restores the same durable chronology.

No UI event, terminal transcript, or fixture prose is promoted into canonical Mission truth. Every state transition used by the journey comes from the backend acknowledgement or a reloaded canonical projection.

## Local Inference authority boundary

Mission Execution Tree now renders exact active and queued Local Inference Lease ids next to the Profile id/version, model, Mission/session attribution, and timings. The projection continues to state that Local Inference schedules compute only: it has no Mission, Queue, review, or acceptance authority. Focused App coverage asserts the exact queued lease `lease:queued-0001` and Profile `worker-v1 · v1` rather than accepting an unattributed inference label.

## Execution compatibility and fallback

The installed provider contract contains thirteen cohorts. Four positive execution cohorts cover both governed effects across both supported protocol generations:

| Effect | Immediately previous protocol | Current protocol |
|---|---|---|
| Local Agent | one-response | streamed effect binding |
| Shell | one-response | streamed effect binding |

The remaining cohorts cover failed execution, timeout cleanup, output limiting, cancellation, provider-free replay, crash-cut normalization to `outcome-unknown`, resource-contract rejection, sandbox-contract rejection, and state-version rejection. Every cohort verifies that Python-owned canonical stores remain unchanged by the shadow verifier.

The same installed-artifact run proves `python_fallback` for both Shell and Local Agent at `selection_boundary: pre-effect`. Fallback is therefore attributable to positive no-effect proof; it cannot occur after request claim, process start, an unresolved receipt, or canonical mutation.

## Performance truth

The performance contract now assigns evidence to separate phase families instead of collapsing responsiveness into one duration:

- launcher: `S1`;
- desktop: `S2`, `S4`;
- React: `S3`, `S8`, `S9`, `R0`, `R4`, `R5`, `R6`;
- backend: `S5`, `S6`, `R2`;
- persistence: `R2`;
- transport: `S5`, `S7`, `R1`, `R3`;
- rendered state: `S8`, `S9`, `R0`, `R5`, `R6`;
- model: load, prompt evaluation, first token, and decoding timings.

The release workflow now runs both the production fixture validator and all four performance contract files before publication. This run exercised contracts and synthetic fixtures only, so it makes no repeatable human-perceptible speed claim. Production cohorts with the exact installed launcher and a human-visible display remain required before such a claim.

## Verification matrix

| Gate | Result |
|---|---|
| Real-backend React release seam | Passed 1/1; complete journey plus restart in about 40 seconds |
| Mission selection and persistence focus | Passed 10/10 |
| Runner recovery and semantic replay focus | Passed 1/1 |
| Packaged shadow contract and fallback focus | Passed |
| Performance fixtures | Passed |
| Performance contracts | Passed 4/4 files |
| TypeScript | Passed |
| Production frontend build | Passed; 39 modules |
| Frontend Vitest | Passed 16/16 files and 321/321 tests |
| Localhost gateway | Passed 23/23 |
| Functional Chromium | Passed 1/1 |
| Responsive Chromium | Passed 4/4 at desktop, compact desktop, tablet, and mobile |
| Rust formatting | Passed |
| Rust full suite | Passed 74 tests; one owner-death subprocess fixture intentionally ignored when not invoked by its parent test |
| Installed release verification | Passed outside the filesystem sandbox, including AppImage, local-registry install, launcher, frontend, backend, thirteen provider cohorts, fallback, and immutable store checks |
| Verified artifact integrity | Passed; `release:check` reports both packages and the provider publishable |
| Complete Python discovery | Ran 797 tests, OK (2 skipped); run as an unprivileged dedicated UID with private fixtures on WSL |

The large App and source-entrypoint files use a file-local 15-second Vitest budget; the installed-startup test has its own 15-second budget, and the release seam uses a separate 75-second integration budget for cold, unprivileged private-fixture execution. No global timeout or assertion waiver was introduced.

WSL exposes its same-UID user manager but denies access to that process's filesystem boundaries. Alfredo continues to fail closed in that ambiguous environment. The representative WSL Python gate therefore runs as an unprivileged dedicated UID with mode-private fixtures, preserving real permission-denial coverage while making the user manager unable to traverse the fixture. The frontend gate uses a root-owned private release fixture for writable package staging, and the integrated seam also passes independently under the dedicated unprivileged UID.

## Explicitly open external acceptance

- Publish only the already verified artifacts through the authenticated registry workflow, then perform a registry-only reinstall smoke. No publication occurred here.
- Perform real-display installed desktop acceptance and human keyboard/screen-reader/accessibility review on the persistent Apple-container workstation or another supported visible workstation.
- Collect repeated production performance cohorts before making any human-perceptible performance statement.

## Principal files

- `mission-control/src/alfredo-release-seam.test.tsx` — real-backend integrated workstation tracer.
- `mission-control/scripts/prepare-release-crash-fixture.py` — deterministic pre-effect dead-owner boundary for public observer recovery and replay.
- `mission-control/src/MissionExecutionTree.tsx` and `src/App.test.tsx` — exact inference lease attribution and non-authority evidence.
- `mission-control/scripts/shadow-provider-contract.js`, `verify-shadow-provider.py`, and their tests — dual-effect, dual-protocol installed compatibility and fallback proof.
- `mission-control/scripts/performance-cohorts.js`, `tests/performance-cohorts.test.js`, and `performance/README.md` — separated phase-family contract and claim boundary.
- `.github/workflows/publish-npm.yml` — protected performance and installed selection-required release gates.
- `mission-control/vite.config.ts` and `src/test-setup.ts` — stable complete frontend gate across Node/jsdom and full worker load.
- `albert_mvp/core.py`, `workspace.py`, and their focused tests — fail-closed retirement proof and non-blocking public live-Shell inspection without weakening effect ownership.
