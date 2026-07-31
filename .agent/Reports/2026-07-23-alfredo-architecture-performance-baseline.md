# Alfredo architecture and performance baseline

**Measured:** 2026-07-23

**Purpose:** establish the current evidence-backed baseline for modernization planning; no modernization was implemented

**Source state:** branch `main`, HEAD `91fb98c9e93f4a3a913cae5acfcf51eeef974f6e`, plus the pre-existing dirty working tree

## Executive baseline

Alfredo has a clear logical authority boundary: the Python Orchestrator owns Mission state, Issue Slices, Local Agent sessions, permissions, Evidence Packages, and review; React projects and requests state, Tauri validates/transports it, and the persistent Python server avoids startup on routine calls. That logical split is supported by the documented command protocol and by source inspection. It is not yet a narrow physical split: most authority and runtime mechanics are concentrated in `albert_mvp/core.py` and `albert_mvp/workspace.py`, while the UI and Rust bridge are also large manually synchronized modules. [Architecture inventory](../System/project_architecture.md#directory-structure), [command boundary](../System/api_endpoints.md#request), [Python authority](../../albert_mvp/core.py#L1166-L1255)

Fresh measurements show three different performance regimes:

- The backend control plane is fast. A fresh one-process `workspace-snapshot` reached a `ready` Workspace Session and Active Mission in **185.117 ms median**; warm persistent snapshots took **9.155 ms median / 11.847 ms p95**.
- An approved Workspace Queue intent became a durable queued Local Agent session in **2.010 ms median** inside the authority service. Including the persistent JSON command round trip took **12.296 ms median**, and the next public runner request had persisted `runner_started_at` by **85.377 ms median** from approval-request start.
- Model work dominates. A deliberately tiny qwen3:14b response took **4.507 s median model-unloaded** and **2.172 s median resident**, with large resident variance. A real one-shot Gemma4-12B coding goal reached a structurally valid, reviewed Evidence Package in **37.842 s**.

The architectural decision this baseline supports is therefore: preserve the Orchestrator's authority and deferred-execution semantics, but treat contract duplication, module concentration, observability, and model/runtime latency as separate modernization problems. Faster JSON persistence alone cannot materially improve the live coding path.

## Scope and canonical definitions

This report uses the repository's canonical terms:

- A **Coding Workspace** is the deliberately selected repository root, distinct from Alfredo's install and backend roots. A **Workspace Session** is the continuous relationship opened over it. [Definitions](../../CONTEXT.md#coding-workspace), [Workspace Session](../../CONTEXT.md#workspace-session)
- A **Mission** holds accepted work and its Mission-specific Shared Context. The **Active Mission** is the one eligible for conversational steering. [Mission terms](../../CONTEXT.md#active-mission)
- A **Local Agent** executes a bounded packet in an isolated worktree and must return an **Evidence Package**. An Evidence Package contains changed files, a bounded real diff, commands, test results, risks, and proposed context updates before review. [Local Agent](../../CONTEXT.md#local-agent), [Evidence Package](../../CONTEXT.md#evidence-package), [required fields in source](../../albert_mvp/core.py#L955-L1004)

“Ready” in the measurements means the returned canonical snapshot had `workspace_session.status == "ready"`, schema version 1, and the expected Active Mission. The source defines `ready` as an Active Mission board containing at least one Issue Slice; an empty board is explicitly `empty`. [Snapshot derivation](../../albert_mvp/workspace.py#L9013-L9048)

## Current architecture

### Authority and module boundaries

| Layer | Current owner and seam | Baseline finding |
|---|---|---|
| Installation and launch | `mission-control/bin/alfredo.js` selects the Coding Workspace, separates repository/bundled backend layouts, runs preflight, and starts either the development Tauri shell or exact installed native adapter. [launcher source](../../mission-control/bin/alfredo.js#L19-L49), [native/development selection](../../mission-control/bin/alfredo.js#L86-L119) | Install root, backend root, runtime root, and Coding Workspace are explicitly distinct. Model availability is not a startup prerequisite. |
| React workstation | `App.tsx` owns optimistic Agent Console interaction and the UI workflow; `WorkspaceClient` is its typed port. [client interface](../../mission-control/src/workspace-client.ts#L75-L123) | React does not own accepted Mission state, but it does orchestrate multi-call workflows such as proposal → canonical Queue reload → approval → deferred dispatch. [automatic delegation flow](../../mission-control/src/App.tsx#L1206-L1407) |
| Tauri bridge | Rust owns typed request/response structs and maps each command to Python argv. A shared persistent backend serves short calls; controller inference and Local Agent execution use isolated processes so they do not hold the shared supervisor mutex. [persistent/isolated split](../../mission-control/src-tauri/src/lib.rs#L247-L303), [snapshot execution](../../mission-control/src-tauri/src/lib.rs#L1223-L1269), [deferred runner](../../mission-control/src-tauri/src/lib.rs#L1866-L1881) | Transport persistence is an optimization, not a second authority. Long work is correctly separated from UI polling. |
| Command transport | `albert_mvp.server` accepts newline-delimited `{id, argv}` requests, invokes the same CLI `main()`, and returns correlated stdout/stderr envelopes. [server source](../../albert_mvp/server.py#L12-L42) | Persistent and one-process CLI paths share command semantics. The envelope id is transport-only; durable mutations use separate correlation ids and expected revisions. [request contract](../System/api_endpoints.md#request) |
| Python application boundary | `cli.py` defines the public grammar and maps validated args onto workspace/core services. [parser](../../albert_mvp/cli.py#L77-L143), [service dispatch](../../albert_mvp/cli.py#L882-L980), [governed/review dispatch](../../albert_mvp/cli.py#L1070-L1161) | The CLI is an adapter, but it is broad: it manually repeats most endpoint fields and action families. |
| Domain authority and execution | `AlbertMission` loads Issue Slices, owns runtime/session/review state, checks worker policy, claims queued sessions, prepares worktrees, runs fake/command/Ollama workers, and collects evidence. [mission state](../../albert_mvp/core.py#L1166-L1255), [runner claim](../../albert_mvp/core.py#L2188-L2268) | The authority boundary is coherent, but domain, persistence, sandbox, Git, runner, repair, evidence, and PR-prep responsibilities coexist in one module. |
| Workspace services and persistence | `workspace.py` holds Agent Console, Working Context, Queue, Mission Draft, Review Workspace, Shell, Activity Journal, Workstation action, synchronization, and canonical snapshot services. Its decorators serialize revision check plus authoritative store mutation. [atomic action wrapper](../../albert_mvp/workspace.py#L67-L133), [snapshot service](../../albert_mvp/workspace.py#L8986-L9048) | Service names create logical seams, but they share a large module and internal fields such as `_snapshots._missions` and `_primary_mission`. |

### Physical concentration

Fresh source measurements (line counts from `wc -l`; Python declarations from `ast`) were:

| File | Lines | Structural count |
|---|---:|---:|
| `albert_mvp/core.py` | 6,794 | 13 top-level classes, 36 top-level functions, 159 class methods |
| `albert_mvp/workspace.py` | 9,367 | 59 top-level classes, 5 top-level functions, 230 class methods |
| `albert_mvp/cli.py` | 1,472 | 11 top-level functions |
| `albert_mvp/server.py` | 46 | 1 top-level function |
| `mission-control/src-tauri/src/lib.rs` | 4,613 | 149 `pub struct` / `pub enum` / `pub fn` / private `fn` declarations |
| `mission-control/src/contracts.ts` | 1,004 | 116 exported or top-level interface/type/class declarations |
| `mission-control/src/App.tsx` | 6,643 | primary rendered workflow module |

These counts do not prove poor design by themselves. They do establish that a change to a cross-language action frequently touches a small number of very large files, which raises review, navigation, merge-conflict, and contract-drift cost.

### Typed command and transport seams

The public application boundary is versioned JSON across four hand-written representations:

1. Python dataclasses/services produce schema-versioned projections.
2. `argparse` exposes the same fields as public CLI arguments.
3. Rust structs deserialize and validate the Python JSON, then Tauri commands expose it.
4. TypeScript interfaces and `TauriWorkspaceClient` repeat the contracts for React. [API overview](../System/api_endpoints.md#endpoints), [TypeScript schema literals](../../mission-control/src/contracts.ts#L55-L61), [client invocation](../../mission-control/src/workspace-client.ts#L170-L199), [Rust schema check](../../mission-control/src-tauri/src/lib.rs#L1180-L1190)

This is genuinely typed at every consumer, and the fresh TypeScript typecheck passed. It is not generated from one schema. Drift is caught by tests and deserialization rather than prevented by construction. The repeated `schema_version: 1` declarations and endpoint-specific adapters are a modernization constraint: any schema extraction must preserve structured failure codes, expected-revision behavior, Mission qualification, and CLI fallback parity.

### Persistence seams

There is no relational database. The current authority is split across nine documented JSON stores plus per-session artifacts: `runtime.json`, workspace preferences, Agent Console history, Workspace Queue, Mission Drafts, Working Context curation, Activity Journal, Shell Terminal, and append-only path-grant requests. Bulky output and `review.diff` stay under session artifact directories. [store inventory](../System/database_schema.md#tables)

The implementation uses sibling-file atomic replacement, `flock`, store/coordinator locks, and expected revisions. The workspace action decorator holds one store transaction around validation and mutation; chronology has a distinct lock to order durable effects and audit phases. [persistence contract](../System/database_schema.md#relationships), [lock implementation](../../albert_mvp/workspace.py#L67-L133)

Important limitation: atomicity is per store/action path, not one transaction spanning every JSON store. The architecture explicitly does not claim cross-file atomicity for canonical effects plus Activity Journal. [architecture note](../System/project_architecture.md#command-deck-activity-journal-boundary) Modernization must preserve replay/reconciliation behavior before replacing storage.

### Compatibility obligations

The current source establishes these obligations:

- `alfredo` is the product command, but deprecated `albert` remains a bin alias and `albert_mvp` / `mission-control` remain compatibility names in code and records. [README](../../README.md#alfredo-local-coding-agent-workstation), [bin aliases](../../mission-control/package.json#L7-L10)
- Source-repository and bundled-backend layouts must both work; `ALBERT_BACKEND_ROOT` and `ALBERT_PYTHON` remain explicit overrides. [launcher fallback](../../mission-control/bin/alfredo.js#L19-L38), [runtime prerequisites](../../README.md#L21-L23)
- Persistent transport and the one-process CLI fallback must remain authority-equivalent. [transport contract](../System/api_endpoints.md#endpoints)
- Projection schemas remain versioned; malformed or non-contiguous state fails closed. Missing optional fields retain compatibility defaults, and legacy unstarted `launched` sessions decode as `queued`. [migration history](../System/database_schema.md#migration-history), [legacy decode](../../albert_mvp/core.py#L1054-L1085)
- The locally verified native distribution baseline is Ubuntu 24.04 x64/glibc 2.39. Wider Linux compatibility and public-registry installation are not yet claims. [release boundary](../../README.md#L11-L23), [current correction](2026-07-12-alfredo-install-queue-acceptance-correction.md#remaining-external-gate)
- Queue is a decision-only inbox, Complete is reviewed/PR-ready rather than merged, and raw model/terminal output cannot become accepted Mission state. These are semantic compatibility requirements, not merely presentational choices. [Queue definition](../../CONTEXT.md#workspace-queue), [Complete Issue Slice](../../CONTEXT.md#complete-issue-slice), [API response boundary](../System/api_endpoints.md#response)

## Fresh performance measurements

All percentiles below use nearest-rank p95. Fixtures, runtime stores, worktrees, and artifacts were created under `/tmp`; repository sources were not used as measurement targets. The deterministic harnesses used the current dirty source directly and preserved the user's worktree.

### Control-plane measurements

| Metric | Definition | n | Median | p95 | Range |
|---|---|---:|---:|---:|---:|
| Coding Workspace → Mission backend-ready floor | Fresh `python3 -m albert_mvp workspace-snapshot` process against a new Coding Workspace, runtime root, PRD, and one AFK Issue Slice; ends at decoded schema-v1 snapshot with ready Workspace Session and expected Active Mission | 20 | **185.117 ms** | **190.416 ms** | 180.776–191.987 ms |
| Warm persistent snapshot | Request write/flush to correlated response read on one `albert_mvp.server`; first request discarded | 100 | **9.155 ms** | **11.847 ms** | 7.629–22.111 ms |
| Approved intent → durable queued session, authority-only floor | `WorkspaceQueueService.decide(approve)` after goal/history/proposal setup; ends when the durable session exists with `status == "queued"` | 50 | **2.010 ms** | **2.739 ms** | 1.683–3.621 ms |
| Coding goal → reviewed Evidence Package, authority-only synthetic floor | Agent Console append → Ad Hoc proposal → approval → deterministic fake worker → valid Evidence Package → Review Workspace accept; fresh fixture each sample | 20 | **136.572 ms** | **137.928 ms** | 134.933–137.996 ms |

The benchmark command was:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 /tmp/alfredo_baseline_benchmark.py
```

The temporary harness SHA-256 was `dfd503e99d502ce95c24cb342629bd2ae0fff5e24c719a98583a7416a93bc9c9`. It instantiated the production `AlbertMission`, `WorkspaceSnapshotService`, `AgentConsoleHistoryService`, `WorkspaceQueueService`, and `ReviewWorkspaceService`; it did not replace their persistence or session implementations. The source tests use the same persistent transport budget and lifecycle invariants. [warm-server acceptance test](../../tests/test_orchestrator_server.py#L61-L107), [queued-before-run invariant](../../tests/test_albert_mvp.py#L1549-L1577), [Ad Hoc approval invariant](../../tests/test_workspace_snapshot.py#L3958-L4022)

A second public-protocol cross-check used correlated `albert_mvp.server` commands and included the canonical Queue reload performed by React:

| Cross-check | n | Median | p95 | Range |
|---|---:|---:|---:|---:|
| Fresh persistent process → first ready snapshot | 20 | 184.766 ms | 187.132 ms | 181.337–191.365 ms |
| Warm persistent snapshot | 50 | 10.248 ms | 11.733 ms | 8.733–20.413 ms |
| Approval request → queued acknowledgement | 20 | 12.296 ms | 14.195 ms | 11.326–14.582 ms |
| Approval-request start → persisted `runner_started_at` after immediate public runner request | 20 | 85.377 ms | 87.011 ms | 84.067–87.687 ms |
| Public runner request → valid fake Evidence Package | 20 | 141.783 ms | 144.507 ms | 140.287–145.195 ms |
| Public goal → canonical Queue reload → approval → valid fake Evidence Package → accepted review | 20 | 210.729 ms | 213.277 ms | 208.053–217.395 ms |

That cross-check command was:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 .agent/Reports/.alfredo_baseline_measure.py
```

The temporary harness SHA-256 was `c6f24156ca71b00f73907983a63daa405e90a1bf36167db74ad8d2ae26831026`. The harness was removed after its results were captured so this report remains the single durable asset.

Interpretation:

- The 2.010 ms authority-only dispatch floor and 12.296 ms public queued acknowledgement are different boundaries, not conflicting results.
- The 85.377 ms figure is the best current controlled bound for approved intent to persisted runner claim. The React scheduler gap in a real rendered desktop was not instrumented.
- Both goal-to-reviewed fake-worker numbers are synthetic orchestration floors. They prove the control path and Evidence Package mechanics; they do not represent coding, model inference, tests, repair, user review dwell time, or useful task quality.

### Isolated qwen3:14b inference

The fixed non-streaming request used the local Ollama `/api/generate` endpoint:

```json
{
  "model": "qwen3:14b",
  "prompt": "Reply with exactly READY.",
  "stream": false,
  "think": false,
  "keep_alive": "10m",
  "options": {
    "temperature": 0,
    "seed": 42,
    "num_predict": 8,
    "num_ctx": 2048
  }
}
```

Every response was exactly `READY.` with 21 prompt tokens and 3 output tokens. Ollama defines `total_duration`, `load_duration`, prompt evaluation, and token evaluation timings in nanoseconds; `keep_alive: 0` unloads the model immediately, while a duration keeps it resident. [official Generate API](https://docs.ollama.com/api/generate), [official timing semantics](https://docs.ollama.com/api/usage), [official keep-alive/unload semantics](https://docs.ollama.com/faq#how-do-i-keep-a-model-loaded-in-memory-or-make-it-unload-immediately)

The request/unload command shape was:

```bash
curl --silent --show-error --fail http://127.0.0.1:11434/api/generate \
  --data '{"model":"qwen3:14b","prompt":"Reply with exactly READY.","stream":false,"think":false,"keep_alive":"10m","options":{"temperature":0,"seed":42,"num_predict":8,"num_ctx":2048}}'

curl --silent --show-error --fail http://127.0.0.1:11434/api/generate \
  --data '{"model":"qwen3:14b","keep_alive":0}'
```

| Residency | n | Total median | p95 | Total range | Load median |
|---|---:|---:|---:|---:|---:|
| Model-unloaded immediately before each measured request | 4 | **4.506606 s** | not reported for n=4 | 3.897191–7.760013 s | **3.909638 s** |
| Resident before each measured request | 10 | **2.172332 s** | **9.800001 s** | 1.157634–9.800001 s | not material |

“Model-unloaded” is intentionally not called boot-cold: OS/filesystem cache and the Ollama server remained warm. The resident set's high p95 is real for this small sample and machine state; median alone would hide it. These timings are not the full Alfredo controller route, whose prompt includes repository instructions, domain context, Working Context, recent conversation, and a typed route contract. [controller prompt source](../../albert_mvp/workspace.py#L3033-L3103)

All exploratory qwen samples between 20:13 and 20:15 UTC were discarded because concurrent Gemma/Qwen residency changes and differing context reloads invalidated their cold/warm classification. They are not included in any table.

### One live Gemma4-12B coding workflow

Exactly one governed model workflow used a fresh Coding Workspace, tracker, runtime root, and normal `gemma4-12b` worker under:

`/tmp/alfredo-gemma-live-baseline-xjqo6_qs`

The goal was to create `READY.md` containing exactly `Alfredo live baseline ready.\n`. The run followed:

1. ready Mission snapshot;
2. durable Agent Console coding goal;
3. Ad Hoc Delegation proposal;
4. canonical Queue reload;
5. approval and durable queued session;
6. `mission.run_session()` through the production Ollama runner and one model iteration;
7. valid automated Evidence Package and `review.diff`;
8. Review Workspace accept to final `reviewed` / Complete state.

| Phase | Monotonic elapsed |
|---|---:|
| Coding Workspace → Mission ready (in-process service setup) | 1.405 ms |
| Coding goal recorded | 0.532 ms |
| Ad Hoc proposal | 0.420 ms |
| Canonical Queue reload | 0.058 ms |
| Approved intent → queued session | 2.468 ms |
| Local Agent run → valid Evidence Package | **37.834318 s** |
| Review accept | 3.244 ms |
| Coding-goal workflow start → reviewed terminal state | **37.842455 s** |

The one model plan created only `READY.md`; there were no planned commands or repair iterations. The file bytes were verified independently, including the final newline; SHA-256 was `9c679facd4ff7b06c739a627a678fd6de58385f36348b70305f8a00bbb228181`. The bounded `review.diff` SHA-256 was `0444686e2ff55870d69f0a099fc2e1652991357ea424f527d553b20b1568a73d`. The Evidence Package recorded the changed file, real diff, Ollama command, no configured test command, no known risk, and no context update.

Exact invocation:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 .agent/Reports/.gemma_live_baseline.py
```

The temporary harness SHA-256 was `381d2d287acc48d38e07ab084712e35beb2affad822046b5f6ee3d73099fd86e`; it was removed after capture. The actual model command was the production command returned by `_runner_command()`:

```bash
ollama run gemma4:12b --think=false --nowordwrap --format json
```

[runner command source](../../albert_mvp/core.py#L2498-L2503), [Ollama execution/repair loop](../../albert_mvp/core.py#L5310-L5479)

The persisted UTC timestamps span about 39.6 seconds while `time.perf_counter()` measured 37.8 seconds, indicating a roughly 1.8-second wall-clock adjustment during the WSL run. Monotonic elapsed is the latency result; persisted timestamps remain useful for ordering but are not trusted here for duration.

This one-file/no-test sample proves the current live path is operable. It is not a representative distribution for real coding goals, test execution, iterative repair, or review quality.

## Environment and exact inspection commands

| Component | Fresh value |
|---|---|
| OS/kernel | Ubuntu 24.04.3 LTS under WSL2; Linux `6.6.114.1-microsoft-standard-WSL2` |
| CPU | Intel Core i5-14400F; 16 logical CPUs exposed to WSL |
| Memory | 15 GiB WSL memory, 4 GiB swap |
| GPU | NVIDIA GeForce RTX 4070, 12,282 MiB; Windows driver 591.86 |
| Python | 3.12.3 |
| Node/npm | Node 24.11.0; npm 11.12.1 |
| Rust/Cargo | 1.96.0 |
| Git | 2.43.0 |
| Bubblewrap | 0.9.0 |
| Ollama | server/client 0.30.6 |
| Installed local models relevant to Alfredo | `qwen3:14b` 9.3 GB, `gemma4:12b` 7.6 GB, `gemma4:26b` 17 GB, `qwen3.6:27b` 17 GB, `qwen2.5-coder:14b` 9.0 GB, `deepseek-r1:14b` 9.0 GB |

Inspection and verification commands run:

```bash
git rev-parse HEAD
git branch --show-current
git status --short
git diff --stat

uname -a
lsb_release -a
lscpu
free -h
python3 --version
node --version
npm --version
rustc --version
cargo --version
bwrap --version
git --version
ollama --version
ollama list
nvidia-smi --query-gpu=name,driver_version,memory.total,memory.free,pstate,temperature.gpu,power.draw --format=csv,noheader
curl --silent --show-error --fail http://127.0.0.1:11434/api/version

wc -l albert_mvp/*.py mission-control/src/*.ts mission-control/src/*.tsx mission-control/src-tauri/src/*.rs
python3 -c 'import ast,json,pathlib; files=["albert_mvp/core.py","albert_mvp/workspace.py","albert_mvp/cli.py","albert_mvp/server.py"]; out={};
for f in files:
 p=pathlib.Path(f); t=ast.parse(p.read_text()); classes=[n for n in t.body if isinstance(n,ast.ClassDef)]; funcs=[n for n in t.body if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef))]; out[f]={"lines":len(p.read_text().splitlines()),"top_level_classes":len(classes),"top_level_functions":len(funcs),"class_methods":sum(len([m for m in c.body if isinstance(m,(ast.FunctionDef,ast.AsyncFunctionDef))]) for c in classes)}
print(json.dumps(out,indent=2))'

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -p 'test_*.py'
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest \
  tests.test_orchestrator_server.PersistentOrchestratorServerTests.test_warm_process_reuses_transport_within_latency_budget \
  tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_approved_ad_hoc_delegation_launches_bounded_session_without_issue_slice \
  tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_review_workspace_accepts_ad_hoc_delegation_evidence_without_issue_slice
(cd mission-control && npm run typecheck)
```

Fresh verification results:

- Full Python suite: **427 tests ran: 426 passed and 1 optional test skipped**, 51.176 seconds.
- Focused persistent-transport / Ad Hoc queue / review lifecycle set: **3 passed**, 0.446 seconds.
- TypeScript: `npm run typecheck` passed.

The architecture report's 418-test, 38-Rust-test, 227-frontend-test, and four-viewport counts are historical 2026-07-13 release evidence, not rerun claims in this baseline. [historical gate](../System/project_architecture.md#verification)

## Historical evidence kept separate

- The 2026-07-11 report measured a warm qwen3:14b controller median of about 1.07 seconds while launcher prewarm still existed; the same report now says prewarm was later removed. It is not comparable to the isolated model-unloaded/resident request above and is not treated as current. [historical controller measurement](2026-07-11-alfredo-one-shot-workstation.md#requirements-delivered)
- A 2026-06-16 Gemma4-12B run generated a runnable prototype, collected evidence, and reached reviewed/PR-ready, but recorded no phase timings and predates the current prompt-first/Ad Hoc workflow. [historical Gemma verification](2026-06-16-gemma-live-verification.md)
- A 2026-06-16 Gemma4-26B run demonstrated a failed first result, repair relaunch, and final reviewed result. It establishes repair capability, not current latency. [historical repair verification](2026-06-16-gemma26-repair-loop-verification.md)
- The 2026-07-13 production release gate proved clean-prefix package install, PATH launch, frontend load, and installed-backend snapshot readiness, but did not record Coding Workspace-to-Mission elapsed time. Publication and registry-only installation remain open. [release correction](2026-07-12-alfredo-install-queue-acceptance-correction.md#release-seam)

## Caveats and unresolved evidence

1. **Dirty source state.** Before this report, `git status --short` contained 78 changed/untracked paths and `git diff --stat` showed roughly 40,102 insertions and 7,168 deletions across 60 tracked files. These measurements describe that working tree, not clean HEAD. The report did not modify or revert those changes.
2. **Desktop readiness is unmeasured.** The 185 ms result excludes npm launcher preflight, Tauri/native process startup, React load/render, file picker/Starting Location choice, capability discovery, and a human seeing a usable Mission. The historical release gate proves readiness correctness, not its duration.
3. **Rendered dispatch gap is unmeasured.** Source inspection shows React dispatches the acknowledged queued session immediately and retries a still-canonical queue up to three times, but no browser/Tauri trace currently timestamps approval, invoke, Python runner claim, model first token, evidence-ready, and review-visible as one correlation. [dispatch implementation](../../mission-control/src/App.tsx#L1628-L1753)
4. **Qwen benchmark is intentionally tiny.** Three output tokens do not predict a full controller route. Resident p95 was 9.8 seconds despite a 2.17-second median, so more controlled samples and first-token timing are needed.
5. **Live coding sample is n=1 and trivial.** Gemma wrote one file, ran no tests, required no repair, and was reviewed immediately. Representative tasks need multiple file sizes, test suites, failure/repair cases, and independent review.
6. **Human time is excluded.** “Goal to reviewed” in synthetic and live measurements submits the acceptance immediately. Real Mission Commander reading and decision time is not product latency and must be reported separately.
7. **Concurrency is not characterized.** This baseline intentionally isolated model residency. It does not measure UI polling during a live worker, two Local Agents, controller-plus-worker competition, GPU eviction, or JSON-store growth.
8. **Persistence scaling is unknown.** Fixtures were small and fresh. There is no latency curve for large Agent Console histories, Activity Journals, Mission catalogs, Queue receipts, artifacts, or thousands of Issue Slices.
9. **Wall-clock duration is unsafe on this WSL host.** The live run observed a clock adjustment; duration instrumentation should use a monotonic clock while durable UTC timestamps remain ordering/audit data.
10. **No external publication claim.** The report says nothing new about npm publisher authority, provenance, public-registry installation, broader Linux compatibility, or human accessibility acceptance.

## Baseline acceptance criteria for the modernization blueprint

A modernization plan can use this report as its “before” state if it:

- preserves Python Orchestrator authority, Mission qualification, exact correlation replay, expected revisions, deferred execution, Evidence Package validation, and CLI fallback parity;
- distinguishes service-only, public transport, rendered desktop, model inference, worker execution, and human review timing rather than combining them;
- treats **185 ms fresh backend readiness**, **12 ms public queued acknowledgement**, **85 ms approved-to-runner-claim**, **2.17 s tiny resident qwen median with 9.8 s p95**, and **37.84 s one-file Gemma goal-to-reviewed** as separate reference points;
- adds end-to-end monotonic tracing before claiming a faster product workflow;
- reruns representative multi-sample live controller and worker tasks under controlled model residency before setting final latency objectives.
