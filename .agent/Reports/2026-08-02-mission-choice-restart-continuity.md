# Mission choice, restart continuity, and source-launch diagnosis

**Implemented:** 2026-08-02

**Issue:** [GitHub #58](https://github.com/EricleungDK/Alfredo/issues/58)

**Baseline:** `b6ea5a64ddae0c5fbdabfd60b12c277e639131d4` on `main`

**Related diagnosis:** closed Issue #62 implementation at `1d9b795`

## Outcome

Alfredo now enforces the complete post-workspace journey already introduced across Python, persistent transport, Tauri, and React. A known Coding Workspace stays behind an explicit Resume Mission or Start New Mission gate. Resume binds the exact requested known Mission; Start New creates a distinct identity; and the canonical Starting Location, Coding Workspace, Active Mission, known Missions, revision, and receipts restore from `workspace-sessions.json` across process and desktop restart. Recent-workspace entries remain explicit relaunch intents and cannot retarget the immutable Tauri process binding.

This completion pass fixed the remaining client-side trust gap: React now rejects an acknowledgement unless its Active Mission and Mission catalog contain the exact requested identity. It also added successful Resume and Start New UI coverage, persistent failed-resume recovery coverage, and a real Tauri-to-Python Resume bridge test. Cross-platform test fixtures compare canonical paths.

## Startup diagnosis

The reported source app-open failure reproduced three times at the public launcher with `ALFREDO_GUI_SMOKE=1`: the partial `node_modules` tree had no local Tauri executable, so `npm run desktop` ended with `tauri: command not found`. A clean `npm ci` advanced startup to a second missing prerequisite, Cargo. Neither failure originated in Issue #62 or the Mission-choice state machine.

The development launcher now checks its lockfile-installed local Tauri executable and Cargo before spawning the desktop. Missing dependencies produce an Alfredo preflight failure with `npm ci`; missing Cargo produces the selected Cargo version command. The installed native package path is unchanged and still does not require either development tool.

After installing the declared dependencies and Rust toolchain, the real managed launcher opened the Tauri window and wrote `gui-smoke-ready.json` with the repository's canonical path, Active Mission `agent-issues`, and phase `workspace-ready`. That marker is emitted before React receives an authoritative snapshot, so it was necessary but not sufficient evidence that the rendered workstation was usable.

## Post-completion correction

The Mission Commander then supplied a screenshot stuck on `Connecting to Alfredo` / `Waiting for an authoritative workspace snapshot`. The exact screen reproduced three times from a standalone `npm run dev` browser preview: `__TAURI_INTERNALS__` was absent and five native `invoke` calls rejected before any canonical state could render. A standalone Vite process also prevented the managed native launcher from starting because it already occupied port 1420.

The correction is recorded in local commit `f8a4e21` and in the authoritative [Issue #58 correction](https://github.com/EricleungDK/Alfredo/issues/58#issuecomment-5160236107). The branch has not been pushed.

After stopping the standalone preview, the restored native path exposed a second defect at the first authoritative boundary. The generated all-Missions catalog included the Active Mission, while the snapshot CLI also loaded that Mission as its primary argument. `WorkspaceSnapshotService` therefore received the same Mission twice and rejected startup with `Workspace Mission ids must be unique`.

The corrective implementation now:

- identifies a non-Tauri browser before any native IPC call and renders an actionable desktop-bridge failure instead of waiting indefinitely;
- reconciles an exact Active Mission entry in the generated catalog with the already loaded primary Mission, while still rejecting duplicate catalog ids or mismatched primary paths; and
- describes `npm run dev` as a visual-only preview and keeps the managed launcher as the functional application path.

The native snapshot correction is covered both through the public Python CLI journey and through Tauri's restored binding into the persistent Python backend. The exact current journey returns schema 1, Active Mission `agent-issues`, and four distinct Missions. The browser feedback loop now reports no connecting text, no page errors, and the explicit managed-launch instruction.

## Issue #62 compatibility

`1d9b795` is an ancestor of the baseline and has no GitHub dependency edge to #58. Its production measurement code does not select, persist, or retarget a Mission. One #62 crash-cut test still patched the removed private `AlbertMission._persist` method after the later persistence refactor. The test now injects the same pre-write cut at `Path.replace` for the exact canonical `runtime.json` destination. The public crash-recovery assertion passes; no #62 production behavior or #58 authority boundary changed.

The post-completion correction is also compatible with #62. Catalog reconciliation happens inside the existing authoritative snapshot operation, and browser bridge detection prevents invalid IPC rather than moving or weakening any S0-S9 or R0-R6 measurement boundary. No measurement stage, fixture, rollback gate, or performance claim changed.

## Verification

Focused acceptance evidence:

- Python CLI, restart, persistent Resume/New/failure recovery, and crash cut: **6 passed**.
- `WorkspaceClient` plus React Mission gate: **46 passed**.
- Development preflight for missing Tauri and missing Cargo: **2 passed**.
- TypeScript typecheck: **passed**; Vite production build: **37 modules built**.
- Tauri exact Resume, structured failed Resume plus distinct Start New, and canonical restart restoration: **3 passed** with desktop features.
- Post-completion snapshot correction: **4/4** Mission-selection CLI tests, **36/36** WorkspaceClient tests, the targeted React startup-failure test, the restored Tauri-to-Python snapshot regression, TypeScript typecheck, production build, and exact public current-journey replay **passed**.
- Exact screenshot feedback loop: standalone browser preview no longer renders the connecting state and reports no page errors; it renders the managed desktop instruction instead.
- Rust formatting and `git diff --check`: **passed**.
- Documentation audit: **completed**, reporting 8 pre-existing connections/orphans; standards validation: **A, 95.5%**, with 5 pre-existing low-severity traceability items.
- Independent review: Spec **clean**; Standards findings for missing-Tauri coverage and documentation evidence were corrected. The duplicated explicit persistent envelopes were retained as readable public-protocol fixtures rather than hidden behind a test abstraction.

The repository's full matrices were run on the current Darwin arm64 host even though its documented release/test baseline is Ubuntu 24.04 x64:

| Matrix | Result | Host-specific limitation |
|---|---:|---|
| Python `unittest discover` | 233 pass, 32 fail, 176 error, 8 skip; 449 total | Linux Bubblewrap, `/proc`, `prlimit`, and macOS `/var` aliases |
| Vitest | 99 pass, 149 fail; 248 total in 14 suites | Node 25 localStorage behavior, Linux package target expectations, and `/var` aliases |
| Node performance | 22 pass, 7 fail; 29 total | canonical scratch containment and `/var` aliases |
| Rust desktop | 33 pass, 12 fail; 45 total | Linux sandbox/package assumptions and `/var` aliases |

All focused #58 paths are green on the same host. No live model request or production performance cohort was run or claimed.

## Viewing the result

From the repository root, the managed native source path is:

```bash
cd mission-control
npm ci
cargo --version
cd ..
node mission-control/bin/alfredo.js workstation --agent qwen3-14b
```

The visual-only browser preview is available with:

```bash
cd mission-control
npm run dev
```

It intentionally reports that the native desktop bridge is unavailable and does not provide authoritative workspace data.
