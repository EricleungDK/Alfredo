# API Endpoints and Command Boundaries

**Last Updated:** 2026-08-09

Alfredo has no remote or production HTTP API. Its application boundary is a versioned JSON command protocol shared by the React client, the Tauri bridge, the development-only localhost gateway, the persistent Python server, and the one-process Python CLI fallback. Python remains authoritative; Rust validates and transports typed payloads, while React renders acknowledged projections.

## Endpoints

The endpoint families are:

| Family | Representative commands | Purpose |
|---|---|---|
| Launch and selection | Tauri `alfredo_launch_context`, `coding_workspace_select`, `mission_choice`; CLI/persistent `coding-workspace-select`, `mission-options`, `mission-choice`, `workspace-context` | Expose Starting Location with no implicit repository, acknowledge an exact Git repository, require explicit Mission choice, and restore the exact acknowledged journey |
| Workspace projection | `workspace-snapshot`, `workspace-updates`, `workspace-action` | Load canonical state and apply expected-revision navigation/preferences |
| Agent Console | `agent-capabilities`, `agent-console-message`, `agent-console-response`, `agent-console-history` | Discover commands/skills/models, append a prompt, route a typed Wayfinder first contact when applicable or generate controller commentary, and restore chronology |
| Working Context | `working-context`, `working-context-curate`, `workspace-scope` | Inspect bounded context and deliberately curate or qualify a prompt |
| Governed work | `workspace-queue`, `workspace-queue-decision`, `ad-hoc-delegation-proposal`, `workstation-action` | Propose, approve, assign, launch, cancel, archive/restore completed Issue Slices, and inspect Mission-qualified work |
| Deferred execution | `workstation-session-run` | Claim exactly one persisted queued session and run it outside the UI request path |
| Runner supervision | `runner-observe` | Deliver or replay one ordered advisory runner observation and return its canonical semantic receipt |
| Retirement lifecycle and storage | `retirement-preserve`, `retirement-verify`, `retirement-storage`, `retirement-inspect`, `retirement-pin`, `retirement-retry`, `retirement-export`, `retirement-discard`; terminal/review/cancel/repair commands and startup reconciliation | Preserve and retire by outcome, inspect bounded Snapshot Payload storage, and execute exact replay-safe blocked actions |
| Review | `review-workspace`, `review-decision` | Inspect validated Evidence Packages and accept, repair, or escalate work |
| Session artifacts | `session-artifact` | Read one exact registered review-safe artifact as bounded inline text without exposing a host path |
| Session output | `session-output` | Poll one exact Mission/Local Agent session's bounded, transient inspector-output page without exposing a host path or promoting raw bytes to canonical history |
| Shell | `shell-terminal`, `shell-terminal-submit`, decision commands, `additional-path-grant-create`, `additional-path-grant-deny` | Execute classified argv commands with explicit permissions and transient output; inspect typed pending contextual path requests and decide their exact boundary |
| Mission planning | `mission-drafts` and Mission Draft create/update/decision commands | Keep proposed mission work separate until confirmation |
| Audit | `activity-journal` | Query meaningful acknowledged actions without raw model or terminal bytes |

The desktop bridge starts a persistent `python3 -m albert_mvp.server` process and exchanges newline-delimited correlated CLI envelopes. It builds the same arguments as the one-process `python3 -m albert_mvp <command>` fallback. Transport persistence is an optimization, not an authority change.

`runner-observe` accepts an observer source/incarnation/positive sequence plus exact Mission, session revision, runner-operation, owner/process-group, Worktree Identity, and result boundaries. Signals use closed enums; exact-valid results require a digest, malformed values and cursor gaps fail before delivery, and reuse of one sequence for a different semantic boundary is rejected. An exact transport retry or semantic duplicate returns the same `SupervisionReceipt`. `no-change` is deliberately silent. Actionable delivery durably records attention, intent, receipt, and cursor before an independent canonical recheck can return `recovered`, `result-reconciled`, or `decision-needed`.

`retirement-preserve` accepts an exact session/Mission, required session `expected_revision`, and required request correlation id. Python claims the Retirement Unit before filesystem work, rejects nonterminal sessions, stale revisions, unbound reservations, ambiguous Worktree Identity, and any supervising-runner lease or process-group result other than independently corroborated `absent`. A successful response stores an exact replay receipt and returns the updated revision, verified/unbound Preservation Budget, and snapshot. `retirement-verify` re-reads the exact manifest, checks every authority/size/hash boundary, and reconstructs Git or managed-directory state in a disposable clean room without changing canonical state. These explicit commands do not themselves remove a worktree.

Outcome transitions invoke the same canonical lifecycle automatically. Accepted reviews and completed cancellations preserve and retire after quiescence; never-started cancellation persists exact no-runner proof and reconciles inline, while a known-live cancelled runner stays pending until runner finalization reconciles it. Terminal cancellation/failure before worktree creation preserves deterministic absence and applies retirement or grace in the same persistent process without materializing a workspace. Failed or rejected sessions enter passive grace; the common CLI option `--retention-grace-seconds` configures the mission policy and defaults to 72 hours. Needs-human-review remains active. Startup and later reconciliation resume `preserving`, `preserved`, `grace`, or `retiring` units idempotently. Removal gets an initial attempt, one automatic short-backoff retry, and one later reconciliation/startup retry before `retirement-blocked`; per-unit locking prevents concurrent effects. Git and managed-directory removal first moves the exact worktree to a deterministic app-local effect path, then validates and deletes only that isolated path; new bytes at the original path remain intact, same-user descriptors/cwd/root/shared mappings block deletion, and restart resumes interrupted isolated, split-move, or partial-marker Git effects. Exact-content comparison accepts only the preserved state or its safe partial-cleanup subset. A missing managed path with a lingering exact Git registration uses registration-only non-force removal. Repair first materializes the verified predecessor snapshot, retires that unit, and only then persists the separately authorized queued repair session.

`--snapshot-storage-retention-seconds` and `--snapshot-storage-budget-bytes` configure the common mission policy and default to 30 days and 5 GiB. Every new session admission counts retained payload bytes plus bound Preservation Budgets and reserves another 32 MiB inside one serialized admission transaction shared by Issue Slice, Ad Hoc, and headless launch paths. When needed, Python persists a reclamation intent and removes expired, unpinned, retired payloads oldest first. Startup and `retirement-storage` inspection sweep all eligible expired payloads, so inspection may advance only deterministic policy-driven reclamation state. Protected or pinned exhaustion persists `snapshot-storage-exhausted` attention and rejects admission. `retirement-storage` returns policy, payload/reserved/committed/available bytes, record/retained/pinned/reclaimed/expired-eligible counts, ordered expiry, ten largest retained payloads, reclamation history, and blockers. Agent Console `/status` includes the compact totals; Active-Mission `/storage` renders the deterministic inspection without model inference; Mission Work adds a prominent `retirement-storage` attention card until the blocker resolves.

`retirement-inspect` returns one exact Retirement Unit revision, terminal/phase/blocker state, Worktree Identity and runner boundary, Preservation Budget, compact Retirement Record, and only actually available actions; `preservation-blocked` cannot export before a verified payload exists. `retirement-pin` changes only a retained payload's pin state. `retirement-retry` durably authorizes one fresh bounded attempt from `preservation-blocked` or `retirement-blocked`. `retirement-export` requires a blocked unit with a verified retained payload and a nonexistent explicit destination; it persists intent before reconstruction and recovers a lost marker/receipt only after re-verifying exact exported content and Git state. `retirement-discard` requires a blocked terminal unit, exact session-id confirmation, reason, expected revision, fresh Runner Quiescence, no open process handles, deterministic managed containment, exact retained-content identity, and Coding Workspace exclusion before exact non-force deletion. All mutations require correlation ids, replay strictly validated action-specific receipts without repeating effects, reject changed-boundary reuse, serialize pinning with destructive effects, and share one-process/persistent/typed Workstation semantics. Mission Work exposes these actions against the session lifecycle revision; Workstation acknowledgement appends one correlated Mission/session-linked Activity Journal entry and recovers it idempotently after an effect-before-receipt crash.

During `npm run dev` or the dedicated `npm run dev:container` mode only, Vite exposes `POST /__alfredo/invoke` on the exact host origin `http://127.0.0.1:1420`. The browser sends `{id, command, args}` plus an injected per-process capability header. The gateway requires exact Host, Origin, method, media type, capability, body limit, command/id grammar, and three-field request before it writes one JSONL line to `alfredo-localhost-bridge`. Host mode additionally requires an exact loopback socket peer. Apple container port forwarding replaces that peer with the VM network address, so only the explicitly configured `apple-container` mode omits the peer-address assertion; `scripts/apple-container-dev` compensates by publishing guest 1420 solely to host `127.0.0.1:1420`, while every application-layer check remains unchanged. The Rust response is exactly one correlated `{id, ok, value}` or `{id, ok, error}` envelope. Unknown commands, extra typed arguments, raw argv, and browser-supplied authority fail closed. This endpoint is absent from builds and from Tauri's `1422` development mode; it is not a remotely authenticated API.

Gateway dispatch is bounded and correlation-based rather than response-order-based. One long `workstation_session_run` may execute beside control/status requests, so canonical polling and cancellation remain available while a Local Agent runs. The backend owns runner timeouts and cancellation; a browser transport deadline must not duplicate or silently terminate accepted work.

The gateway owns the transport process lifecycle as well as its request boundary. Normal shutdown terminates its complete Unix process group or Windows task tree. On Unix, bridge stdin owner loss triggers immediate termination of the previously verified dedicated Cargo/Rust/Python group before worker joins, so an active runner cannot keep the development bridge orphaned after an abnormal Vite exit.

## Bounded Session Output

`session-output` accepts the exact `mission_id`, `session_id`, and non-negative `after_sequence` cursor. Python is the only reader of the app-local per-session JSONL journal; it validates every retained record's schema, exact identity, contiguous sequence, UTF-8 byte limit, and runtime containment before returning a page of at most 256 events. Each returned event always includes one of `streaming | complete | failed` phases. A cursor newer than the retained sequence and malformed, cross-session, oversize, symlinked, or out-of-boundary data fail closed through the typed bridge.

`complete` means both that the Local Agent runner is terminal and that no retained output remains after the supplied cursor. A terminal journal larger than one page therefore reports `complete: false` until the final page is read. React subscribes only to the selected exact Mission/session pair, treats its first valid exact response—not callback installation—as subscribed, preserves already rendered output through a recoverable reader failure, limits automatic retries, exposes retry inline, and tears down the poller when the inspector closes, selection changes, unmounts, or the Active Mission changes. The journal is bounded to 128,000 bytes, never reports a filesystem path, is not an Evidence Package, and never enters Agent Console or the Activity Journal.

Before a Mission exists, `alfredo_launch_context` returns schema version 1, Starting Location, nullable Coding Workspace and Active Mission, and one of `selection-required | mission-choice-required | workspace-ready`. `coding-workspace-select` accepts `correlation_id`, exact `workspace_path`, and `existing | create`; its acknowledgement includes the canonical Starting Location, canonical Coding Workspace, null Active Mission, replay status, and human-readable message. `mission-options` exposes known Missions for that exact acknowledged workspace. `mission-choice` requires the current journey revision and either resumes one exact known Mission or creates one distinct Mission identity; `workspace-context` restores the acknowledged canonical workspace/Mission state. Tauri binds the first acknowledged repository immutably for that process, permits only exact correlation replay, rejects retargeting before another Python effect, and blocks all Mission-qualified commands until a validated choice acknowledgement names the requested Mission. Structured failures remain typed and recoverable where appropriate. No `workspace-snapshot` exists in selection-required or mission-choice-required state.

`workspace-snapshot.mission_board.issue_slices[]` includes both `tracker_status` and `work_type`. The desktop uses those authoritative metadata fields to project active AFK assignment work while excluding terminal history and human-only checks. This is a presentation filter only: it does not remove issues from the canonical mission graph or alter blocker evaluation. Queue creation APIs remain available to the prompt/controller path, while the Queue UI exposes only pending decision APIs. See the [2026-07-12 acceptance correction](../Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md).

## Request

Every persistent-server request contains a transport-only string `id` and an `argv` array containing the public CLI command plus its arguments. That envelope `id` correlates one request/response exchange and is not durable mutation identity. Mutating workspace commands separately carry a `correlation_id`, exact Mission identity, actor, target, normalized request boundary, and `expected_revision` shown to the user. Agent Console prompts and responses use the durable prompt `message_id` and the Mission-qualified scope captured when submitted. Runner dispatch carries both Mission and session identity so equal session ids in different Missions cannot collide.

Illustrative request:

```json
{
  "id": "ui-42",
  "argv": [
    "agent-console-response",
    "--target-repo", "/workspace/project",
    "--tracker-dir", "/workspace/project/.agent/issues",
    "--runtime-root", "/home/user/.alfredo/runtime",
    "--mission-id", "mission-alpha",
    "--message-id", "message-42",
    "--expected-revision", "12",
    "--scope-kind", "mission",
    "--scope-target", "mission-alpha",
    "--scope-label", "Mission Alpha",
    "--scope-mission-id", "mission-alpha",
    "--agent-id", "qwen3-14b"
  ]
}
```

Request validation rejects missing identities, malformed booleans, unknown commands/skills/agents, stale revisions, role-ineligible assignments, unapproved mutations, unsafe paths, and command boundaries that exceed declared access. Controller and worker authority is explicit: cloud, unavailable, gated, delegate-only, controller/router/frontier-routed, or metadata-incomplete capabilities cannot be selected as ordinary workers. Workstation session actions persist their request marker in the created session; approve, assignment, and cancellation persist it in the Mission runtime alongside the mutation. An exact retry checks that canonical marker before the stale-revision guard, completes or replays the preference acknowledgement, and idempotently restores missing Journal/Console audit phases. Workspace Queue keeps the exact proposal/decision request and acknowledgement together, projects their validated correlation ids on each item, validates its history-derived revision and semantic effect, and recovers an already-durable issue/session effect without accepting a contradictory later decision. A legacy Queue item with empty projected ids backfills only from one uniquely matching canonical receipt; if no exact chain exists, the id remains empty and React emits no conversational effect claim. Queue attention stays in Mission Work and is not duplicated as an Agent Console action claim; the receipt-bound Queue transcript is the conversational authority source. A projected Queue correlation that points elsewhere or an ambiguous legacy chain fails the persistence read. Mission Draft receipts additionally bind ordered prior/effect draft state, acknowledgement, accepted Issue identity, and decision reason into one canonical lifecycle chain. Reusing a correlation id with a different Mission, scope, goal, criteria, paths, policy, worker, origin, target, decision, actor, or draft state is rejected.

When Shell rejects an out-of-workspace boundary, the backend persists a typed contextual request containing `request_id`, correlation, Mission, canonical path, access level, 900-second duration, reason, affected action, and validated request time. `shell-terminal` projects that request and its derived `pending | granted | denied` status. Path-grant creation may include `request_id` and must exactly match the pending record; denial likewise binds the known request. Changed paths/access/duration, malformed timestamps, duplicate ids, or replayed decisions fail closed. React hydrates this projection directly and never reconstructs authority by parsing error prose.

Local model prompts run with sanitized environments and minimal Bubblewrap mounts; controllers and routers receive the repository read-only, while workers write only inside their isolated worktree and declared paths. `/tmp` executables and interpreter scripts must resolve to the exact validated regular file rather than a symlink escape. Aggregate stdout/stderr capture is bounded, child processes receive address-space/file-size/open-file/process-count limits, and overflow or timeout terminates the whole process group, including descendants that survive the leader. Git probes, path listings, worktree creation, diffs, and applies use the same bounded runner and fail closed on unexpected Git errors. Controller routes cap reply/task/criteria sizes. Worker plans cap file count, per-file and aggregate bytes, command count, and command length before any file is written or command runs.

`agent-capabilities` discovers each skill through a cumulative 64 KiB binary/UTF-8 front-matter read and stops at the closing delimiter. The non-symlink capability walk is capped at 20,000 visited entries and 1,024 matches. A large body is not loaded; oversized, unclosed, invalid, symlinked, or over-budget metadata is excluded without blocking the remaining catalog. Agent Console user input is capped at the shared 16,000-character durable boundary before transport.

## Response

The persistent envelope returns the same transport `id`, `success`, and captured `stdout`/`stderr`; successful stdout contains the typed JSON projection emitted by the selected CLI command. Canonical state projections carry the schema version and revision required by their own contract; response-only projections such as Agent Console routing or Session Artifact content intentionally omit an unrelated workspace revision. Mutations distinguish acknowledged, queued/pending, and rejected outcomes; a task acknowledgement is never presented as completed execution. Agent Console records preserve a single arrival-ordered chronology. Controller replies persist `action_outcome: no-action | awaiting-orchestrator` and the exact fixed `action_message` for that enum; append and read reject substituted copy. Neither value is an effect receipt, and commentary cannot carry `correlation_id`/`action_phase`. Canonical Console and Workstation acknowledgement events instead carry the exact returned correlation plus phase. Session projections expose launch only from one matching Queue/Workstation request and acknowledgement, evidence only from the persisted Mission-qualified Evidence Package identity, and review only from matching Review request, workspace event, and Journal decision.

`agent-console-response` returns the persisted assistant message, a typed route, and an optional Wayfinder projection. The Python response boundary resolves Wayfinder before controller routing: a new project or consequential change enters `chart`; a fresh Wayfinder map/ticket reference enters `work-through`; a durable active flow continues without nesting. Read-only explanation, status, review, diagnosis, and inspection do not automatically enter Chart mode. The projection carries `mode`, `gate.status` (`pending | open`), optional active-flow identity, and `turn_complete`. Gate opening is limited to an explicit Mission Commander confirmation or a visible persisted assistant record with `source: "wayfinder-agent"`; acknowledgement ends the turn and never auto-invokes a skill, creates an artifact, delegates, or launches production work. Pending-flow safe prompts continue through the normal command/controller route while retaining the active Wayfinder projection.

```json
{
  "message": {
    "message_id": "console-000043",
    "role": "assistant",
    "content": "Controller classified this prompt as a coding task. Untrusted reply prose was not retained; no action has occurred.",
    "outcome": "model-commentary",
    "action_outcome": "awaiting-orchestrator",
    "action_message": "Coding task route selected. No action has occurred until a correlated Orchestrator receipt is recorded."
  },
  "route": {
    "intent": "coding-task",
    "task_request": "Improve the polling reliability.",
    "acceptance_criteria": ["Polling recovers after a transient transport failure."]
  },
  "wayfinder": {
    "mode": "outside",
    "gate": {"status": "not-applicable"},
    "flow": null,
    "turn_complete": false
  }
}
```

The only route intents are `discussion` and `coding-task`. Invalid JSON, extra fields, blank or oversized values, invalid criteria, and unsupported intent values produce a fixed malformed-response discussion message and cannot preserve the model's prose. Valid model output also cannot make free-form reply prose authoritative: Alfredo discards the raw reply and persists a deterministic discussion or coding-route template while retaining only the bounded typed route fields. This structural boundary covers success idioms without maintaining an enumerable verb blacklist. Deterministic slash commands never become coding-task routes. Explicit delegation has narrow deterministic prefix and suffix forms—such as `Please ask a subagent to fix …` and `fix … with a subagent`—while questions, explanations, and ambiguous checks remain controller discussion. React renders commentary and its no-action/awaiting outcome separately from proposal, decision, queued, running, evidence, Review Decision, and accepted-completion events; each effect milestone displays its exact canonical correlation and phase. See the [false-success diagnosis](../Reports/2026-07-24-workspace-selection-false-success-diagnosis.md) and [Issue #59 implementation report](../Reports/2026-08-02-conversational-action-receipts.md).

When a Wayfinder flow has a pending gate, every Mission Draft mutation, `ad-hoc-delegation-proposal`, approval of an Ad Hoc Delegation, legacy `route`/`approve-delegation`, `workstation-action` session launch, direct `launch`/TUI launch-or-repair, and headless worker command rejects with the recoverable stable error `shared-understanding-required` before it mutates canonical state. One shared loader reports malformed state with structured `persistence-read-failure`. Read-only/discovery boundaries remain available.

Illustrative response:

```json
{
  "id": "ui-42",
  "success": true,
  "stdout": "{\"message\":{\"message_id\":\"console-000043\",\"outcome\":\"model-commentary\",\"role\":\"assistant\"},\"route\":{\"intent\":\"discussion\",\"task_request\":\"\",\"acceptance_criteria\":[]}}\n",
  "stderr": ""
}
```

Failures return `success: false`; `stderr` contains a structured error with a stable code, human-readable message, recovery flag, and details where useful. Current contract examples include `stale-action`, `revision-gap`, `scope-mismatch`, `persistence-read-failure`, and `contract-failure`; domain and policy failures use the same structured bridge mapping. The UI preserves the last acknowledged projection, reports the failure inline, and reloads a canonical snapshot after revision gaps or reconnects. Shell submission and decisions poll canonical metadata while a process runs. A lost response reloads the exact correlation instead of creating a new command; `executing` remains visible, while a dead execution becomes durable `outcome-unknown`, is attributed in Console/Activity, and is never retried automatically.

Workspace session summaries expose validated `last_activity_at` from terminal
session timestamps and a distinct validated `runner_started_at`. The latter is
the only rendered R6 runner-claim source; a conversational running milestone
also requires `launch_correlation_id`. Validated evidence projects
`evidence_correlation_id`, and a persisted Review Decision projects
`review_correlation_id`; only Approved outcomes may add a separate accepted-
completion phase. Later cancellation, completion, or general activity cannot
masquerade as runner start. Malformed values fail the projection and absent
values remain absent. UI cancellation maps to terminal unsuccessful/failed
presentation rather than a completed card. A valid
Workstation request that reaches Python and is rejected persists durable Console
`request`/`rejection` phases. A transport failure before Python receives the
request cannot claim a backend audit event; React may retain only its bounded
negative outcome for refresh continuity.

`workstation-action` accepts `issue-archive` and `issue-restore` for an exact
Mission-qualified Issue Slice, alongside cancellation, review, retry, repair,
and assignment actions. Archive accepts `pr-ready`/complete work or a
tracker-merged Issue Slice and
returns a correlated acknowledgement that explicitly says history remains
inspectable; restore returns the same retained identity and history to active
Mission Work. React sends the typed action only after showing its exact
consequence, and renders success only from the matching acknowledgement. A
repair action projects a bounded inherited task-packet preview but still
launches only the single canonical repair command. Blocker recommendations are
read-only snapshot fields and state their rationale, proposed acceptance,
assigned actor, and that no follow-up approval can unblock the original work.

## Performance measurement boundary

Measurement is opt-in through either the complete legacy
`ALFREDO_MEASUREMENT_*` identity for a new process or one absolute
`ALFREDO_MEASUREMENT_CONTROL_PATH` for a persistent warm desktop. The two forms
cannot be combined. An absent control file leaves bootstrap unmeasured; each
later command rereads the atomically replaced regular non-symlink JSON object.
No variables means normal product behavior; a partial identity fails closed.
Launcher, native, React, and Python append bounded JSON Lines stage marks using
their own monotonic clocks. Cross-process clocks are never subtracted.

`performance_mark` accepts frontend- or native-owned S0-S9/R0-R6 marks and
returns only whether a mark was recorded. The production cohort driver owns S0,
verifies a clean committed Git archive and exact packaged artifact hashes,
executes balanced randomized AB/BA pairs sequentially, and preserves fixture
proofs plus gate evidence. Warm records carry the native desktop PID and stable
desktop-session id, and R0-to-R5/R6 uses one frontend clock. Contract, replay,
crash-cut, packaging, and rollback
records must bind the same run, variant, cohort, fixture, source, artifact, and
fixed repository gate-runner hash. Any mismatch or failed gate invalidates the
associated sample.

Raw model streams, worker prompts/responses, terminal bytes, test logs, and diffs are not returned in canonical snapshots. They stay transient or in bounded per-session artifacts; finalized Agent Console turns use the separate history endpoint, while Evidence Packages link safe artifacts and redact non-Normal file content. Evidence controls are projected only for registered references accepted by the bounded reader (`app-local://...` or the supported opaque artifact form); unsupported relative references are omitted rather than exposed as dead controls.

`session-artifact` requires `--artifact-mission-id`, `--session-id`, and `--artifact-ref`. The reference must be registered to that exact session and review-safe. A regular non-symlink text file must resolve below the session runtime directory; runtime Evidence Package references are projected from structured state. The result contains `artifact_id`, label, media type, content, returned byte count, the 128,000-byte limit, and a truncation flag—never a path or the submitted reference. Stable failures are `session-artifact-not-found`, `session-artifact-forbidden`, `session-artifact-unsupported`, and `session-artifact-unavailable`; the UI renders these inline and retries only recoverable failures.

## Security and Authentication

Alfredo has no remote authentication endpoint. The development localhost capability defends a loopback transport; it does not grant domain authority. Authority comes from the local Mission Commander action, exact expected revision, accepted Issue Slice or approved queue item, configured agent role, command policy, and explicit path grants. The Python Orchestrator enforces all of these constraints even when callers bypass React or Tauri.
