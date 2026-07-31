# Alfredo workspace selection and false-success routing diagnosis

**Diagnosed:** 2026-07-24

**Purpose:** resolve the Wayfinder research ticket **Diagnose workspace selection and false-success action routing** without implementing the modernization

**Source state:** branch `main`, HEAD `91fb98c9e93f4a3a913cae5acfcf51eeef974f6e`, plus the preserved pre-existing dirty working tree

## Executive finding

The two reported paths stop before different missing boundaries:

1. Source launch never enters a Coding Workspace selection flow. The launcher commits the invocation directory as `selected_workspace`; direct `npm run desktop` falls back to the Alfredo backend root. Tauri then configures that path as the Python `target_repo`, and React receives only an already-selected, non-null workspace. The later “Recent workspace” control creates a relaunch command; it cannot choose or create the current Coding Workspace. This is a missing cross-layer Starting Location-to-Coding Workspace contract, not a path-propagation failure after selection.
2. A confirmed filesystem request that reaches the controller can be returned as a valid `discussion` route with success-sounding reply text. Python validates JSON shape and route bounds but not the semantic agreement between reply and route, persists every controller reply as `model-commentary`, and returns both fields independently. React displays the reply first and invokes the effectful path only when `route.intent === "coding-task"`. A `discussion` route therefore stops before Mission Formation, proposal creation, worker eligibility, approval, session creation, or dispatch. The Orchestrator correctly performs no unauthorized effect, but Alfredo can still make a false product claim because no effect receipt is bound to the reply.

The healthy proposal-to-session path is not implicated. Focused tests prove that an exact `coding-task` route proceeds through canonical proposal reload, eligible-worker selection, approval, queued session creation, and deferred dispatch; boundary mismatch fails closed.

## Canonical domain boundary

This diagnosis uses the repository's established terms:

- The **Starting Location** is the directory from which Alfredo begins repository choice. It is not automatically the Coding Workspace.
- The **Coding Workspace** is the repository root deliberately selected by the Mission Commander.
- A **Controller Route** is model output. It classifies a turn; it does not grant authority or prove an effect.
- Only the **Orchestrator** may acknowledge an accepted mutation, proposal, approval, assignment, or session.
- A narrow effectful coding request belongs on the governed Ad Hoc Delegation path; broader goals must eventually use the agreed Mission Formation Route.

No new domain term was required. The code conflicts with the existing Starting Location/Coding Workspace distinction because it represents only an already-selected workspace.

## Red-capable feedback loops

### Workspace selection

The minimal deterministic loop launches the current development adapter in dry-run mode and asserts the desired pre-selection state:

```bash
ALFREDO_DESKTOP_DRY_RUN=1 \
ALFREDO_RUNTIME_ROOT=/tmp/alfredo-wayfinder-45-loop \
node mission-control/bin/alfredo.js workstation --agent qwen3-14b \
| node -e 'let input=""; process.stdin.on("data", chunk => input += chunk); process.stdin.on("end", () => { const launch = JSON.parse(input); if (launch.starting_location && !launch.selected_workspace) process.exit(0); console.error(`RED: launch immediately selected ${launch.selected_workspace ?? "<none>"} and exposed no uncommitted Starting Location`); process.exit(1); });'
```

Observed in 0.1 seconds:

```text
RED: launch immediately selected <Alfredo source repository> and exposed no uncommitted Starting Location
```

### False-success routing

A temporary one-test harness reused the production `WorkspaceSnapshotService`, `AgentConsoleHistoryService`, `AgentConsoleResponseService`, and `WorkspaceQueueService`. It submitted:

```text
Yes, create the requested folder in this Coding Workspace now.
```

The fake bounded controller returned a valid JSON discussion route plus:

```text
Done — I created the requested folder.
```

The assertion rejected the exact combined symptom: `model-commentary`, `discussion`, success wording, zero Queue items, and zero sessions.

```bash
PYTHONPATH=/tmp python3 -m unittest \
  wayfinder45_false_success_test.FalseSuccessRoutingRepro.test_confirmed_filesystem_request_cannot_end_as_unaudited_success_commentary
```

Observed in 0.029 seconds:

```text
AssertionError: True is not false : controller success copy survived despite no Orchestrator effect, Queue item, or session
```

The temporary harness was removed after the diagnosis.

## Path A — Starting Location to Coding Workspace

| Stage | Current behavior | Where it stops |
|---|---|---|
| Terminal invocation | `parseWorkstationLaunch()` calls `trackerArgs(process.cwd())` and unconditionally sets `selected_workspace: process.cwd()`. [launcher](../../mission-control/bin/alfredo.js#L500-L539) | Starting Location is never represented as a distinct state. |
| Direct source development | `npm run desktop` is `tauri dev`. Without launcher environment, `BridgeConfig::for_repository()` sets `target_repo` to the backend root. [package](../../mission-control/package.json), [Rust fallback](../../mission-control/src-tauri/src/lib.rs#L79-L111) | Alfredo's own repository is selected by fallback before the UI starts. |
| Launcher-to-Tauri handoff | The launch environment exports `ALFREDO_SELECTED_WORKSPACE` from the already-committed launch plan. [launcher environment](../../mission-control/bin/alfredo.js#L780-L795) | There is no pending selection request or acknowledgement. |
| Tauri configuration | `BridgeConfig::from_environment()` replaces `target_repo` with that environment path. [bridge configuration](../../mission-control/src-tauri/src/lib.rs#L100-L133) | The persistent Python command boundary is now permanently qualified to that repository for this desktop process. |
| Launch-context contract | Rust and TypeScript require `selected_workspace: String`; neither contract has `starting_location`, nullable selection, selection status, repository candidates, or create/select actions. [Rust launch context](../../mission-control/src-tauri/src/lib.rs#L27-L34), [TypeScript contract](../../mission-control/src/contracts.ts#L10-L16) | React cannot express “selection required.” |
| Python Orchestrator | Every command receives the configured `--target-repo`; snapshots correctly project that path as the Workspace Session. | Python sees a selected Coding Workspace, not a Starting Location. |
| React projection | The prompt status displays `Workspace <basename>`. Recent paths are combined into a relaunch selector and copyable command. [projection](../../mission-control/src/App.tsx#L2742-L2765), [status](../../mission-control/src/App.tsx#L3645-L3665) | The control can relaunch another process but cannot select/create the current Coding Workspace. |
| Mission behavior | Tracker and Mission identity were derived from the preselected path before launch. | There is no known-repository choice between Resume Mission and Start New Mission. |

### Classification

- **Missing product/architecture contract:** Starting Location, selection-pending state, existing-repository selection, repository creation, validation, and known-repository Resume Mission/Start New Mission choices do not exist across launcher, Tauri, Python, or React.
- **Misleading product feedback:** React labels the preselected directory “Workspace” without showing that no deliberate selection occurred. The “Recent workspace” control looks selectable but only produces a relaunch command.
- **Not a propagation defect:** once `selected_workspace` exists, install/backend root separation, Tauri configuration, Python qualification, and snapshot projection agree on the same path.
- **Source fallback defect relative to the new contract:** using the backend root as `target_repo` is a practical development fallback, but it violates the now-established rule that the Starting Location is not a Coding Workspace until deliberately selected.

## Path B — confirmed filesystem request to visible non-action

| Stage | Current behavior | Where it stops |
|---|---|---|
| Prompt persistence | React optimistically renders the Mission Commander turn, then Python persists it as `proposed`. | Safe and expected. |
| Deterministic intent | `/task`, `/use`, explicit remediation forms, or explicit subagent forms bypass model classification. A confirmation such as “Yes, create…” matches none of those forms. [intent parser](../../mission-control/src/App.tsx#L167-L212) | The turn falls through to the model controller. |
| Controller context | Python includes up to eight recent turns, bounded Working Context, repository instructions, domain context, and the current prompt. [prompt construction](../../albert_mvp/workspace.py#L3033-L3097) | Context is present; the failure is not caused by sending only the word “Yes.” |
| Controller contract | The prompt asks for `discussion` or `coding-task` and tells the model not to claim Orchestrator acknowledgement. [prompt contract](../../albert_mvp/workspace.py#L3091-L3097) | This is advisory model instruction, not an enforced invariant. |
| Route parsing | Python validates JSON keys, reply bounds, task bounds, and criteria bounds. A valid `discussion` route returns the reply unchanged; malformed routes also fall back to discussion while preserving safe reply text. [route parser](../../albert_mvp/workspace.py#L2774-L2842) | Reply truth and route intent are never compared. |
| History persistence | Every controller reply is appended with `outcome="model-commentary"` and `source="frontier-model"` regardless of route. [response persistence](../../albert_mvp/workspace.py#L2747-L2764) | No effect correlation, proposal id, acknowledgement, or explicit “no action” fact is attached. |
| React projection | React immediately appends and renders the reply. It invokes `delegateCodingTask()` only for `coding-task`; discussion falls through to context refresh. [route branch](../../mission-control/src/App.tsx#L1468-L1500) | This is the exact false-success stopping point. |
| Mission Formation | The current controller schema has only `discussion` and `coding-task`; it cannot express Ad Hoc Delegation vs bounded Mission discovery vs Wayfinding as a first-class Mission Formation Route. | No Mission Formation decision is created. |
| Proposal and approval | Not entered for discussion. For a coding task, React builds a Mission/scope-qualified proposal, reloads the canonical Queue boundary, verifies exact equality, and submits approval. [governed delegation](../../mission-control/src/App.tsx#L1246-L1407) | No Queue item exists in the failing path. |
| Worker eligibility | Not evaluated for discussion. The healthy path requires an available, assignable, ungated, non-delegate local worker before proposal/approval proceeds. | No worker is selected in the failing path. |
| Session dispatch | Not entered for discussion. A healthy acknowledged Queue decision returns a session id and starts the deferred runner. | No session or Mission Work card exists in the failing path. |
| Durable truth | Agent Console history contains the success-sounding controller text; Queue, Mission runtime, Activity Journal, and sessions contain no corresponding effect. | Authority remains safe, but the human-visible chronology is semantically false. |

### Classification

- **Controller/model classification error:** the model emitted `discussion` for a confirmed effectful request. This is not an Orchestrator execution failure.
- **Backend contract defect:** reply and route are independent. Python accepts success-sounding reply content on a non-effectful route, including the fail-closed malformed-route fallback, and has no effect-receipt binding.
- **Missing Mission Formation contract:** the route cannot represent the already-agreed Ad Hoc Delegation / bounded `grilling` / `wayfinder` choice. Narrow work can reach Ad Hoc Delegation only through the generic `coding-task` branch.
- **React product-feedback defect:** the UI renders arbitrary controller text plus a small `frontier-model / model-commentary` label but gives no explicit “No action taken” outcome when discussion follows effectful language. It does not surface the route decision.
- **Persistence safety is correct but insufficient:** no false Queue, Activity, session, or accepted Mission state is fabricated. The unsafe part is the durable human-facing assertion in Agent Console history.
- **Proposal, approval, eligibility, and dispatch are not defective in this reproduction:** they were never called. Focused tests confirm the healthy exact-boundary path and its fail-closed mismatch behavior.

## Hypotheses tested

1. **Launcher collapses Starting Location and Coding Workspace. Confirmed.** The red launch plan and source show unconditional `process.cwd()` selection, with Rust backend-root fallback for direct source development.
2. **Reply and route are validated independently. Confirmed.** The 0.029-second harness preserved false success on a valid discussion route.
3. **React dispatches solely from `route.intent`. Confirmed.** Discussion ends after display/context refresh; coding-task enters governed delegation.
4. **Truthfulness is prompt-only. Confirmed.** The model is instructed not to claim acknowledgement, but no receipt or semantic validation enforces it.
5. **Feedback does not distinguish non-effect strongly enough. Confirmed.** The raw reply is primary UI content; source/outcome metadata is secondary, and no “no action” projection exists.

## Healthy-path cross-checks

The following existing focused tests passed:

```bash
python3 -m unittest \
  tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_controller_model_routes_a_natural_coding_request_as_bounded_task \
  tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_controller_route_safely_falls_back_to_discussion
```

Result: 2 tests passed in 0.173 seconds.

```bash
cd mission-control
npm test -- --run src/App.test.tsx \
  -t 'automatically approves and dispatches an exactly bounded subagent coding request'

npm test -- --run src/App.test.tsx \
  -t 'keeps discussion in controller chat instead of creating a coding proposal'
```

Result: each selected React test passed. They prove both the successful governed branch and the intentional discussion non-action branch.

## Required blueprint seams

The later modernization blueprint and Rust vertical slice should treat these as acceptance seams rather than UI polish:

1. **Pre-workspace launch state:** carry a distinct Starting Location and a selection state. `selected_workspace` must be absent until an existing repository is selected or a new repository is created and validated.
2. **Acknowledged workspace transition:** selecting/creating a Coding Workspace must be a typed result that establishes tracker/Mission context only after acknowledgement. Known repositories must offer Resume Mission or Start New Mission without silent duplication.
3. **First-class Mission Formation Route:** represent narrow Ad Hoc Delegation, bounded visible-route discovery, and multi-session Wayfinding explicitly; do not overload a two-value discussion/coding-task model result.
4. **Effect-truth invariant:** controller prose is never evidence of an effect. Any UI claim that work was proposed, approved, executed, created, changed, or completed must come from a correlated Orchestrator receipt or canonical projection.
5. **Fail-closed controller feedback:** if an effectful request cannot produce a valid effect route, Alfredo must visibly clarify or refuse and state that no action occurred. A malformed route must not preserve success-sounding prose as the primary answer.
6. **React route projection:** render controller commentary as commentary and show the route/no-action outcome. Render proposal, approval, session, and completion only as separate canonical events.
7. **Regression seams:** retain a fast launcher contract test for uncommitted Starting Location and a backend-plus-React test that rejects an unaudited success claim while Queue and sessions remain empty.

## Wayfinder consequence

This diagnosis makes the existing **Prototype the Mission Execution Tree** ticket actionable by removing its final blocker. It also supplies exact compatibility and acceptance boundaries to **Prototype a Rust Orchestrator vertical slice** and **Choose Alfredo's backend modernization architecture**. No duplicate ticket is needed, and the map's remaining fog is unchanged.
