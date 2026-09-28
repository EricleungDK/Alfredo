# API Endpoints and Command Boundaries

**Last Updated:** 2026-08-30

Alfredo has no remote or production HTTP API. Its application boundary is a versioned JSON command protocol shared by the React client, the Tauri bridge, the development-only localhost gateway, the persistent Python server, and the one-process Python CLI fallback. Python remains authoritative; Rust validates and transports typed payloads, while React renders acknowledged projections.

## Native Rust terminal boundary

The separate `alfredo-tui` binary owns its own task store and calls Ollama directly.
`--doctor` runs without terminal initialization: the ordinary stores validate named
conversation state, GET /api/tags checks its restored model, sanitized bounded Git
reads inspect the worker baseline/config, and metadata checks locate required tools.
Exit 0/2 indicates passed/failed preflight checks. It may create private lock
namespaces, but saves no conversations/task receipts and sends no inference.
`/models` calls `GET /api/tags` under ten-second/1-MiB/256-entry bounds. `/model NAME`
selects a catalog model for an idle, non-interrupted conversation; it does not change
existing task assignments. Discovery failures retain the last catalog with an error.
Its `/task`, `/after`, `/permit`, `/approve`, `/run`, `/cancel-task`, `/evidence`,
`/recover`, `/repair`, `/accept`, `/reject` and `/branch` commands do not call the Python protocol described below.
Mutations carry expected revision and correlation identity. Dependent /run requests
first verify accepted-parent candidates and compose an isolated Git baseline with
ancestry/conflict checks. Schema-v4 run receipts bind input task/run/digest/candidate
identities; the store rechecks evidence at claim. Preparation has a 60-second
deadline/cancellation and changes only derived Git objects/managed refs; model,
worktree and check execution follow the durable run claim. Explicit file/check
policy plus fresh approval is required before a durable run claim; terminal
completion binds retained evidence, and review verifies that evidence again.
`/branch ID` verifies an accepted candidate and confirms/creates its deterministic
local ref without checkout or overwrite, then records a schema-v5 Branch receipt.
An exact existing ref permits receipt reconciliation; different targets refuse.
Opening `/evidence ID` also selects that exact task, keeping displayed evidence and
ID-less review commands aligned. The terminal renders parsed check/diff/output
sections after store verification; rendering grants no new authority.
`/recover ID` acquires an existing released worker-owner lock and acknowledges a
valid saved result without replaying effects; missing/invalid evidence preserves
the uncertain claim. Refresh projects advisory ownership/recoverability.
`/activity [query or #ID]` reads/searches canonical task receipts without creating
a task mutation or conversation action claim.
Native discussion, planning and worker inference share same-user capacity per
normalized endpoint origin (default two, `--parallel-models` 1–8), independent of
mission state directories. Foreground discussion/planning has bounded priority over
background workers, without preemption. A live capacity conflict refuses. Queued
requests remain cancellable before HTTP dispatch; discovery bypasses admission.
`Queued` and changed `QueueProgress` observations precede `Admitted`, which follows
post-capacity planner/task guards. The UI distinguishes shared Alfredo queueing
from upstream waiting; neither proves GPU capacity or server-side cancellation.
Workers recheck cancellation and the exact captured Running task/run, policy and
model after capacity admission. They accept unrelated task-store revision changes
and await provider-future cancellation before publishing Finish. The client `queue`
timing spans request preparation and validation through admission, not only shared
capacity waiting.
`--qualify-inference REPORT [--qualification-repetitions 1..3]` runs a separate
diagnostic cohort without terminal initialization or an active user mission. It
defaults to three repetitions of four scenarios paired across baseline and explicit
foreground-8192/background-16384 context profiles, with fixed shared capacity one,
128 maximum generation requests and a 1,800-second cohort deadline plus cleanup.
`--inspect-qualification REPORT` validates and summarizes only; it sends no HTTP and
never resumes an incomplete report. Existing report or artifact paths refuse.
Artifacts are retained at `<REPORT>.artifacts`. Production defaults remain unchanged.

Recorded qualification generations retain exact payload/profile hashes and bounded
message/prefix identities. Before `Done`, while their shared permit remains held,
they inspect `/api/version`, `/api/tags` and `/api/ps` through read-only concurrent
requests, each with a ten-second deadline and 1-MiB ceiling; model lists allow at
most 256 entries. These probes do not acquire another admission ticket. Ordinary
unrecorded calls do not probe. Generation time, probe duration and instrumented total
are separate observations. Missing metadata or probe failure cannot produce runtime
qualification, even when generation itself completes. Runtime pins/token headroom
remain unverified; inspection hashes are neither reconstructable prompts nor upstream
attestations. No automatic profile promotion or performance claim follows.
See [terminal commands](../../alfredo-tui/README.md) for syntax and current limits.
The shared Rust execution callbacks optionally observe stdout/stderr through a
nonblocking 32-by-4-KiB channel. This changes no request/receipt schema or desktop
JSONL output; desktop callers leave the observer absent. Terminal live tails are
advisory and saved execution receipts remain authoritative.
The existing execution validator accepts optional `--unshare-net`; terminal workers
always request it, while desktop callers retain their existing boundary.

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
| Local inference governance | normal Ollama route/worker turns, `workspace-snapshot`, `workstation-session-run` | Resolve versioned Local Inference Profiles, admit bounded turns, serialize capacity through one non-authoritative Lease, and project validated receipts/telemetry |
| Local inference qualification | `inference-qualification`, `inference-qualification-promote`, `inference-qualification-rollback` | Inspect bounded governed Profile reports, promote an exact non-withdrawn runtime pin after quality/reliability gates, and replay-safe rollback to the retained prior pin |
| Shell | `shell-terminal`, `shell-terminal-submit`, decision commands, `additional-path-grant-create`, `additional-path-grant-deny` | Execute classified argv commands with explicit permissions and transient output; inspect typed pending contextual path requests and decide their exact boundary |
| Mission planning | `mission-drafts` and Mission Draft create/update/decision commands | Keep proposed mission work separate until confirmation |
| Rust shadow execution | `alfredo-execution-provider` JSONL process (app-local candidate only) | Compare typed Rust execution receipts against Python on production-equivalent fixtures without granting Rust canonical write authority |
| Audit | `activity-journal` | Query meaningful acknowledged actions without raw model or terminal bytes |

The desktop bridge starts a persistent `python3 -m albert_mvp.server` process and exchanges newline-delimited correlated CLI envelopes. It builds the same arguments as the one-process `python3 -m albert_mvp <command>` fallback. Transport persistence is an optimization, not an authority change.

`runner-observe` accepts an observer source/incarnation/positive sequence plus exact Mission, session revision, runner-operation, owner/process-group, Worktree Identity, and result boundaries. Signals use closed enums; exact-valid results require a digest, malformed values and cursor gaps fail before delivery, and reuse of one sequence for a different semantic boundary is rejected. An exact transport retry or semantic duplicate returns the same `SupervisionReceipt`. `no-change` is deliberately silent. Actionable delivery durably records attention, intent, receipt, and cursor before an independent canonical recheck can return `recovered`, `result-reconciled`, or `decision-needed`.

`retirement-preserve` accepts an exact session/Mission, required session `expected_revision`, and required request correlation id. Python claims the Retirement Unit before filesystem work, rejects nonterminal sessions, stale revisions, unbound reservations, ambiguous Worktree Identity, and any supervising-runner lease or process-group result other than independently corroborated `absent`. A successful response stores an exact replay receipt and returns the updated revision, verified/unbound Preservation Budget, and snapshot. `retirement-verify` re-reads the exact manifest, checks every authority/size/hash boundary, and reconstructs Git or managed-directory state in a disposable clean room without changing canonical state. These explicit commands do not themselves remove a worktree.

Outcome transitions invoke the same canonical lifecycle automatically. Accepted reviews and completed cancellations preserve and retire after quiescence; never-started cancellation persists exact no-runner proof and reconciles inline, while a known-live cancelled runner stays pending until runner finalization reconciles it. Terminal cancellation/failure before worktree creation preserves deterministic absence and applies retirement or grace in the same persistent process without materializing a workspace. Failed or rejected sessions enter passive grace; the common CLI option `--retention-grace-seconds` configures the mission policy and defaults to 72 hours. Needs-human-review remains active. Startup and later reconciliation resume `preserving`, `preserved`, `grace`, or `retiring` units idempotently. Removal gets an initial attempt, one automatic short-backoff retry, and one later reconciliation/startup retry before `retirement-blocked`; per-unit locking prevents concurrent effects. Git and managed-directory removal first moves the exact worktree to a deterministic app-local effect path, then validates and deletes only that isolated path; new bytes at the original path remain intact, same-user descriptors/cwd/root/shared mappings block deletion, and restart resumes interrupted isolated, split-move, or partial-marker Git effects. Exact-content comparison accepts only the preserved state or its safe partial-cleanup subset. A missing managed path with a lingering exact Git registration uses registration-only non-force removal. Repair first materializes the verified predecessor snapshot, retires that unit, and only then persists the separately authorized queued repair session.

`--snapshot-storage-retention-seconds` and `--snapshot-storage-budget-bytes` configure the common mission policy and default to 30 days and 5 GiB. Every new session admission counts retained payload bytes plus bound Preservation Budgets and reserves another 32 MiB inside one serialized admission transaction shared by Issue Slice, Ad Hoc, and headless launch paths. When needed, Python persists a reclamation intent with the descriptor-proven payload-root identity and removes expired, unpinned, retired payloads oldest first through exact descriptor-relative entry checks; root or entry substitution fails closed without deleting replacement bytes. Legacy crash-left intents acquire that identity safely before replay. Startup performs the same eligible sweep. Explicit unpin changes policy eligibility and may clear stale protected-exhaustion attention, but never deletes payload bytes; the next startup or admission reclaims them. `retirement-storage`, Agent Console `/storage`, and `/status` are deterministic read-only projections. Protected or pinned exhaustion persists `snapshot-storage-exhausted` attention and rejects admission. `retirement-storage` returns policy, payload/reserved/committed/available bytes, record/retained/pinned/reclaimed/expired-eligible counts, ordered expiry, ten largest retained payloads, reclamation history, and blockers. Agent Console `/status` includes the compact totals; Active-Mission `/storage` renders the deterministic inspection without model inference; Mission Work adds prominent `retirement-storage` attention until the blocker resolves.

`retirement-inspect` returns one exact Retirement Unit revision, terminal/phase/blocker state, Worktree Identity and runner boundary, Preservation Budget, compact Retirement Record, and only actually available actions; direct retained-source export and discard remain hidden until both runner-owner and process-group evidence are absent, while snapshot export remains independent of the retained source. `retirement-pin` changes a retained payload's compact pin-policy field and advances the session revision. `retirement-retry` durably authorizes one fresh bounded attempt from `preservation-blocked` or `retirement-blocked`; preservation failures run preservation directly before outcome policy is reapplied. `retirement-export` requires a blocked unit and a nonexistent explicit destination outside the retained source and complete app-private runtime root for both snapshot and direct exports. It reconstructs a verified retained Snapshot Payload when available; for `preservation-blocked`, it instead exports the exact independently quiesced managed path with bounded entry, mode, byte, directory, and symlink proof while excluding only a proven live Git administration pointer. Publication permanently claims the normalized destination, materializes through a descriptor-bound private sibling stage, durably reserves at most two stage attempts, and publishes no-replace only after exact repository/marker verification; source, parent, runtime, marker, and top-level identities are reproved before the receipt. Exact legacy intents and complete legacy outputs are replayed conservatively, while partial or foreign data is preserved untouched. `retirement-discard` requires a blocked terminal unit, exact session-id confirmation, reason, expected revision, fresh Runner Quiescence, recursive cross-filesystem open-handle absence, deterministic managed containment, exact root identity and full bounded retained-tree manifest, and Coding Workspace exclusion. Snapshot-backed deletion remains exact and non-force; an explicitly confirmed no-snapshot discard may remove the exact proven Git worktree or delete the exact isolated managed path when Git metadata is already broken. Direct tree deletion binds the isolated root descriptor and revalidates each named entry around descriptor-relative unlink/rmdir, so a replacement is preserved and blocks completion. Partial deletion replay accepts only an exact subset of the authorized manifest, and broken-Git fallback removes and verifies its exact registration. All mutations require correlation ids, replay Mission/session-bound action-specific receipts without repeating effects, reject changed-boundary reuse, serialize pin, retry, export, and discard through one per-session effect lock, and share one-process/persistent/typed Workstation semantics. Public inspection commands load without startup reconciliation, so `/storage`, status, board, and other reads cannot reclaim payloads. Mission Work exposes actions against the session lifecycle revision; the inspector renders phase, blocker, runner, budget, and record facts in human-readable fields. Workstation acknowledgement appends one correlated Mission/session-linked Activity Journal entry and recovers it idempotently after an effect-before-receipt crash.

During `npm run dev` or the dedicated `npm run dev:container` mode only, Vite exposes `POST /__alfredo/invoke` on the exact host origin `http://127.0.0.1:1420`. The browser sends `{id, command, args}` plus an injected per-process capability header. The gateway requires exact Host, Origin, method, media type, capability, body limit, command/id grammar, and three-field request before it writes one JSONL line to `alfredo-localhost-bridge`. Host mode additionally requires an exact loopback socket peer. Apple container port forwarding replaces that peer with the VM network address, so only the explicitly configured `apple-container` mode omits the peer-address assertion; `scripts/apple-container-dev` compensates by publishing guest 1420 solely to host `127.0.0.1:1420`, while every application-layer check remains unchanged. The Rust response is exactly one correlated `{id, ok, value}` or `{id, ok, error}` envelope. Unknown commands, extra typed arguments, raw argv, and browser-supplied authority fail closed. This endpoint is absent from builds and from Tauri's `1422` development mode; it is not a remotely authenticated API.

Gateway dispatch is bounded and correlation-based rather than response-order-based. One long `workstation_session_run` may execute beside control/status requests, so canonical polling and cancellation remain available while a Local Agent runs. The backend owns runner timeouts and cancellation; a browser transport deadline must not duplicate or silently terminate accepted work.

The gateway owns the transport process lifecycle as well as its request boundary. Normal shutdown terminates its complete Unix process group or Windows task tree. On Unix, bridge stdin owner loss triggers immediate termination of the previously verified dedicated Cargo/Rust/Python group before worker joins, so an active runner cannot keep the development bridge orphaned after an abnormal Vite exit.

## Bounded Session Output

`session-output` accepts the exact `mission_id`, `session_id`, and non-negative `after_sequence` cursor. Python is the only reader of the app-local per-session JSONL journal; it validates every retained record's schema, exact identity, contiguous sequence, UTF-8 byte limit, and runtime containment before returning a page of at most 256 events. Each returned event always includes one of `streaming | complete | failed` phases. A cursor newer than the retained sequence and malformed, cross-session, oversize, symlinked, or out-of-boundary data fail closed through the typed bridge.

`complete` means both that the Local Agent runner is terminal and that no retained output remains after the supplied cursor. A terminal journal larger than one page therefore reports `complete: false` until the final page is read. React subscribes only to the selected exact Mission/session pair, treats its first valid exact response—not callback installation—as subscribed, preserves already rendered output through a recoverable reader failure, limits automatic retries, exposes retry inline, and tears down the poller when the inspector closes, selection changes, unmounts, or the Active Mission changes. The journal is bounded to 128,000 bytes, never reports a filesystem path, is not an Evidence Package, and never enters Agent Console or the Activity Journal.

Before a Mission exists, `alfredo_launch_context` returns schema version 1, Starting Location, nullable Coding Workspace and Active Mission, and one of `selection-required | mission-choice-required | workspace-ready`. `coding-workspace-select` accepts `correlation_id`, exact `workspace_path`, and `existing | create`; its acknowledgement includes the canonical Starting Location, canonical Coding Workspace, null Active Mission, replay status, and human-readable message. `mission-options` exposes known Missions for that exact acknowledged workspace. `mission-choice` requires the current journey revision and either resumes one exact known Mission or creates one distinct Mission identity; `workspace-context` restores the acknowledged canonical workspace/Mission state. Tauri binds the first acknowledged repository immutably for that process, permits only exact correlation replay, rejects retargeting before another Python effect, and blocks all Mission-qualified commands until a validated choice acknowledgement names the requested Mission. Structured failures remain typed and recoverable where appropriate. No `workspace-snapshot` exists in selection-required or mission-choice-required state.

`workspace-snapshot.mission_board.issue_slices[]` includes both `tracker_status` and `work_type`. The desktop uses those authoritative metadata fields to project active AFK assignment work while excluding terminal history and human-only checks. This is a presentation filter only: it does not remove issues from the canonical mission graph or alter blocker evaluation. Queue creation APIs remain available to the prompt/controller path, while the Queue UI exposes only pending decision APIs. See the [2026-07-12 acceptance correction](../Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md).

## Local Inference Governance

Normal configured Ollama entries use the bounded HTTP adapter. Before `/api/generate`, Python resolves the exact installed model digest and quantization metadata, records the versioned Profile, sends the composed prompt with `raw: true`, and rejects it when the tokenizer-independent UTF-8 byte token bound plus the output budget exceeds the context budget. Requested GPU/CPU placement is carried in `options.num_gpu`. A Lease admission then records one active or queued request with Mission/session identity, priority, sequence, residency, and model digest. Queue selection is priority-first, qualified normal-resident affinity second, and FIFO third; cancellation and queue wait are bounded.

Each turn records the exact Profile, admission/headroom, load/prompt-evaluation/first-token/decoding timings, usage, Lease snapshot, and one structured outcome. Response and thinking bytes share the same total-output bound. A candidate completion must also expose all required Ollama usage/timing metrics and pass a bounded `/api/ps` check that matches the resolved running-model digest and records requested placement plus exact GPU/total bytes. Only that runtime-bound complete stream, valid JSON, declared schema, and accepted domain value has `authoritative: true` and `outcome: completed`; only then may the Lease record the model as resident for affinity. Partial, malformed, oversized, timed-out, cancelled, transport, queue, lease, metadata, and digest outcomes remain non-authoritative and cannot route work or mutate Mission state. The explicit configured command path is compatibility/test-only and does not replace this normal HTTP boundary.

`workspace-snapshot` projects aggregate `mission_board.inference` with `turn_count`, the public last receipt, and the Lease state (`active`, bounded `queued`, `resident`, and bounded `audit`). Each `MissionSessionSummary.inference` contains only that exact session's turn count/state/last receipt. `workstation-session-run` returns exact-session public turns and the current Lease. Public receipts omit prompts and raw streams; runtime persistence validates schema, identity, timestamps, bounded sizes, Profile canonicality, admission, timings, usage, and Lease shape on write and reload. See the [Issue #69 Local Inference Governance report](../Reports/2026-08-13-issue-69-local-inference-governance.md).


The `albert_mvp.inference_qualification` public seam runs repeated governed fixture cohorts without granting model output authority. `InferenceQualificationService` reports valid routes/plans/evidence, accepted outcomes, repairs, escalations, policy blocks, cancellation, model swaps, queued Local Agents, decomposed stage timings, and goal-to-reviewed-Evidence-Package latency. Context comparison uses bounded controller/normal-worker profiles, output-headroom admission, digest-keyed deterministic source selection, and exact-prefix reuse/invalidation observations. `QualificationReportStore` persists only bounded metadata below `runtime_root/inference/qualification/`; its promotion state pins the exact Profile and non-withdrawn runtime/binary/configuration digests, rejects quality/reliability regression, supports exact replay, and retains a rollback action. It never persists prompts, raw streams, plans, Evidence Packages, authority decisions, or source-dependent outcomes as reusable truth. See the [Issue #70 qualification report](../Reports/2026-08-13-issue-70-inference-qualification.md).

## Shadow Rust Execution Receipts (Issue #72)

The app-local `alfredo-execution-provider` consumes the same versioned `ExecutionRequest` JSON object as Python and emits one typed `ExecutionReceipt` or structured failure per JSONL request. `execution_shadow.py` hashes every canonical store before and after a sample, compares a normalized receipt projection, and records crash or parity uncertainty without transferring authority. Cohorts bind exact fixture/source/artifact digests, production-equivalent stages, and explicit sample metadata; reducer, sidecar, and microbenchmark evidence is rejected. Rust eligibility is persisted separately and fails closed on parity, canonical-store, crash-cut, state-version, packaging, release-gate, production-equivalence, or stage-measurement failure.

The Rust validator follows the Python version-1 prepared-process contract exactly: the first `--` closes Bubblewrap options, a second exact separator belongs to the required trusted `prlimit` wrapper, and the wrapper values must equal the request limits. Only the fixed `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, and `/etc` read-only source/destination pairs may retain their lexical mount aliases for merged-`/usr` compatibility; all authority-declared and other implicit paths remain canonical. Generic command arguments cannot create implicit host-read mounts; only governed `PATH` executables and private temporary interpreter scripts may add executable/script bindings. Resource maxima, sanitized environment keys/byte bounds, and duplicate allowed-path rejection match Python. See the [Issue #72 report](../Reports/2026-08-15-issue-72-rust-shadow-execution.md#linux-acceptance-correction-2026-08-30).

On supported Linux releases, the platform npm package includes `bin/alfredo-execution-provider` and binds its SHA-256 in `desktop.json`. A publishable `release:verify` install must run exactly thirteen named/statused cohorts through the installed Python backend and exact installed provider: Local Agent on the immediately previous one-response and current streamed protocols, Shell on those same two protocols, failure, timeout/cleanup, output limit, cancellation, Python-authoritative replay, provider crash, resource validation, sandbox validation, and state-version rejection. Every expected outcome and normalized receipt must match; current-protocol samples must stream the exact effect-child binding; each Rust sample must leave both canonical roots byte-unchanged; timeout must prove absence of a delayed descendant marker for both providers; crash must pass the complete normalized comparison; explicit zero flags must select the packaged Python provider for both effects before claim; and the consumer must recompute the suite digest. `release:check` re-extracts and rehashes the provider from the exact platform tarball. Eligibility reopens the complete production manifest, recomputes SHA-256 and npm SHA-512, validates both package identities, the exact meta dependency and aliases, and complete AppImage/provider desktop metadata, then revalidates the external provider plus manifest hashes on load. This release evidence does not transfer Mission, scheduler, journal, or reconciliation authority from Python.

## Host Execution Request/Receipt

Local Agent runner commands and Shell Terminal commands continue to enter through their existing Python policy seams, but both now hand one exact effect to `albert_mvp.execution`. `ExecutionRequest` is schema-versioned, argv-only, `shell=False`, and binds the effect-specific authority input: Local Agent Mission/session revision, runner operation, Worktree Identity, and allowed paths; or Shell Mission/command/correlation, classification, requester, approval actor, working directory, requested paths, and access level. The request also carries the already-resolved Bubblewrap argv, sanitized environment, bounded input digest, timeout/output/resource limits, and filesystem boundary.

### Local Agent Rust provider selection (Issue #73)

The public Local Agent command/model/test seams are unchanged. Immediately before the shared coordinator validates and claims their prepared request, `local_agent_execution_provider_from_environment` selects Python or the exact integrity-verified Rust binary. Installed desktop and `alfredo run`/`alfredo review` launch paths resolve the same packaged provider identity. `ALFREDO_RUST_CANDIDATE_ENABLED=1` and `ALFREDO_RUST_LOCAL_AGENT_ENABLED=1` are both required for Rust. Either flag at `0` selects Python before optional native-package resolution; an unavailable or mismatched selected artifact may select Python only during the selector's pre-claim proof window.

The current JSONL provider emits a `process-started` event containing the effect child's PID and start identity, followed by one typed receipt. Python journals the binding before completion and polls cancellation while the provider is live. Streaming is negotiated through the provider environment so the immediately previous one-response JSON provider remains readable. Any untrustworthy post-launch transport result becomes a Rust `outcome-unknown`; exact replay is provider-free and never invokes Python as a retry.

### Shell Rust provider selection (Issue #74)

The public Shell Terminal CLI, persistent-server, Tauri, WorkspaceClient, and React shapes are unchanged. After Python classifies the command, applies approval and Additional Path Grant policy, resolves canonical mounts and sanitized environment, and builds the exact Bubblewrap/resource argv, `shell_execution_provider_from_environment` selects one provider. Rust requires both `ALFREDO_RUST_CANDIDATE_ENABLED=1` and `ALFREDO_RUST_SHELL_ENABLED=1`, an exact provider SHA-256 matching the installed adapter's release-qualified digest, and no persisted `RustEligibilityStore` circuit breaker. If an eligible shadow/release decision exists, its verified release evidence must name that same provider digest. Either explicit zero or an ineligible/corrupt decision selects Python before validation or journal claim. Invalid selected-Rust configuration fails closed without falling through to Python. Typed Rust transport/contract failures persistently disable the candidate for later correlations while the uncertain correlation stays bound to Rust.

The coordinator records the selected provider in the durable per-Mission receipt. Exact replay, including replay after the feature flag changes, returns that receipt without provider invocation; a changed command under the same correlation remains a conflict. The streamed transport preserves live child binding, cancellation, malformed/crashed-provider uncertainty, and immediately previous one-response compatibility. The installed adapter validates the exact provider only while at least one Rust effect is enabled; disabling the global candidate and both effects keeps the packaged Python rollback operable even when the candidate artifact is unavailable. See the [Issue #74 report](../Reports/2026-08-30-issue-74-shell-rust-cutover.md).

`ExecutionCoordinator` validates authorization and the prepared boundary before claiming the intent, durably records deterministic pre-effect failures as typed `start-failed`, then persists the claim before the provider call. `PythonExecutionProvider` delegates to the existing bounded runner, preserving process supervision, cancellation, Bubblewrap and environment enforcement, resource limits, process-tree cleanup, timeout, and bounded output. A POSIX request must contain a structurally valid prepared Bubblewrap/prlimit argv with the declared bindings, fixed system roots or exact regular non-symlink executable/script implicit binds, protected writable roots excluded, and the exact resource boundary; the builder and validator receive the same governed `PATH`. A sandbox description alone cannot authorize an unsandboxed launch, and injected executors cannot bypass this validation. Environment keys/values and resource limits are bounded by the contract. `ExecutionReceipt` is the typed reconciliation seam. The per-Mission `ExecutionJournal` is durable and lock/atomic-write protected, stores no raw output or prompt bytes, rejects raw fields on reload, binds child PID/start identity immediately after `Popen`, returns exact terminal receipts on replay, rejects a changed boundary, fails closed on missing schema/receipt identity, and converts dead in-flight owners or provider crash cuts to `outcome-unknown` requiring human/system reconciliation. Shell metadata and Local Agent lifecycle remain the canonical projections: Shell inspection repairs a terminal projection from the submitting Mission's completed journal receipt, with a read-only and idempotent legacy app-level ledger fallback, and Local Agent startup repairs the bounded session receipt history only after matching the durable request/session authority; missing or mismatched bindings fail closed. Automatic runner recovery waits for reconciliation when a Local Agent receipt is executing or uncertain. Deterministic sandbox/command preflight failures are typed before any child starts, Shell keeps public start-failure replay at exit code `126` while the typed receipt remains `127`, Darwin cleanup fails closed when process-group quiescence cannot be verified, and non-POSIX host effects fail closed. The CLI, persistent transport, and desktop keep their existing command/response shapes.

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

### Native terminal completion timing

Completion timing is shown as **Server timing** in the conversation, plan preview
and verified worker evidence when Ollama supplies it. Load, prompt evaluation,
generation and total durations are optional; generated tokens/s uses only reported
generation duration and token count. These are server observations, separate from
client queue and first-content latency. Invalid fields are ignored independently
(durations above ten minutes and counts above one million are omitted), and missing
metrics do not fail an otherwise complete response. Conversation and plan timing is
transient; worker timing is retained in digest-bound evidence. It grants no approval
or check success. See [Ollama chat response metrics](https://docs.ollama.com/api/chat).

## Thinking-stream progress

Ollama's [thinking protocol](https://docs.ollama.com/capabilities/thinking) can emit
reasoning separately from answer content. The native provider accepts thinking-only
frames and emits one payload-free progress event per request. Conversation status
shows `Thinking / waiting for text`, the planner shows `Model thinking`, and a coding
worker shows `Thinking` until answer content arrives. Reasoning text is neither
shown as an answer nor retained in conversation, plan or worker output. Thinking and
answer bytes share the existing 128-KiB output budget; individual frames remain
limited to 64 KiB. This preserves bounded transport while supporting thinking models.

Thinking status is transient, attempt-bound and cleared by retry/restart; late events
cannot revive completed or cancelled work. First-content timing still measures answer
text, not hidden reasoning. This change observes the phase without changing the
model's thinking configuration, token budget or execution authority.

## Native Wayfinder first-contact routing

The Rust `wayfinder` adapter runs before conversational model dispatch. It ports the
legacy deterministic entry vocabulary: new projects/consequential changes enter Chart,
Wayfinder map/ticket/issue references enter Work-through, and ordinary read-only
explanation/status/review/diagnosis/inspection stays outside. Existing project scope
continues across missions and restarts without another flow entry. Model continuations
receive captured scope as reference; model prose cannot mutate or confirm scope.

A flow entry records the originating prompt (at most 16 KiB), a mode and a pending
brief with explicit unknowns. It cannot be confirmed until the Commander supplies a
scope draft. Four complete labeled lines (Destination, Scope, Constraints, Uncertainty)
save a bounded draft against the observed revision. `confirm shared understanding N`
requires the exact draft revision and records agreement only. These deterministic
responses are receipt-backed and end the turn without model inference or task actions.
Manual `/scope` commands remain available. Ambiguous/malformed field text stays
conversation; refused writes are not acknowledged as successful actions.

Routing writes are tracked independently of cancellable inference jobs. Cancellation
cannot abandon a pending receipt: polling still observes the outcome; switching and
normal quit wait for routing completion. Dispatch pauses while routing is pending,
and a newly pending gate turns it off. Safe inspection/reconciliation commands remain
available while new task actions wait for the routing result.

## Generation limits

A response ending with Ollama `done_reason: "length"` is incomplete. The terminal
retains partial answer text and bounded server metrics, but marks the turn failed
instead of complete. Workers and planners cannot treat that response as successful
structured output, even if the partial text happens to be valid JSON. Shorten the
request or choose an appropriate model before explicitly retrying; no retry or
token-budget increase happens automatically. Normal `stop` and legacy responses
without a reason keep their existing completion behavior.

## Structured-request thinking

Planner and coding-worker requests now send `think: false` by default alongside
their JSON schema. This avoids a reproduced qwen3:14b/Ollama 0.34.0 failure where
thinking-only frames ended without answer text or a completion marker. The same
edge-case coding checks passed with thinking disabled. Ordinary chat requests
retain the model/server thinking default.

`--structured-thinking off|on|auto` explicitly sets this policy for schema-constrained
requests: off is the default, on requests thinking, and auto omits the option. Use
a mode supported by the selected model. The setting is invocation-local; pass it
again when restarting. It follows workspace switches within the process. No token
budget, deadline, admission limit or Ollama server configuration changes. No silent
retry or fallback occurs. This workaround is tested on synthetic coding cases;
complete role/model quality qualification remains open.

## Refine an unsaved task plan

After `/plan REQUEST` completes, use `/plan-revise REQUEST` to refine its tasks,
paths, checks and dependencies. The planner receives the previous task list,
original request and accumulated revision requests, plus freshly captured committed
repository context and project scope. It retains the draft's planner model even
if the conversation model has changed. Review the complete replacement before
`/plan-save`; saving proposes tasks and never approves or runs them.

A failed or malformed revision restores the previous complete draft with its
original task revision, so a failed refinement cannot refresh a stale save. Saving
and another revision are blocked while inference is active. `/plan-cancel` discards
the current draft and pending revision; late responses cannot restore them. Revision
requests share the 8 KiB prompt limit, and previous task reference data is bounded
to 64 KiB. Complete drafts now survive restart as described below; full Mission
Draft/Issue Graph formation remains unfinished.

## Plan draft restart continuity

Conversation snapshots v5 retain a bounded complete task-plan draft and its original
task revision. Autosave, normal shutdown and a quiescent workspace/mission switch
preserve it under the existing conversation owner lock and atomic save. A pending
refinement checkpoints its prior complete draft; partial first-generation output
is not a plan and is not restored. No inference resumes automatically.

Restored plans open for review, retaining their original repository/scope bindings
and stale-state checks. `/plan-save` still requires explicit submission and approval
remains separate. `/plan-cancel` clears the draft on the next checkpoint or normal
shutdown. A completed save followed by a crash before conversation checkpoint may
restore an older preview, but its old task revision prevents duplicate publication.
Saved plans are limited to 256 KiB and must pass the normal plan validation.

V1–v4 snapshots remain readable. First v5 save keeps the exact source bytes in a
versioned `.vN-backup`; conflicting backups, future versions, invalid plans and old
versions carrying plan data refuse without overwriting the original. Task v9 and
scope v2 remain unchanged. Full Mission Draft/Issue Graph formation is still open.

## Explicit Wayfinder capability

Type `@wayfinder REQUEST` to address the native scope adapter. Tab completes the
name; arrows select, Enter fills the composer, and Escape dismisses completion.
Completion never submits a turn. F1 lists this capability alongside commands.
Unknown leading `@` names and an empty Wayfinder request produce an error and retain
the draft without model dispatch. Mentions inside ordinary prose are not commands.

An explicit discussion request without a saved scope enters Chart (or Work-through
for an existing Wayfinder map/ticket). Ordinary read-only prompts outside the capability keep their existing
exclusion from automatic Chart. Four-field briefs and exact-revision confirmation
also accept the prefix; their outcomes retain the same scope receipts and grant no
task approval or execution. Subsequent discussion reuses the existing flow.
Other native roles remain accessible through their documented commands; this is
not a general skill/plugin executor or complete capability-routing implementation.

## Recovery before check launch

New worker runs retain a version-1 `execution-boundary.json` bound to the task,
run and baseline before preparing work. Before launching the approved check, the
worker exclusively creates and syncs `check-launch-intent.json` and its directory.
Failure to record this intent prevents check launch. The sandbox cannot write these
records outside its worktree. They are lifecycle records, not success or approval.

When the worker owner lock is free, `/recover ID` can reconstruct a Failed result
only if the start boundary matches, final evidence is absent and check-launch intent
is absent. It preserves partial work and states that no patch was reconstructed.
It does not invoke Git, inference or checks. Repeating recovery returns the existing
acknowledgment; `/repair` then proposes a new task with inherited permissions and
fresh approval. This does not prove retained-worktree quiescence or allow cleanup.

Missing/invalid/legacy boundaries, mismatched identities, any check-launch intent
(including an incomplete file), and existing malformed evidence remain unresolved.
Existing saved terminal evidence retains its previous recovery path. No task,
conversation or scope schema migration is required; old runs are not assigned new
proof by default. Recovery after a possibly launched check remains an open boundary.


## Explicit plan acceptance criteria

New generated plans require 1–16 distinct observable acceptance criteria per task,
each a nonempty single line of at most 1024 UTF-8 bytes. Review them alongside the
paths, check command and dependencies before `/plan-save`. Saving still creates
Proposed tasks; approval remains separate. A passing check does not automatically
establish every criterion or accept a task.

Task schema v10 retains the criteria in the immutable Plan receipt. The selected
task and evidence review show the recorded contract, and workers receive it as
reference within the approved policy. Repair descendants inherit the original
criteria and need fresh approval. Legacy/manual tasks without criteria explicitly
say not recorded; no criteria are inferred from a command or successful result.

Conversation schema v6 preserves criteria in unsaved plan drafts across restart
and quiescent mission handoff. Task v1–v9 and conversation v1–v5 remain readable;
the first newer write retains an exact version-named backup. Conflicting backups,
malformed criteria, future schemas and older schemas claiming new criteria refuse
without replacing the original. Task/conversation locations are unchanged.

This advances the Local Agent task-packet contract. Full Mission Draft/Issue Graph
formation and attributed action chronology
remain separate unfinished requirements.


## Criterion-level review

Use `/review ID JSON` to record an explicit accept/reject decision, its reason and
an evidence note for each recorded criterion, in order starting at 1:

```text
/review 4 {"accept":true,"reason":"Reviewed implementation and checks","criteria":[{"criterion":1,"met":true,"note":"Retained test asserts VALUE equals 42"}]}
```

Acceptance requires every recorded criterion to be marked met. A rejected review
may mark criteria not met. Notes are reviewer assertions supported by the inspected
evidence, not independently verified facts. The reason is a nonempty single line
of at most 2048 UTF-8 bytes; each note is a nonempty single line of at most 1024
bytes, with at most 16 ordered criteria. Unknown fields and mismatched coverage
refuse. Legacy/manual tasks without recorded criteria use an empty criteria list.

New `/accept` calls for tasks with criteria refuse and direct the user to `/review`.
`/accept` for tasks without criteria and `/reject` remain available. Historical
boolean review receipts remain readable and exactly replayable without invented
reasons or criterion assessments. Both review paths retain the existing successful
check, evidence-digest, expected-revision and exact-correlation guards. No inference
or repair runs inside the review transaction. Enabled dispatch may subsequently
start already-approved dependents when their parent becomes Accepted.

Task schema v11 adds the Assess receipt. Replay validates each assessment against
the already validated Plan receipt prefix and repair lineage. Before the first v11
mutation, a v1–v10 store receives an exact version-named backup; conflicting backups
or older schemas carrying Assess receipts refuse unchanged. Conversation v6 and
scope v2 are unchanged. Notes appear in task details, saved Activity and evidence
review; an open evidence view updates after acknowledgment. Rejected notes become
reference data for a separately proposed and approved repair, under the existing
128-KiB combined repair-context bound.

Explicit outcome support is documented below. Automated Frontier Reviewer
decisions and tiered automatic repair routing remain unfinished.

## Explicit review outcomes (task schema v12)

`/review ID JSON` also accepts `outcome` in place of the legacy `accept` field:
`approved`, `approved-with-limitations`, `needs-repair`, `needs-human-review`, or
`rejected`. The reason and ordered criterion evidence notes remain required.

```text
/review 4 {"outcome":"approved-with-limitations","reason":"Inspected implementation and checks","criteria":[{"criterion":1,"met":true,"note":"Retained check verifies VALUE equals 42"}],"limitations":["Performance outside this fixture remains unmeasured"]}
```

Both approving outcomes require all recorded criteria met, intact evidence and an
original successful worker completion. Limited approval additionally requires
1–8 distinct, nonempty single-line limitations, each at most 1024 UTF-8 bytes.
Other outcomes cannot carry limitations. Limitations cannot waive failed criteria.

Needs human review holds the task and its dependents. Direct approval, run and
repair cannot bypass the hold; an explicit new review decision must resolve it.
A failed run held for review still cannot be approved. Needs repair and Rejected
remain unaccepted; repair requires a separate proposal and fresh approval.
Saved Activity, task details and open evidence show the recorded outcome; resolving
a hold replaces stale review notes while preserving criteria and dependency inputs.

Task schema v12 adds the Decide receipt and human-review status. The first mutation
of a v1–v11 store preserves an exact version-named backup; conflicting backups and
older schemas carrying Decide refuse unchanged. Legacy Review and Assess receipts
remain readable and replayable. Conversation v6 and scope v2 are unchanged.
Automatic reviewer inference, tiered repair routing and architect escalation remain
unfinished; these decisions are explicit user actions.

## Review risk escalation (task schema v13)

A review may declare `risk` as `critical`, `security`, or `merge-risk`:

```text
/review 4 {"outcome":"rejected","risk":"security","reason":"Review found an unsafe input path","criteria":[{"criterion":1,"met":false,"note":"Inspected input handling needs correction"}]}
```

Recording any of these risks with Rejected, Needs repair or Needs human review
automatically holds the task for human review. The original outcome and risk remain
in the receipt and Activity; task status shows the hold. This blocks dependent
execution, direct approval/run, repair of the held task and sibling repair proposals
from its parent. Risk classification is a reviewer assertion, not automatic risk
detection. Absent classification means unrecorded, not verified safe.

Approved outcomes carrying a risk refuse. Resolve a held task with a subsequent
explicit `/review`, omitting `risk` and explaining the human decision in `reason`.
Approval still requires the original successful check and every recorded criterion
met. A failed run cannot become approved through risk escalation. Resolving to
Needs repair permits a separate repair proposal that still requires fresh approval.
No model request or repair starts inside the review transaction.

Task schema v13 adds the optional typed risk. Exact v1–v12 backups precede the first
mutation; older schemas carrying a classified risk refuse unchanged. Existing
unclassified reviews remain readable/replayable with no invented classification.
Conversation v6 and scope v2 are unchanged. Same/fresh-agent continuity and
Architect revision routing remain unfinished.

## Atomic review-to-repair proposals (task schema v14)

`/review ID JSON` now records an unclassified `needs-repair` or `rejected` outcome
and creates an inherited repair task in one transaction. The terminal names both
the reviewed parent and the new child, and links the child from parent readiness.
The child is Proposed: inspect it and explicitly `/approve CHILD` before `/run` or
enabled dispatch can start it. Recording the review does not launch a model/check.

```text
/review 4 {"outcome":"needs-repair","reason":"Correct the calculation edge case","criteria":[{"criterion":1,"met":false,"note":"Observed failing boundary input"}]}
```

The compound receipt stores both review intent and the child proposal; its primary
receipt task is the new child. Activity can find it under either parent or child.
Review notes and acceptance criteria follow repair lineage, and Local Agent
continuity counts compound Rejected decisions exactly like earlier rejections.
Risk-bearing reviews still enter a human hold without creating a repair.

Evidence, criterion coverage, open project scope, task/receipt/storage capacity and
unresolved-child guards apply to the whole operation. Failure leaves both review
and proposal uncommitted. Exact retries after restart return the same child;
conflicting/stale requests cannot add another child or overwrite the outcome.
A pending scope gate blocks the compound operation because it proposes new work.

Schema v14 adds `review-and-repair`; earlier schemas cannot contain that receipt.
Exact v1–v13 backups precede migration. Old Decide, Assess and boolean Review
receipts retain their original meaning; they do not retroactively create children.
Legacy `accept` JSON and `/reject` remain review-only, with `/repair` available for
explicit separate proposals. Conversation v6, scope v2 and agent transcript v1 are
unchanged.

Repair dependency resolution is implemented in schema v15 below. Review-triggered
execution authorization and full launch qualification remain open.


The Rust terminal /resolve-repair ID explicitly selects an accepted repair for its unsuccessful ancestor dependencies. It requires current revision, open scope, verified evidence and no unresolved family branch. The selection is immutable and exact request retry is idempotent. Future input records distinguish declared task from optional source_task.


## Architect revision after repeated failures

A reviewer can set `"failure":"architecture"` on a nonapproval `/review` decision.
The first classified failure proposes a normal repair. A second distinct reviewed
run in that lineage records an Architect route and stops ordinary repair work;
critical/security/merge risk takes precedence and remains a human hold. Absence
of classification stays unrecorded, and repeating a review cannot manufacture a
second failed run.

The current acknowledgment opens the real Frontier Architect when no draft or
inference is already active. `/architect-revise ID` explicitly resumes a pending
route after disconnect/restart or after another draft is cleared. Restoring state
never repeats inference. The Architect receives bounded verified lineage evidence,
review notes and criteria, plus repository context from the parent's exact commit.
It produces one revised repair task with explicit criteria and exact file/check
policy. This bounded repair revision does not rewrite unrelated mission tasks.

`/plan-save` explicitly adopts that draft as a linked Proposed repair, preserving
original dependencies and execution baseline/inputs. New paths/checks still require
fresh approval. The revised task starts a fresh Local Agent conversation and a new
architecture-failure cycle. Accepted revised work uses `/resolve-repair` normally.
Source task/run/evidence digest and route revision bind the draft; stale or tampered
source evidence refuses adoption. Draft refinement/restoration retains provenance.
Task schema16 and conversation schema7 introduce these fields with exact prior
version backups; earlier schemas cannot claim the new provenance.

The latest adopted Architect repair defines the active family branch. Older branches
cannot start workers, create repairs, change reviews or resolve the family. Cancelling
an adopted repair before it runs reopens its source Architect route; an explicit
new draft/adoption can replace the cancelled proposal without restoring old policy.
