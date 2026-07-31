# Alfredo desktop Rust performance research

**Investigated:** 2026-07-29
**Question:** Does the current evidence show that Rust improves Alfredo desktop startup or representative rendered workflows?
**Source state:** `main` at `91fb98c9e93f4a3a913cae5acfcf51eeef974f6e`, with a large pre-existing dirty working tree preserved
**Scope:** production-path measurement and planning evidence only; no production backend migration or permanent benchmark instrumentation was implemented

## Answer

No supported Rust-versus-Python desktop performance result exists yet.

Alfredo has a real production path with an npm launcher, installed AppImage, Tauri/WebKit window, production React bundle, persistent Python authority process, and canonical Workspace Snapshot. It also has a bounded Rust prototype. The two do **not** perform equivalent work:

- the current Python path loads and validates canonical Mission, Workspace, persistence, policy, and rendered workstation state;
- the standalone Rust benchmark parses a small embedded snapshot, applies two in-memory prototype transitions, serializes the result, and separately times a prototype sidecar write;
- the Rust shadow GUI surrounds each in-memory Rust transition with **two production Python snapshots** and performs no canonical write.

The Rust timings therefore establish only that the disposable reducer is inexpensive. They do not establish a faster desktop, a faster authoritative action, or even the likely size of either improvement.

Fresh measurements of the exact locally verified Python release make the scale clearer:

- the supported installed `alfredo` entrypoint produced a matching WSLg top-level window in **552.346 ms p50 / 564.079 ms p95** under the process-cold approximation and **536.839 / 546.415 ms** warm;
- its first successful canonical Workspace Snapshot marker arrived in **1,293.516 / 1,316.627 ms** process-cold approximation and **1,274.870 / 1,292.196 ms** warm;
- the exact installed Python backend's first persistent ready snapshot was about **100.090 / 112.753 ms** process-cold approximation, while a warm persistent snapshot was **7.348 / 8.788 ms**;
- warm Queue and Review projections were **7.268 / 8.446 ms** and **7.946 / 9.110 ms**; and
- capability discovery was **20.099 / 23.637 ms** warm.

The readiness marker is an early backend-ready proxy, not a React usable-paint marker, so it cannot satisfy the full rendered-workflow claim by itself. Even so, the whole first Python snapshot is only about 0.10 seconds of a roughly 1.29-second supported-entrypoint proxy. Replacing that stage with zero work—an impossible best case—would bound the startup saving near eight percent. Warm Python projection work remains single-digit milliseconds. Model-bound reference paths remain seconds to tens of seconds and were intentionally excluded from the desktop cohorts. [Existing model/control boundaries](2026-07-23-alfredo-architecture-performance-baseline.md#fresh-performance-measurements)

The architecture decision should treat “Rust improves Alfredo desktop performance” as **unproven** until the same production UI, fixture, contract, durable effects, and rollback invariants run through both authorities with end-to-end monotonic stage marks.

## Evidence boundary

This investigation used only repository-owned documentation, source, build scripts, tests, existing artifacts, and a prebuilt local prototype binary.

The current authority split is explicit: React renders acknowledged projections, Tauri transports typed requests, and Python owns accepted Mission state and policy. [Architecture](../System/project_architecture.md#overview) The production React entry mounts `App` with `TauriWorkspaceClient`; it is not the Rust prototype entry. [Production React entry](../../mission-control/src/main.tsx#L1-L9) The normal Tauri build registers the Python-backed command family, while `rust_orchestrator_prototype` exists only under its prototype feature. [Tauri command registration](../../mission-control/src-tauri/src/lib.rs#L2462-L2529)

The installed release artifact found locally is a verified production-AppImage set from 2026-07-13, not a fresh build of this dirty 2026-07-29 source. Its staged AppImage is 77,761,016 bytes with SHA-256 `3faec58bc4e4a0b1c825cb58a3ec5475e5daac36bb0c839e0699ae6ddf006be2`; that digest matches its `desktop.json`. The publishable tarball manifest identifies the set as `production-appimage`, but the manifest records the tarball digest rather than the inner AppImage digest. [Verified release manifest](../../mission-control/release/out/verified/manifest.json) [Desktop adapter manifest](../../mission-control/release/out/alfredo-agent-linux-x64-gnu/desktop.json)

The two exact verified tarballs were installed together into an isolated `/tmp` npm prefix with an isolated npm cache. `alfredo --version` returned `Alfredo 0.1.0`, the installed native binary returned `Alfredo Desktop 0.1.0`, and its SHA-256 matched `desktop.json`. This historical release is valid as a reproducible **Python production-path baseline**. It is not a same-source Rust comparison, and the current GUI readiness marker ends before React renders the canonical workstation, so its result is reported as an early backend-ready proxy rather than a current usable-desktop distribution.

## What the current paths actually measure

| Path | Production-equivalent portion | Missing or extra work | Comparison status |
|---|---|---|---|
| Installed `alfredo` → AppImage → Tauri → React → Python snapshot | Real launcher, package-integrity checks, native shell, production frontend, typed bridge, Python authority | Existing marker is written after Python snapshot decode but before the ready React commit; the release verifier polls it every 200 ms | Valid correctness path, not currently a precise usable-desktop timer |
| Persistent Python `workspace-snapshot` | Real public transport and authority service | Excludes launcher, AppImage, WebKit, React, rendered paint, and secondary hydration | Valid backend component baseline only |
| Rust standalone `--review-json` | Real compiled Rust prototype parser/reducer/serializer and prototype atomic sidecar helper | No production locks, stores, audit reconciliation, runner, evidence, review, CLI parity, Tauri/WebKit/React, or canonical mutation | Directional microbenchmark only |
| Rust shadow GUI command | Real production React/Tauri shell plus live Python before/after verification around the Rust reducer | Adds two Python snapshots per action and performs zero Rust canonical writes | Rollback/feasibility evidence, not a Rust performance candidate |

The launcher itself can dominate or distort a backend-language comparison. Before spawning the desktop it reads configuration, hashes the full AppImage during adapter resolution, starts native `--version`, Python `--help`, Bubblewrap, and optional Ollama probes, records launch context, and only then spawns the native child. [Adapter hashing](../../mission-control/bin/desktop-adapter.js#L59-L72) [Adapter validation](../../mission-control/bin/desktop-adapter.js#L152-L203) [Launcher preflight](../../mission-control/bin/alfredo.js#L140-L179) [Desktop spawn](../../mission-control/bin/alfredo.js#L798-L830)

The Python process does not start with Tauri. The first Python-backed bridge command lazily creates `python3 -m albert_mvp.server`, retains it for warm requests, and retries once after a broken request. [Persistent backend lifecycle](../../mission-control/src-tauri/src/lib.rs#L169-L237) [Lazy reuse and retry](../../mission-control/src-tauri/src/lib.rs#L253-L289)

React starts the snapshot, launch-context, capability, console-history, Working Context, Queue, and Shell loads through separate effects. Only the Workspace Snapshot gates replacement of the loading screen with `CommandDeck`; capability/history hydration may still be in flight. [Snapshot connection](../../mission-control/src/App.tsx#L712-L720) [Concurrent secondary loads](../../mission-control/src/App.tsx#L726-L835) [Loading-to-workstation render](../../mission-control/src/App.tsx#L2093-L2117)

## Fresh production Python measurements

### Host, fixture, and clocks

The fresh production-path samples ran sequentially on Ubuntu 24.04.3 under WSL2, kernel `6.6.114.1-microsoft-standard-WSL2`, Intel i5-14400F with 16 logical CPUs, Node 24.11.0, Python 3.12.3, and WSLg. No model was invoked. All durations used monotonic clocks:

- Node `performance.now()` around launcher/AppImage process spawn, WSLg window observation, and readiness-marker observation;
- Python `time.perf_counter_ns()` around exact installed backend processes and correlated persistent requests.

The immutable fixture was a `/tmp` Coding Workspace with one `ready-for-agent` AFK Issue Slice, one Active Mission, empty Queue/review/runtime state, and the exact installed default agent registry. Fixture hashes were:

```text
PRD.md        bd674377a29e33d02561b09929f6941ef02ecacb1d4bc8703047dbd6b3f831f3
01-ready.md   b3b3a0e4543e54a37621358b0d5c4fc3eb96d5cef884939ebd080e5c5798de44
```

The first validation command returned schema version 1, Workspace Session status `ready`, Active Mission `issues`, and the expected one-slice Mission board. The desktop harness required the WSLg top-level-window PID and `gui-smoke-ready.json` PID to match for every sample, required the exact Coding Workspace path and backend-ready status, opened one desktop at a time, and terminated only that detached test process group after both observations. There were no failed or discarded samples in the final cohorts.

The desktop harness SHA-256 was `c49204a1c34ed44e8e840198573963f4b8cf6438e4116d5230de68e421040ebf`; the backend harness SHA-256 was `097302c2e8baf1a2425cb713a9f75babb292a9b45e229582e38649169f485117`. Both were temporary measurement shells under `/tmp`; they changed no repository source or canonical Alfredo runtime.

“Cold” below means a **process-cold approximation**: a fresh runtime root plus `POSIX_FADV_DONTNEED` for the AppImage, installed launcher/adapter metadata, and installed bundled Python backend before each sample. It does not evict system libraries or the OS-wide cache and is not a cold boot. “Warm” reuses one initialized runtime root per cohort with no file-cache advice, but deliberately starts fresh launcher, AppImage/Tauri/WebKit, React, and Python processes for each desktop sample. Persistent-backend cohorts keep one Python server alive and discard its first request.

### Installed launcher and AppImage

The desktop harness polled the WSLg log from the exact pre-spawn byte offset every 5 ms and recorded the first matching `ClientGetAppidReq ... appId:Alfredo-desktop` event. It separately polled the Tauri smoke marker every 5 ms. That marker is written after the first Python Workspace Snapshot returns and Rust validates it, before React receives and paints the canonical result.

| Path and endpoint | Condition | n | p50 | p95 | Range |
|---|---|---:|---:|---:|---:|
| Installed launcher preflight dry-run → validated native launch plan | process-cold approximation | 20 | 90.775 ms | 97.054 ms | 87.806–97.799 ms |
| Installed launcher preflight dry-run → validated native launch plan | warm | 20 | 78.721 ms | 82.844 ms | 77.306–83.244 ms |
| Direct AppImage spawn → matching top-level window | process-cold approximation | 20 | 227.592 ms | 247.248 ms | 215.386–249.666 ms |
| Direct AppImage spawn → backend-ready marker | process-cold approximation | 20 | 973.981 ms | 1,011.693 ms | 959.304–1,024.100 ms |
| Direct AppImage visible window → backend-ready marker | process-cold approximation | 20 | 748.116 ms | 765.996 ms | 731.864–793.066 ms |
| Direct AppImage spawn → matching top-level window | warm | 20 | 215.427 ms | 221.580 ms | 205.963–227.897 ms |
| Direct AppImage spawn → backend-ready marker | warm | 20 | 965.652 ms | 981.658 ms | 949.099–989.738 ms |
| Direct AppImage visible window → backend-ready marker | warm | 20 | 748.100 ms | 765.753 ms | 736.101–768.297 ms |
| Supported installed `alfredo` spawn → matching top-level window | process-cold approximation | 20 | 552.346 ms | 564.079 ms | 539.388–580.844 ms |
| Supported installed `alfredo` spawn → backend-ready marker | process-cold approximation | 20 | 1,293.516 ms | 1,316.627 ms | 1,275.022–1,324.300 ms |
| Supported visible window → backend-ready marker | process-cold approximation | 20 | 743.165 ms | 757.128 ms | 731.931–758.166 ms |
| Supported installed `alfredo` spawn → matching top-level window | warm | 20 | 536.839 ms | 546.415 ms | 520.510–547.218 ms |
| Supported installed `alfredo` spawn → backend-ready marker | warm | 20 | 1,274.870 ms | 1,292.196 ms | 1,259.381–1,294.586 ms |
| Supported visible window → backend-ready marker | warm | 20 | 738.031 ms | 748.171 ms | 726.303–751.768 ms |

Interpretation:

- The supported Python product opens a WSLg top-level window in about 0.54–0.55 seconds and reaches the early backend-ready proxy in about 1.27–1.29 seconds on this host.
- The process-cold approximation changed the supported readiness p50 by only 18.646 ms and the direct-AppImage readiness p50 by 8.329 ms. This does not prove a true OS-cold result; it does show that the controlled files were not the dominant difference in these cohorts.
- The supported entrypoint reaches the window about 0.32 seconds later than direct AppImage spawn. The standalone preflight dry-run is about 0.08–0.09 seconds. Independent cohorts cannot be subtracted as one causal trace, and the supported path additionally performs nested native probing/process launch, so the remaining difference is launcher/package-process overhead—not backend-language time.
- About 0.74 seconds elapses from the first matching top-level window to the early backend-ready marker under both direct and supported launches. This interval contains WebKit/frontend boot, React scheduling, Tauri invoke, lazy Python-server creation, snapshot work, serialization/bridge decode, and possible contention with concurrent hydration. It is not a Python-authority timer and ends before the usable React paint.

### Exact installed Python backend

The backend harness executed from the exact installed `alfredo-agent/bundled-backend` directory with `PYTHONHOME` and `PYTHONPATH` removed, matching the corrected AppImage child-process boundary. It validated every snapshot's schema, ready Workspace Session, and Active Mission and validated schema version 1 on every secondary projection.

| Boundary | Condition | n | p50 | p95 | Range |
|---|---|---:|---:|---:|---:|
| Fresh one-process CLI → decoded ready Workspace Snapshot | process-cold approximation | 20 | 103.808 ms | 107.059 ms | 100.932–108.243 ms |
| Fresh one-process CLI → decoded ready Workspace Snapshot | warm files, fresh process/runtime | 20 | 103.134 ms | 108.871 ms | 98.950–110.683 ms |
| Spawn persistent server → first decoded ready Workspace Snapshot | process-cold approximation | 20 | 100.090 ms | 112.753 ms | 96.070–116.555 ms |
| Spawn persistent server → first decoded ready Workspace Snapshot | warm files, fresh process/runtime | 20 | 103.928 ms | 121.521 ms | 92.116–133.729 ms |
| Fresh one-process CLI → capability catalog | process-cold approximation | 20 | 129.828 ms | 133.259 ms | 123.274–133.471 ms |
| Fresh one-process CLI → capability catalog | warm files, fresh process/runtime | 20 | 125.906 ms | 130.926 ms | 123.174–132.697 ms |
| Warm persistent Workspace Snapshot request/response | first discarded | 100 | 7.348 ms | 8.788 ms | 6.500–13.909 ms |
| Warm persistent capability request/response | first discarded | 100 | 20.099 ms | 23.637 ms | 18.057–27.305 ms |
| Warm persistent Queue projection request/response | first discarded | 100 | 7.268 ms | 8.446 ms | 6.383–15.382 ms |
| Warm persistent Review projection request/response | first discarded | 100 | 7.946 ms | 9.110 ms | 6.498–17.003 ms |

The first complete Python snapshot is about 100 ms p50, while direct AppImage visible-to-marker is about 748 ms and supported-entrypoint spawn-to-marker is about 1,294 ms. Because these are independent stage cohorts and React starts secondary loads concurrently, subtraction is only a scale bound: even eliminating the entire first Python snapshot would save at most roughly 0.10 seconds, around eight percent of the supported early-readiness proxy. A real Rust candidate would still perform equivalent persistence, validation, serialization, failure mapping, and audit work, so its achievable saving must be smaller.

Warm Python read projections are 7–8 ms p50 except capability discovery at about 20 ms. The prior public Queue-mutation baseline was about 12 ms p50, also without rendered scheduling. A backend-language replacement therefore has no demonstrated route to a human-perceptible warm rendered improvement; only paired native `R0→R5` measurements could establish one.

### Raw desktop samples

All values are milliseconds in execution order; percentiles above use nearest rank.

<details>
<summary>Show the 20-sample desktop cohorts</summary>

```text
launcher preflight process-cold:
97.054, 91.311, 88.163, 92.005, 89.303, 90.775, 90.533, 91.182, 91.988, 97.799, 88.914, 90.393, 88.782, 91.763, 91.241, 87.806, 93.749, 88.420, 88.269, 92.973

launcher preflight warm:
83.244, 82.844, 79.403, 79.029, 78.279, 78.529, 77.672, 79.044, 78.721, 78.487, 79.301, 81.604, 78.528, 78.003, 80.518, 77.306, 77.621, 80.808, 78.244, 79.418

direct AppImage process-cold window:
231.034, 233.526, 228.333, 226.886, 227.329, 227.592, 232.749, 222.025, 249.666, 233.267, 228.253, 225.643, 217.334, 231.359, 220.847, 222.757, 215.386, 226.803, 237.921, 247.248

direct AppImage process-cold backend-ready:
1024.100, 977.545, 987.130, 971.742, 993.325, 979.459, 973.981, 982.351, 1011.693, 965.132, 971.778, 970.841, 972.022, 984.913, 959.304, 970.873, 962.209, 970.034, 1003.420, 995.615

direct AppImage warm window:
205.963, 215.427, 212.915, 210.969, 215.463, 211.034, 221.580, 219.960, 227.897, 216.741, 215.478, 217.241, 211.190, 221.441, 213.699, 211.918, 215.905, 216.155, 211.582, 206.068

direct AppImage warm backend-ready:
953.824, 965.788, 955.227, 954.646, 951.564, 960.244, 969.680, 965.178, 971.158, 967.461, 961.257, 976.556, 965.652, 989.738, 971.041, 954.597, 981.658, 966.050, 966.380, 949.099

supported entrypoint process-cold window:
543.656, 580.844, 552.111, 547.359, 553.289, 541.830, 564.079, 552.346, 553.443, 547.778, 558.460, 547.541, 539.388, 543.090, 542.562, 552.743, 552.395, 556.586, 562.775, 557.401

supported entrypoint process-cold backend-ready:
1287.263, 1324.300, 1294.893, 1293.516, 1289.514, 1285.665, 1306.076, 1309.474, 1290.249, 1287.219, 1316.627, 1291.764, 1287.208, 1275.022, 1283.484, 1300.774, 1299.370, 1296.176, 1298.364, 1300.566

supported entrypoint warm window:
539.775, 526.953, 522.697, 532.276, 546.024, 520.510, 543.481, 541.573, 540.482, 547.218, 533.078, 536.522, 546.415, 536.839, 538.982, 537.177, 538.470, 531.435, 532.137, 536.576

supported entrypoint warm backend-ready:
1283.432, 1269.647, 1260.504, 1268.327, 1292.196, 1265.737, 1280.295, 1273.640, 1281.156, 1284.573, 1259.381, 1272.761, 1294.586, 1274.870, 1279.280, 1284.128, 1280.805, 1268.422, 1267.007, 1288.343
```

</details>

## Complementary 2026-07-23 Python measurements

The 2026-07-23 dirty-source baseline complements the exact-release measurements above with mutating control-plane and model-bound paths. Its report used `time.perf_counter()`, fresh temporary fixtures, nearest-rank p95, and retained only summarized results; the temporary harnesses and raw sample lists were removed. Consequently these values are reference summaries, not raw data that can be independently recomputed from this repository.

| Boundary | n | p50 | p95 | Important exclusion |
|---|---:|---:|---:|---|
| Fresh one-process Python `workspace-snapshot` to decoded ready snapshot | 20 | 185.117 ms | 190.416 ms | No launcher, native shell, WebKit, React, or visible paint |
| Warm persistent Python snapshot | 100 | 9.155 ms | 11.847 ms | No rendered UI |
| Public Queue approval request to durable queued acknowledgement | 20 | 12.296 ms | 14.195 ms | No React event/render timing |
| Approval request start to persisted `runner_started_at` | 20 | 85.377 ms | 87.011 ms | No model first token or rendered visibility |
| Public synthetic goal through fake Evidence Package and accepted review | 20 | 210.729 ms | 213.277 ms | No useful model work or human review time |

Source: [control-plane table and protocol cross-check](2026-07-23-alfredo-architecture-performance-baseline.md#control-plane-measurements). That report explicitly identifies desktop readiness and the rendered dispatch gap as unmeasured. [Caveats](2026-07-23-alfredo-architecture-performance-baseline.md#caveats-and-unresolved-evidence)

## Fresh bounded Rust measurements

The existing prebuilt binary was newer than both current prototype sources and was executed directly, so no build output or source file changed:

```text
source main.rs mtime:  2026-07-29 16:32:05 +02:00
source model.rs mtime: 2026-07-29 16:32:05 +02:00
binary mtime:          2026-07-29 16:32:49 +02:00
binary bytes:          817848
binary SHA-256:        a9325cc786a690768ee92337a89b854d90710fe82872014f96eae50b87c14a0c
```

Command:

```bash
mission-control/src-tauri/prototypes/rust-orchestrator-slice/target/release/alfredo-rust-orchestrator-slice-prototype --review-json
```

Six valid runs were executed sequentially on Ubuntu 24.04.3 under WSL2, Linux `6.6.114.1-microsoft-standard-WSL2`, Intel i5-14400F with 16 logical CPUs, and Rust 1.96.0. Every run reported all ten contract scenarios passing and the rollback proof passing.

Each row below is one raw run summary emitted by the binary. `Reducer p50/p95` summarize 2,000 internal samples of parse schema-v1 fixture → form Ad Hoc route → queue one session → serialize. `Sidecar p50/p95` summarize 100 internal `create → write → sync_all → rename` samples. Percentiles use the prototype's nearest-rank implementation. [Benchmark source](../../mission-control/src-tauri/prototypes/rust-orchestrator-slice/src/main.rs#L395-L478)

| Run | Reducer p50 | Reducer p95 | Sidecar p50 | Sidecar p95 | Scenarios | Rollback |
|---:|---:|---:|---:|---:|---|---|
| 1 | 0.002422 ms | 0.002618 ms | 1.080097 ms | 1.487178 ms | pass | pass |
| 2 | 0.002439 ms | 0.002541 ms | 0.930037 ms | 1.246039 ms | pass | pass |
| 3 | 0.002400 ms | 0.002495 ms | 0.978109 ms | 1.242020 ms | pass | pass |
| 4 | 0.002428 ms | 0.002512 ms | 0.978224 ms | 1.157379 ms | pass | pass |
| 5 | 0.002407 ms | 0.002492 ms | 1.035457 ms | 1.390068 ms | pass | pass |
| 6 | 0.002477 ms | 0.002579 ms | 0.948814 ms | 1.167144 ms | pass | pass |

These values must not be divided into, subtracted from, or otherwise compared numerically with the Python production values. The prototype source itself states that the loop omits Python's production locks, stores, audit reconciliation, sandbox, runner, evidence, review, CLI parity, and migrations. [Prototype interpretation](../../mission-control/src-tauri/prototypes/rust-orchestrator-slice/src/main.rs#L481-L502)

The shadow GUI's displayed `elapsed_micros` is also not a reducer timer: its stopwatch starts before a Python snapshot, includes the Rust transition, then includes another Python snapshot and equality check. [Shadow command timing](../../mission-control/src-tauri/src/lib.rs#L2358-L2433) React renders that combined value after the invoke returns. [Shadow React invoke](../../mission-control/src/prototypes/RustOrchestratorGuiPrototype.tsx#L191-L219)

## Production-equivalent benchmark fixture

One immutable fixture family should feed both variants. Fixture generation and installation happen outside timed intervals.

### Artifact and host

- Build both variants from the same committed source revision with `npm run build` and release Cargo settings.
- Package both through the same AppImage build path, install through the same exact local tarball layout, and launch through the same `alfredo` entrypoint. The only variant is the bounded authority module selected at build time.
- Record source revision, dirty-state prohibition, package/AppImage/binary SHA-256, Rust/Python/Node versions, kernel, native Ubuntu versus WSL, CPU governor/power mode, display server/compositor, monitor scale/refresh, filesystem type, and Ollama reachability.
- Use an ext4 temporary workspace for the primary Ubuntu result. If WSL `/mnt/c` matters, report it as a separate matrix cell, never mixed into one distribution.
- Do not call a sample “cold boot.” Use **process-cold** for a fresh launcher/Tauri/Python process with an isolated runtime root, and **process-warm** for repeated actions within one already-ready desktop. OS page cache is uncontrolled unless a separately approved privileged experiment resets it.

### Canonical state

Use versioned, deterministic copies of:

1. **Minimal ready fixture:** one Coding Workspace, one ready Active Mission, one Issue Slice, empty Agent Console, empty Queue, current default agent registry.
2. **Representative warm fixture:** the same identities plus fixed-size Mission catalog, Issue Slice graph, Agent Console history, Working Context curation, Activity Journal, and Queue history. Persist the exact byte counts and item counts with the samples.
3. **Pending Ad Hoc fixture:** one exact pending Ad Hoc Delegation with a deterministic fake eligible worker, fixed goal, acceptance criteria, allowed paths, command policy, expected revisions, and origin message. Model inference and test execution remain outside the rendered governance-action benchmark.

Every mutating sample starts from a fresh byte-identical copy. Reusing one runtime would change revisions, session counts, journal length, filesystem cache, and correlation history across samples.

### Rendered workflow matrix

Measure more than one rendered path, but keep every endpoint narrow enough to attribute:

| Workflow | Timed endpoint | Why it belongs | Fixture/source |
|---|---|---|---|
| Startup | Installed `alfredo` spawn → usable and hydrated paints | Only end-to-end startup result | Minimal and representative fixtures above |
| Queue defer | Trusted decision click → canonical deferred item painted | Pure durable governance action without runner/model work | Pending Ad Hoc fixture |
| Queue approve | Trusted approval click → one exact canonical session painted; runner claim separate | Representative authority mutation, receipt, reload, and deferred dispatch | Pending Ad Hoc fixture and current rendered ordering |
| Evidence open | Trusted evidence click → bounded artifact content or explicit failure painted | Representative bounded read, validation, bridge, and React content render | Fixed UTF-8 artifact at small and 128,000-byte cap-adjacent sizes; current loader renders a loading state before the bounded response. [Artifact load workflow](../../mission-control/src/App.tsx#L2859-L2913) |
| Review accept | Trusted accept click → canonical Complete/PR-ready state painted | Representative revision-guarded mutation plus canonical reload and secondary review refresh | Fixed complete Evidence Package; current handler reloads the snapshot before displaying acknowledgement. [Review workflow](../../mission-control/src/App.tsx#L1552-L1621) |
| Optimistic prompt echo | Trusted submit event → optimistic user turn painted | Client-only negative control: a backend-language change should not materially improve it | Fixed short prompt; React registers the optimistic message before awaiting persistence. [Optimistic message path](../../mission-control/src/App.tsx#L1130-L1159) |

The existing responsive fixture provides a useful rich schema-v1 projection, artifact reference, running session, ready issue, and review controls, but it installs a mocked Tauri bridge; reuse its data shape, not its browser timing, for the native fixture. [Rich layout fixture](../../mission-control/e2e/responsive-layout.pw.ts#L5-L120) The release-seam test similarly provides deterministic fake controller/worker configuration and real Python command semantics, but it uses jsdom and fresh CLI subprocesses rather than native WebKit plus the persistent Tauri backend. [Release-seam fixture](../../mission-control/src/alfredo-release-seam.test.tsx#L24-L120)

### Authority variants

**Python-authority variant:** the current installed production path unchanged.

**Bounded Rust-candidate startup variant:** replace only the `workspace_snapshot` authority implementation behind the existing Tauri command. Preserve the launcher, AppImage, window, React bundle, TypeScript contract, schema version, structured failures, selected Workspace/Mission, and all secondary Python-backed loads. The candidate must read a shadow copy or an explicitly versioned compatibility store until correctness and rollback gates pass.

**Bounded Rust-candidate rendered-action variant:** implement only pending Ad Hoc Queue decision → durable queued session → canonical reload behind the existing `workspace_queue_decision` and `workspace_snapshot` contracts. It must perform the same lock, expected-revision, correlation, receipt, persistence, audit-reconciliation, and deferred-runner semantics before its timing can be compared. The current prototype is useful model code for a candidate, but it is not this candidate: it uses a different v2 state/action vocabulary and an in-memory `sessions` vector. [Prototype action/state model](../../mission-control/src-tauri/prototypes/rust-orchestrator-slice/src/model.rs#L71-L133) [Prototype dispatch](../../mission-control/src-tauri/prototypes/rust-orchestrator-slice/src/model.rs#L434-L492)

## Exact monotonic stages

Record a single correlation id through all stages. Durations must come from monotonic clocks. The outer harness owns end-to-end elapsed time; each process records local ordered segments. Do not compare UTC timestamps or subtract timestamps from unrelated clock origins.

### Startup stages

| Stage | Exact start | Exact end |
|---|---|---|
| `S0 total launch` | Harness immediately before spawning the exact installed `alfredo` path | Harness receives `S8` ready-paint marker |
| `S1 launcher` | First launcher statement after Node module initialization | Immediately before `spawn(desktopCommand, ...)` |
| `S2 native shell` | First statement in Rust `main()` before `albert_mission_control::run()` | Production webview reports the first frontend-entry mark |
| `S3 React boot paint` | First statement in `main.tsx` | Second `requestAnimationFrame` after `.boot-screen` is committed |
| `S4 snapshot bridge` | Immediately before React invokes `workspace_snapshot` | Rust command handler entry |
| `S5 backend/process` | Rust handler entry | Correlated Python server request accepted; split child spawn from warm reuse |
| `S6 Python authority` | Python begins `WorkspaceSnapshotService.snapshot()` for that request | Python has encoded a schema-v1 success or structured failure |
| `S7 bridge decode` | Python response is readable in Rust | Rust has decoded/validated the snapshot and returned the Tauri response |
| `S8 usable desktop` | React receives the ready/empty snapshot | Second animation frame after `main[aria-label="Prompt Workstation"]` is committed with the exact Workspace Session and Active Mission from the fixture |
| `S9 hydrated desktop` | `S8` | Capability catalog, Agent Console history, Working Context, Queue, and Shell loads have each reached success or an explicit rendered failure |

`S8`, not the current GUI smoke marker, is the primary usable-desktop endpoint. The current marker is written inside the Rust `workspace_snapshot` handler immediately after `execute_snapshot`, before React receives the result. [Current marker boundary](../../mission-control/src-tauri/src/lib.rs#L2094-L2135) The release verifier then checks for that file at 200 ms intervals, which is sufficient for a 45-second smoke but too coarse for a latency distribution. [Release smoke polling](../../mission-control/scripts/verify-installed-release.js#L345-L424)

Report `S0→S8` and `S0→S9` separately. A backend rewrite can affect `S5–S7`; it cannot take credit for unchanged launcher hashing, AppImage extraction, WebKit initialization, or React painting.

### Rendered Queue-decision stages

Use the pending Ad Hoc fixture twice: one **defer** sample for a pure governance action, and one **approve** sample for durable session creation. Do not include model execution.

| Stage | Exact start | Exact end |
|---|---|---|
| `R0 visible intent` | Trusted button event handler entry | Pending action card is committed on the next animation frame |
| `R1 invoke` | Immediately before `workspace_queue_decision` invoke | Tauri handler entry |
| `R2 authority transaction` | Authority obtains the exact Queue/store locks and begins expected-revision/correlation validation | Canonical Queue/session/receipt write is durable and required reconciliation state is recorded |
| `R3 acknowledgement` | Durable commit complete | React receives and validates the typed acknowledgement |
| `R4 canonical reload` | React invokes `workspace_snapshot` after acknowledgement | Ready/empty snapshot is decoded by React |
| `R5 visible result` | React calls `setState` with the canonical reload | Second animation frame showing the exact acknowledged/deferred state or one exact queued session and no duplicate |
| `R6 runner claim` | Approval handler receives `session_id` | Separately, `runner_started_at` is durable and the rendered projection shows the claimed session |

The current handler already exposes the intended ordering: pending state, typed Queue decision, optional deferred runner start, canonical snapshot reload, acknowledged visible action, then Queue refresh. [Rendered Queue workflow](../../mission-control/src/App.tsx#L1755-L1817) `R5` and `R6` must remain separate because runner start is intentionally deferred and can race the reload.

## Correctness and rollback gates

No latency sample is valid unless its corresponding correctness record passes.

### Common contract gates

- Same schema version, success/failure code, recoverability, Workspace Session identity, Active Mission identity, revision, Queue item, session boundary, and visible projection.
- Expected-revision rejection before mutation.
- Exact correlation replay returns the original effect without another revision or session.
- Reusing a correlation with a changed boundary fails closed.
- Approval persists `queued` before runner/worktree effects; the rendered action does not imply execution.
- One-process CLI fallback and persistent transport remain authority-equivalent.
- Capability/history/Working Context failures remain explicit; the candidate must not define “fast” by omitting them.

Production tests already encode key parity checks: warm correlated transport, durable queued-before-worktree behavior, and exact Ad Hoc boundaries. [Persistent transport test](../../tests/test_orchestrator_server.py#L61-L107) [Ad Hoc approval invariant](../../tests/test_workspace_snapshot.py#L4006-L4063)

### Shadow and rollback gates

- Python remains the sole canonical writer during comparison. Rust reads an immutable/shadow fixture and emits a candidate projection/effect record.
- Hash all canonical Python stores before and after every Rust shadow sample; any unplanned byte change invalidates the run.
- Compare normalized projections and receipts, but freeze or explicitly normalize generated timestamps/ids before claiming parity.
- Inject crash cuts before validation, before durable write, after write/before acknowledgement, and after acknowledgement/before render. Restart must yield either the old state or one fully committed new state, never a duplicate or partial effect.
- The feature flag can return all commands to Python without state conversion, data loss, or a rebuild of user runtime state.
- A cutover is not eligible until both variants pass the same public contract and restart/replay suite. Language choice does not waive persistence migration or CLI compatibility.

The prototype already demonstrates useful but narrower gates: schema-v1 import, exact replay, correlation conflict, false-success/no-action truth, eligible deferred dispatch, source-byte preservation, and crash-cut sidecar preservation. [Prototype scenarios](../../mission-control/src-tauri/prototypes/rust-orchestrator-slice/src/main.rs#L170-L340) [Prototype rollback proof](../../mission-control/src-tauri/prototypes/rust-orchestrator-slice/src/main.rs#L345-L393)

## Sampling and reporting

- Run at least 30 process-cold starts per variant and at least 100 process-warm rendered actions per fixture/variant.
- Alternate randomized `AB`/`BA` pairs to reduce drift; never run all Python samples before all Rust samples.
- Keep one variant active at a time. The prototype's process-id-qualified temporary path collided when separate isolated executions ran concurrently with the same visible PID, confirming that parallel samples can corrupt the measurement fixture.
- Preserve every raw monotonic stage record as JSON Lines, plus artifact/fixture hashes and correctness outcomes.
- Report n, raw sample link, p50, nearest-rank p95, min/max, failures/timeouts, and paired deltas. Do not report p95 for tiny exploratory sets as if stable.
- Predeclare the smallest product-relevant improvement before running the comparison. Statistical significance without a meaningful `S0→S8` or `R0→R5` reduction is not an architecture reason.
- Treat model first token, worker completion, and human review dwell as separate distributions. Backend language must not take credit for model-residency changes or immediate synthetic review.

## Measurement risks

1. **No equivalent Rust authority exists.** This is the primary gap; the current prototype cannot answer the ticket's product-performance question.
2. **Early readiness marker.** `gui-smoke-ready.json` means Python snapshot returned, not that the canonical workstation painted.
3. **Coarse smoke polling.** The existing 200 ms poll can quantize a sub-second startup result by a material fraction.
4. **Launcher-heavy critical path.** Full AppImage hashing and multiple subprocess probes occur before the native child; a Rust backend does not remove them.
5. **AppImage extraction policy.** Installed launch sets `APPIMAGE_EXTRACT_AND_RUN=1`, so extraction behavior must remain identical across variants. [Installed adapter environment](../../mission-control/bin/desktop-adapter.js#L197-L203)
6. **Asynchronous hydration.** Snapshot readiness, capabilities, history, Working Context, Queue, and Shell are separate effects and may contend for the shared persistent-backend mutex.
7. **Development distortion.** `npm run desktop` starts Vite and a debug Tauri build; `StrictMode`, compilation, dev server, and source maps make it unsuitable for production startup claims.
8. **Shadow overhead.** Rust GUI prototype timings contain two Python snapshots and zero canonical writes.
9. **Fixture-size drift.** Fresh tiny stores understate histories, journals, Mission catalogs, Queue receipts, and Issue graphs.
10. **Filesystem and WSL effects.** `/mnt/c`, ext4, AppImage extraction, Windows-host display forwarding, and WSL scheduling are distinct test environments.
11. **Cache terminology.** Restarting processes does not clear page cache; unsafe privileged cache dropping is neither necessary nor authorized for the primary comparison.
12. **Cross-process clocks.** Python `perf_counter`, Rust `Instant`, Node `hrtime`, and browser `performance.now` are monotonic locally but do not expose one guaranteed shared origin.
13. **Runner race.** Approval starts the deferred runner before the canonical reload completes, so visible queued/running state can differ unless `R5` and `R6` are separately marked.
14. **Historical artifact mismatch.** The locally verified AppImage predates the current dirty source and Rust prototype; rebuilding is required before a current product comparison.
15. **Microbenchmark overinterpretation.** Microseconds for an in-memory reducer do not imply a perceptible desktop improvement when real persistence, rendering, and model work are absent.

## Decision consequence

The performance question can be resolved without inventing a Rust win:

1. **Does Rust currently produce a repeatable, human-perceptible improvement?** No accepted evidence shows that it does. The available Rust result is a non-equivalent reducer/sidecar microbenchmark.
2. **What currently dominates the measured Python path?** The supported release opens a window around 0.54–0.55 seconds and reaches an early backend-ready proxy around 1.27–1.29 seconds. The whole first Python snapshot is about 0.10 seconds; warm Python snapshot, Queue, and Review projections are 7–8 ms p50. Most measured startup time and all usable-paint uncertainty sit outside the Python authority stage.
3. **Is a speed improvement large enough to justify compatibility, coexistence, packaging, and maintenance cost?** No such justification is established. Even the impossible zero-time replacement of the whole first Python snapshot bounds the proxy saving near 0.10 seconds, while a real Rust authority must retain equivalent persistence, validation, receipts, failure mapping, audit reconciliation, CLI compatibility, and rollback.
4. **What follows for the backend architecture decision?** Do not choose a full Rust rewrite for performance. Retaining Python or an evidence-gated staged hybrid remains consistent with the measurements; any Rust module must be justified by a separately demonstrated reliability or maintainability benefit and must treat paired product-performance evidence as a cutover gate, not an assumption.

If a future Rust module claims desktop speed, its required evidence step is a **bounded shadow authority benchmark**, not a full rewrite:

1. add correlation-scoped monotonic marks for `S0–S9` and `R0–R6`;
2. preserve one production AppImage/React/Tauri path for both variants;
3. implement only the exact Workspace Snapshot and Ad Hoc Queue-decision candidate seams in Rust;
4. pass the same contract, persistence, restart, replay, and rollback gates;
5. publish raw paired samples and only then judge whether the user-visible p50/p95 improvement is material.

The durable conclusion is: **Rust's isolated reducer is fast, the installed Python control plane is already millisecond-scale after startup, the measured desktop is dominated elsewhere, and performance does not justify a full Rust backend or immediate authority transfer.**
