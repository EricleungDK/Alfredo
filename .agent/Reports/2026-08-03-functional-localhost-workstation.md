# Functional localhost workstation and launch regression diagnosis

**Reported:** 2026-08-03

**Implemented:** 2026-08-03

**Fixed and verified:** 2026-08-03

**Severity:** High — both advertised local viewing paths could stop before a functional workstation

**Status:** Fixed locally; not committed or pushed

## Summary

The supplied browser screenshot was reproducible: standalone Vite rendered `Alfredo workstation unavailable` because it had no Tauri global. A simultaneously running Vite process could also occupy native development's only port and prevent the managed Tauri launcher from starting. The latest two commits did not introduce either failure, and the installed macOS toolchain was complete.

Alfredo now has two independent functional source-development paths. `npm run dev` serves a canonical browser workstation through a development-only authenticated loopback Rust/Python bridge on port 1420. Tauri development uses a gateway-free Vite server on port 1422. A permanent Chromium journey proves the browser path creates a real Git Coding Workspace, creates a Mission through Python authority, and renders its canonical snapshot without Tauri or prototype data.

## Environment and reproduction

- Host observed during diagnosis: Darwin arm64 / macOS 26.5, despite the repository's Ubuntu release baseline.
- Toolchain checks passed: Node 25.2.1, npm 11.6.2, Python 3.14.6, rustc/Cargo 1.97.1, Tauri CLI 2.11.3, Xcode and the macOS SDK.
- `npm run test:browser` reproduced the exact supplied heading four consecutive times before the fix.
- A tagged browser probe showed `hasTauriInternals: false` at a fresh loopback origin.
- A separate browser prototype passed on the same host, proving Chromium itself was healthy; its fabricated data was not used as a fix.
- `lsof` showed a standalone Vite process already listening on `127.0.0.1:1420`, the same strict port used by Tauri's old `beforeDevCommand`.

## Root causes

### 1. Standalone Vite was deliberately fail-closed

Commit `f8a4e218641d4c909fb2c92814e7a6fcf6637a72` introduced the explicit non-Tauri fallback ten commits before the reported head. `src/main.tsx` always created `TauriWorkspaceClient`; in a normal browser, `isTauri()` correctly returned false and the client rendered the unavailable screen before Python was contacted.

The latest two commits were not causal. `04cba04` changed documentation, and `c4d39d6` added two Python validation lines for persisted Wayfinder state. Neither changed Vite, the launcher, Tauri, or `WorkspaceClient`.

### 2. Browser and native development competed for port 1420

Tauri's `beforeDevCommand` ran the same strict `npm run dev` server used by a standalone preview. If the browser preview was already running, native startup could not bind the port. The launcher had no way to make the two valid development surfaces coexist.

### 3. macOS exposes two strings for the same temporary path

After the real browser bridge was connected, the full repository-creation journey found a second cross-platform defect. Vite passed `/var/folders/...` as Starting Location. Python correctly resolved it to `/private/var/folders/...` in the acknowledgement, while the initial Rust launch context retained the alias. React's exact-boundary check rejected the valid result as `invalid-workspace-acknowledgement`.

This alias was macOS-specific, but it was not the cause of the original unavailable screen. Rust now canonicalizes Starting Location before both initial projection and command dispatch, so the UI and Python compare one exact boundary.

## Fix and authority boundary

- `createWorkspaceClient()` selects native Tauri first, an explicitly injected localhost capability second, and the existing fail-closed ordinary-browser client last.
- Vite's `localhost` mode binds only `127.0.0.1:1420`, injects a random immutable per-process capability, and accepts only same-origin loopback `POST /__alfredo/invoke` requests with exact Host, Origin, media type, capability, body size, and three-field envelope.
- The default Starting Location is Alfredo's parent directory rather than the forbidden backend itself, so the advertised create-repository flow can produce a safe sibling repository without a test-only override.
- The gateway fixes backend/runtime/config authority from its own environment. Browser input cannot provide raw argv, backend root, install root, runtime root, or agent configuration.
- A no-desktop Rust binary accepts bounded JSONL `{id, command, args}` requests. Its closed allowlist and strict typed argument decoders match every `TauriWorkspaceClient` command and reuse the same Rust execute/decode functions. Python remains authoritative for validation, persistence, Mission state, expected revisions, permissions, runner execution, and structured failure.
- Tauri now runs gateway-free Vite on `127.0.0.1:1422`. Native IPC retains priority whenever the Tauri global exists.

## Recurrence prevention

The first working bridge implementation passed startup but failed independent design review: one synchronous JSONL request would have put every snapshot, update, status, and cancellation request behind a long `workstation_session_run`; its five-minute transport timeout was also shorter than a legal governed runner. The final implementation closes that gap:

- one shared `WorkstationBridge` preserves the immutable in-process Coding Workspace/Mission binding;
- one bounded runner worker permits one consequential Local Agent run at a time;
- four independently reserved control workers keep status, snapshot, update, and cancellation traffic available;
- a single writer emits atomic correlated responses that may finish out of order;
- the gateway tracks at most 32 in-flight IDs, rejects excess work, and gives the bounded backend runner up to two hours while ordinary requests retain a five-minute deadline;
- the bridge spawns only after Vite has successfully acquired its strict port;
- normal shutdown stops the complete owned Unix process group or Windows task tree;
- on Unix/macOS, unexpected owner-pipe loss immediately kills the bridge's previously verified dedicated Cargo/Rust/Python process group before any blocked runner join; a real subprocess test proves both the bridge fixture and its `/bin/sleep` descendant disappear. Windows owner-loss hardening would require a Job Object and is not claimed by this correction.

The permanent `npm run test:browser` gate runs on port 1421 with a fresh runtime and Starting Location. It asserts that no Tauri global exists, the unavailable screen is absent, selection-required Agent Console is visible, a real repository can be created below the Starting Location, a new `localhost-e2e` Mission is acknowledged, and the canonical Mission snapshot renders without page errors. Because macOS temporary paths use the `/var` alias, the same test also guards the canonical-path correction.

The repository's Ubuntu publication workflow explicitly prebuilds the no-desktop Rust bridge, then runs both `npm run test:gateway` and `npm run test:browser` after installing Chromium, before layout, desktop, packaging, or publication gates. The Playwright gate also carries a cold-Cargo-safe startup deadline for direct developer runs.

## Verification

- Real Chromium canonical journey: **1/1 passed** in 2.7 seconds warm; **1/1 passed** in 29.3 seconds with an isolated empty Cargo target.
- Gateway security, lifecycle, capacity, timeout, and out-of-order correlation: **16/16 passed**.
- TypeScript client plus rendered selection/Mission gate: **52/52 passed** (41 client, 11 gate).
- Rust JSONL parsing, recovery, capacity, blocked-runner/fast-control ordering, and actual owner-loss process-group termination: **6/6 passed**, with one intentionally ignored subprocess fixture entry.
- Rust typed dispatcher allowlist, hostile argument rejection, and real Python selection → Mission → snapshot: **3/3 passed**.
- Python workspace selection, Mission journey, and persistent server: **21/21 passed**.
- TypeScript typecheck: **passed**.
- Vite production build: **37 modules built**.
- Native-feature Cargo check, no-default bridge check, Rust formatting, `git diff --check`, and diagnostic-tag scan: **passed**.
- Live browser verification: the authenticated bridge returned `selection-required` with default Starting Location `/Users/ericleungkawing/AI-Projects`; Chromium rendered Agent Console with the unavailable screen absent and no page errors.
- Managed macOS verification: `node mission-control/bin/alfredo.js workstation --agent qwen3-14b` emitted Alfredo's GUI-ready `selection-required` marker. Browser workstation `1420` and gateway-free native Vite `1422` listened simultaneously, then the managed native process stopped without disturbing `1420`.
- Independent frontend review: no integration defect. Independent Standards/Spec review's lifecycle, concurrency, path, and documentation findings were incorporated.

The known Node 25/jsdom `window.localStorage.clear is not a function` setup failure still prevents one broader release-seam test from entering its test body on this host. It predates and is independent of the localhost transport; focused frontend tests and the real Chromium gate pass.

## Viewing the result

Functional browser development from the repository root:

```bash
cd mission-control
npm run dev
```

Open `http://127.0.0.1:1420`. Keep the terminal open; `Ctrl+C` stops the complete development process tree.

Managed native development remains:

```bash
node mission-control/bin/alfredo.js workstation --agent qwen3-14b
```

The two modes can run together because native development uses port 1422.

## Related documentation

- [Project architecture](../System/project_architecture.md#development-localhost-bridge-boundary)
- [API and command boundaries](../System/api_endpoints.md)
- [Development workflow](../SOP/development_workflow.md#browser-ui)
- [Historical browser-fallback correction](2026-08-02-mission-choice-restart-continuity.md)
