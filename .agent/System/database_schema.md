# Persistence Schema

**Last Updated:** 2026-09-27

Alfredo has no relational database and no database migration layer. Authoritative configuration begins in local Markdown/JSON files, while runtime projections are versioned JSON documents stored below the configured app-local runtime root. Each Python authority store listed below uses atomic sibling-file replacement; append or read-modify-write stores additionally use `flock`, and expected-revision action families share a cross-process coordinator lock.

## Tables

The Rust terminal owns `rust-tasks-v1/<workspace-mission-sha256>/tasks.json`
below its app-local state root, outside the coding workspace. Schema v16 stores
workspace/mission identity, revision, bounded tasks and ordered command receipts.
Tasks retain id, title, model and earlier-task dependencies, with optional exact
file/check policy and run identity, Git baseline and evidence digest. Optional
`repair_of` names an earlier terminal parent and is proven by its Repair receipt. Statuses are
Proposed, Approved, Cancelled, Running, ReviewReady, Accepted, Failed, Rejected and NeedsHumanReview.
Policy changes reset approval; bare historical approval cannot start execution.

`TaskStore` uses an exclusive OS file lock and synced atomic replacement. Exact
request replay precedes stale-revision checking; changed correlation payloads and
stale new requests fail. Loading replays receipts to validate task projection.
The 4096-receipt / 4-MiB journal reserves one receipt and worst-case serialized
completion bytes for every Running task before admitting other mutations, including
new run claims. Finish consumes this reservation and remains subject to actual hard
bounds. Historical overcommitted stores can finish only when the actual result fits;
no history is deleted or authority inferred. Reservations are computed under the
same transaction lock and require no schema change.

Schemas v1/v2/v3/v4/v5/v6/v7/v8/v9/v10/v11/v12/v13/v14/v15 remain readable and receive a version-named exact-byte backup before
the first v16 mutation. Repair receipts require v3 or later; old tasks gain no
lineage, approval or permissions. Fresh repair proposals inherit policy/model and
require approval; worker context revalidates parent evidence. Future/malformed state is preserved. See the [migration SOP](../SOP/database_migrations.md).

Schema v8 optionally binds a Plan to its exact committed repository context:
Git baseline, bounded path map, selected source path/blob/content and omission counts.
The trusted context collector supplies this field, not the model's output schema.
Earlier plans retain absent context; versions 1–7 cannot claim context provenance.
Workers check a grounded plan's original committed baseline before preparing inputs
and claiming a run. Accepted dependencies still compose onto that baseline normally.

Schema v7 adds Assign receipts. Assignment changes only an unstarted Proposed or
Approved task's worker model, preserves its exact policy/dependencies/plan lineage,
and returns it to Proposed for fresh approval. Started or terminal tasks cannot be
reassigned. Replay proves the current model from proposal/plan plus subsequent
assignment receipts. Versions 1–6 cannot carry Assign receipts. The terminal adapter
queries installed models before a new assignment; exact saved requests replay
without inference/catalog dependence. Catalog membership is not profile qualification.

Schema v6 adds a Plan receipt with original prompt, planner model and 1–16 ordered
steps (title, worker model, exact policy and earlier-step dependencies). Replay
expands the entire batch into Proposed tasks and translates local dependency numbers
to durable task IDs. Validation and capacity checks precede one atomic save; no step
becomes approved or Running. Versions 1–5 cannot carry Plan receipts. Plan generation
is transient and non-authoritative; only explicit save invokes the task transaction.

Schema v5 adds Branch receipts binding an accepted task to its deterministic local
branch name and candidate commit. No new Task status is inferred. Git ref state
is verified by the handoff module; the receipt records the confirmed handoff, not
a promise that external Git actions never move the branch. Earlier schemas cannot
carry Branch receipts and gain no inferred handoffs. Before Git ref creation,
read-only branch admission validates request identity and prospective receipt/count/
byte capacity, including reserved worker completions. Exact recorded handoffs need
no new receipt. Admission does not reserve space across Git operations; concurrent
writes or later I/O failures still use exact-ref reconciliation.

Run claims in v4 or later optionally retain exact dependency `inputs` (task, parent run,
evidence SHA-256 and candidate commit). Receipt replay validates accepted parent
identity, and new transactions recheck saved candidate evidence. Earlier versions
cannot carry dependency inputs. Old runs receive no inferred inputs. The worker
verifies/composes Git ancestry before claiming; the store remains the receipt owner.

Each run retains a detached worktree, bounded model response and `evidence.json`
under `runs/<run-id>/`. Evidence binds baseline, outcome, diff and execution receipt;
its SHA-256 is verified for completion and review. Optional `candidate_commit`
binds a retained Git object for new successful runs. Missing legacy fields remain
absent; no task-store schema or approval migration occurs. Candidate verification
checks the sole parent and exact diff, with Git replacement objects disabled. A
`refs/alfredo/candidates/<commit>` ref retains the object independently of mutable
worktree files. Acceptance does not integrate
patches. Each newly launched task holds `worker-<id>.lock` from before claim until result
publication. Scoped claims and recovery probes explicitly unlock on orderly drop,
so an unrelated fork-inherited descriptor cannot prolong ownership. On process death,
the OS releases the lock after all inherited descriptors close. `/recover` can commit a
valid retained terminal result after acquiring the existing owner lock; it cannot
adopt a live or unmarked legacy run. Missing/invalid evidence preserves uncertainty.
No effects are replayed. Storage transactions wait at most 250 ms for brief lock
contention; retained ownership checks remain nonblocking. No retirement or aggregate
disk budget exists yet.
These stores are independent of the legacy Python schemas below. See
[Rust terminal commands](../../alfredo-tui/README.md).

The Rust terminal also stores schema-v2 conversation sets as
`conversations-<name-sha256>.json` in the same workspace/mission namespace. A separate
OS ownership lock permits one writer per named set. Snapshots include sessions,
selected conversation, model, draft/cursor, message pairs, status and attempt identity.
Version, namespace, role ordering, lifecycle, cursor and size bounds validate before
restore. Active requests become explicitly interrupted on reload without replay.
Autosave is asynchronous and coalesces to current state; final save joins the prior
write before synced atomic replacement. Malformed/future state and symlink targets
are preserved/rejected. No Python conversation importer exists. See the
[terminal README](../../alfredo-tui/README.md) for limits and checkpoint-loss window.

There are no SQL tables. The table-like JSON stores are:

| Store | Owner | Key fields | Purpose |
|---|---|---|---|
| `workspace-sessions.json` | `WorkspaceJourneyStore` | Starting Location, Coding Workspace, revision, Active Mission, known Missions, mission catalog, selection and choice receipts | Canonical pre-session workspace/Mission journey and exact restart restoration |
| `runtime.json` | `AlbertMission` | mission, issues, sessions, reviews, delegations, command policy, `workstation_actions`, `archived_issue_ids`, `supervision`, `retirement_storage`, `inference_turns`, per-session Preservation Budget and Retirement Unit state/action receipts, persisted evidence correlation ids, timeline | Canonical mission runtime, Local Agent lifecycle, deterministic runner-supervision, bounded local-inference receipt authority, preservation/storage authority, retained completed-issue archive identity, evidence identity, and mutation-coincident recovery markers |
| `workspace-preferences.json` | `WorkspaceSnapshotService` | revision, active mission, conversation scope, operations view, events, `workstation_receipts` | Canonical desktop projection preferences, ordered updates, and idempotent Workstation action acknowledgements |
| `agent-console-history.json` | `AgentConsoleHistoryService` | message id, sequence, role, content, scope, outcome, source, optional correlation id/action phase, controller `action_outcome`/`action_message` | Durable unified controller/workstation chronology, exact controller non-action truth, and idempotent Workstation audit phases |
| `wayfinder-state.json` | `WayfinderService` and `AlbertMission` launch boundary | schema version, one active flow, mode, originating prompt message id, gate status/opening receipt | Project-level canonical Wayfinder entry and Shared Understanding Gate state, stored below `runtime_root/wayfinder/<repository-hash>/` independently of Mission runtime; no controller memory is authority |
| `workspace-queue.json` | `WorkspaceQueueService` | revision, queue items with proposal/decision correlation ids, `receipts` | Pending proposals, confirmations, Ad Hoc Delegations, exact projected receipt identity, and idempotent proposal/decision acknowledgements |
| `mission-drafts.json` | `MissionDraftService` | revision, drafts, ordered lifecycle receipts | Proposed mission composition, exact replay boundaries, and confirmation recovery before accepted state |
| `working-context-curation.json` | `WorkingContextService` | revision, dispositions | Eligible source pin/exclude choices |
| `activity-journal.json` | `ActivityJournalService` | revision, contiguous entries | Attributed meaningful actions and evidence links |
| `shell-terminal.json` | `ShellTerminalService` | revision, commands, execution owner identity, grants, grant denials | Metadata-only governed commands, crash-safe execution state, expiring path authority, and exact grant/denial decisions |
| `path-grant-requests.json` | `ShellTerminalService` | request id, correlation id, Mission, canonical path, access, duration, reason, affected action, requested time | Append-only typed contextual authority requests that survive restart independently of mutable terminal decision state |
| `execution-receipts.json` | `ExecutionJournal` | schema version, exact redacted request, request digest, typed receipt, process/owner identity, output byte counts/digests | Shared Local Agent/Shell host-effect intent and receipt ledger; prevents exact replay from launching a duplicate effect and records uncertain crash cuts for reconciliation |
| `inference/qualification/reports/*.json` | `QualificationReportStore` | schema version, exact Profile/runtime pin, governed fixture digests, bounded observations/metrics, promotion blockers | Bounded qualification evidence; prompts, streams, plans, Evidence Packages, authority decisions, and source-dependent outcomes are excluded |
| `shadow/rust-eligibility.json` | `RustEligibilityStore` | schema version, revision, gate evidence, disabled reason | Fail-closed Rust shadow eligibility; never grants canonical writer authority |
| `inference/qualification/promotion-state.json` | `QualificationReportStore` | revision, active/previous exact report and runtime pin, correlation history | Replay-safe Profile promotion and rollback metadata under a cross-process lock; does not grant Mission authority |

Bulky raw command output, worker prompts/responses, test logs, and `review.diff` files live under per-session artifact directories and are registered on the owning session rather than embedded into the JSON stores or Markdown tracker. Review projections replace eligible host paths with opaque app-local references. The bounded Session Artifact reader resolves only an exact Mission/session/reference tuple and returns text without creating an artifact-content JSON store.

The execution ledger is per Mission runtime and uses schema version 1. Each record stores the request without input bytes plus a typed receipt without stdout/stderr; input/output SHA-256 digests and byte counts preserve evidence without promoting raw host data into canonical state, and reload rejects any record that reintroduces raw fields. A request is structurally validated, then claimed before provider invocation under an inter-process lock; deterministic pre-effect authorization/boundary failures are durably recorded as `start-failed`, while the claim/receipt replacement also syncs its containing directory. Exact terminal replay returns its stored receipt; a changed request identity is rejected; a dead `executing` owner becomes `outcome-unknown` and requires reconciliation. `LocalAgentSession.execution_receipts` stores a bounded redacted terminal projection only after matching the durable request, authority, current runner operation, and Worktree Identity, while Shell command records store the receipt identity/status and can be repaired from the submitting Mission's ledger after a projection crash, with a read-only, idempotent legacy app-level ledger fallback. Missing ledgers decode as empty, while malformed requests, receipts, identities, digests, raw fields, or schema fail closed. Automatic same-session runner recovery is blocked when the Local Agent ledger still contains an executing or uncertain effect. On Darwin, process-group cleanup requires a matching leader start identity and observable quiescence; uncertainty fails closed rather than being treated as absence.

## Shadow Rust Eligibility

Rust shadow samples do not write canonical Mission or execution stores. The app-local eligibility record retains only bounded sample/cohort identities, gate booleans, stage names, failure codes, a revisioned decision, and optional exact release evidence. Release evidence stores canonical provider/manifest paths plus their SHA-256 values; loading it reopens the complete production manifest, recomputes both archive SHA-256 and npm SHA-512 values, validates platform/meta package identities, exact dependency/aliases, and complete AppImage/provider desktop metadata, then rechecks the external provider and manifest. Missing artifacts, synthetic manifest shapes, package-identity drift, path changes, byte changes, or hash mismatches disable Rust.

Finalized Agent Console user/controller turns remain in the durable full history store so conversation continuity survives restart. Only the Working Context extraction used for one model turn is windowed and content-bounded.

## Non-Authoritative Continuity

- The launcher keeps best-effort `recent-workspaces.json` and `launch-context.json` below the runtime root. A Starting Location is never added to recent workspaces merely because Alfredo was invoked there, and selecting a recent entry is an explicit relaunch only. Authority for restart continuity comes from `workspace-sessions.json`, not either launcher convenience file.
- React uses workspace-scoped browser keys for selected controller, local card/detail continuity, and at most 100 terminal negative Workstation action turns with per-turn content bounds. Corrupt, pending-only, or accepted-only local records cannot create canonical state.
- `MissionSessionSummary.last_activity_at` is derived from validated session timestamps (terminal end/cancel/start fallback) rather than stored as a separate mutable authority record. `runner_started_at` is projected separately from its exact durable session field for R6 evidence. Malformed timestamps fail projection validation; absence renders as `Not recorded`.
- Production measurement JSON Lines and correctness-gate files are append-only, non-authoritative evidence outside the runtime stores above. They bind fixture bytes, clean source archive, packaged artifact, variant, cohort, correlation, and monotonic stage marks but cannot mutate or replace Mission truth.

## Relationships

- `project_key + mission_id` isolates each `runtime.json` namespace.
- Workspace preferences reference one active Mission from the in-memory mission catalog; Mission summaries reference their own sessions and queue attention.
- Every Local Agent session retains its Issue Slice or `ADHOC-*` id, assigned agent, task packet, worktree, evidence, artifacts, runner ownership/process-group identities, runner operation id, Worktree Identity, monotonic session revision, automatic-recovery count, supervision receipt id, any durable typed runner-result candidate, a schema-versioned Preservation Budget, and schema-versioned Retirement Unit state.
- The Preservation Budget records `reserved | verified | discarded`, its fixed reserved byte capacity, binding state, and reservation/verification/discard time. Missing legacy fields decode as one bound 32 MiB reservation. Retirement Unit state records `active | preserving | preserved | grace | retiring | retired | preservation-blocked | retirement-blocked`, preservation/retry/export/discard intents, Mission/session-bound action receipts, last exact runner/process boundary (including synthetic exact no-runner proof for never-started cancellation), grace start/expiry, bounded retirement-attempt count, `git-worktree | git-registration | managed-directory | managed-absence | retained-worktree-discard` removal kind, retired/discard time, failure reason, and compact snapshot record. A direct-discard intent additionally binds the root device/inode, one canonical bounded schema-v2 tree manifest, full and materialized digests, proven Git-pointer projection, removal strategy, and any exact stale-registration cleanup obligation; replay permits only a remaining exact manifest subset whose modes are either original or the deterministic removal-preparation mode. An export intent binds the normalized destination lock, exact parent/runtime/source identities, manifest digest, claimed time, one of two deterministic stage names, and `reserved | bound` anchor/payload identities. Exact six-field legacy export intents remain readable and migrate only from safe absent or exactly complete states. Malformed state fails closed. A preserved or later non-discarded unit must name a snapshot; verified preservation or explicit discard releases the bound budget.
- Retirement Snapshot payloads live below `runtime/<project-key>/retirement/payloads/<session-id>/`, outside the Coding Workspace. `manifest.json` binds canonical authority and identity to registered Git state, an exact app-managed directory tree, or a verified deterministic `managed-absence`, plus evidence registrations, payload/manifest/total snapshot byte counts, and hashes; the total including `manifest.json` must fit the bound budget. The compact session record adds Mission/session/outcome identity, creation/expiry, pin state, retained/reclaimed disposition, and reclamation receipt without changing the immutable manifest digest. Git payloads hold a self-contained baseline bundle, staged/unstaged patches, typed non-ignored and ignored entries, registered evidence, and exact status bytes. Directory payloads hold exact regular-file bytes and permission modes. Absence payloads retain app-local runtime evidence without creating a worktree. Clean-room reconstruction validates every boundary. Temporary capture, reconstruction, export verification, and repair-materialization directories remain app-local; one per-session retirement effect lock serializes pin, retry, export, discard, preservation, and removal; deterministic `retirement/removal-effects/<session-hash>.worktree` paths make pre-cleanup Git/directory isolation restart-safe and keep late original-path writes outside the deletion target. Same-user descriptors, process cwd/root boundaries, and writable shared mappings are inspected before deletion; split Git move back-pointers and partial marker loss are repaired only from exact administration state; a late publish-boundary race quarantines the unpublished payload and blocks the unit.
- Mission `retirement_storage` schema version 1 records aggregate reclamation totals, the bounded recent reclamation tail, per-session pre-effect reclamation intents, and current storage attention. Each current reclamation intent binds the exact manifest/path/size request plus the descriptor-proven payload-root `root_device` and `root_inode`; a pre-upgrade intent without those fields is accepted, safely upgraded from a still-verified root or exact absence before deletion, and then replayed through the same identity guard. Admission counts retained payload bytes plus every bound Preservation Budget, reserves another 32 MiB before session creation inside the shared launch lock, and reclaims only expired unpinned `retired` payloads by `(created_at, session_id)`. Startup performs the same eligible sweep. Explicit unpin only changes eligibility and may clear stale protected-exhaustion attention; it does not reclaim bytes, so exact action replay has no delayed global deletion effect. Storage and status inspection remain read-only. Deletion is descriptor-relative and limited to the exact deterministic payload root and entries whose identities and content still match; a swapped root or entry fails closed and remains untouched. The record changes to `reclaimed` only after canonical absence is proven. Legacy verified snapshots without storage-policy fields remain readable and acquire conservative creation/expiry metadata when first inspected for policy or mutated.
- Mission `supervision` schema version 1 contains observer incarnation/cursor/sequence-receipt maps, Local Agent Attention Records, pending/applied/blocked recovery intents, and semantic incident receipts. Actionable delivery commits attention, intent, receipt, and cursor in one runtime replacement before applying the lifecycle effect. Missing legacy supervision state decodes as an empty version-1 ledger; malformed present state fails closed.
- Mission `inference_turns` is a bounded ordered ledger of at most 128 schema-version-1 Local Inference receipts. Each receipt binds Mission/session/request/turn identity, exact Profile and resolved model digest, admission/headroom, outcome/authority, requested processor policy plus observed GPU/total bytes, load/prompt-evaluation/first-token/decoding timings, usage, and an optional active Lease snapshot. Receipt JSON is capped at 512 KiB, contains no prompt or raw stream, and must be complete/canonical on write and reload; only `completed` with matching `/api/ps` runtime evidence may be authoritative. Missing legacy state decodes as an empty ledger, while malformed shape, identity, timestamps, profile, admission, timing, usage, lease, or authority state fails closed. The separate `runtime_root/inference/lease-state.json` ledger stores one active entry, at most 64 queued entries, resident model identity, at most 128 audit records, and a monotonic sequence under an inter-process lock; corrupt entries fail with `ledger-invalid`.
- Local Inference qualification reports live below `runtime_root/inference/qualification/reports/` under a digest-derived filename. Each schema-version-1 report binds a report id, exact canonical Profile, runtime/binary/configuration pin, governed fixture ids, repetition count, bounded observations, quality/reliability and outcome counters, decomposed timing/reviewed-latency p50/p95 summaries, context/prefix reuse measurements, promotion blockers, and rollback-tested state. Observations contain flags and identities only: prompts, raw streams, plans, Evidence Packages, authority decisions, and source-dependent outcomes are rejected on reload. `promotion-state.json` retains one active exact report/pin, one previous record for rollback, a bounded action history, and the last action under `promotion-state.lock`; promotion requires a promotion-ready non-withdrawn pin and exact report match, while exact replay returns the existing state.
- Agent Console messages retain the exact Mission-qualified Conversation Scope captured for their originating prompt.
- A project has at most one active Wayfinder flow across all of its Missions. Its `chart | work-through` mode, originating exact prompt identity, and `pending | open` Shared Understanding Gate survive restart in `wayfinder-state.json`; only an explicit Mission Commander confirmation or a persisted `wayfinder-agent` acknowledgement can open that gate.
- While an active Wayfinder gate is pending, canonical Mission Draft mutation, all delegation paths (including legacy route/approval), and all production launch paths (Workstation, CLI, TUI, and headless worker execution) reject before mutation. Conversation, read-only inspection, bounded research, Grilling, and throwaway prototypes continue through their normal command/controller boundaries.
- Frontier Model messages carry exactly one typed `no-action` or `awaiting-orchestrator` outcome and its fixed canonical display message. The append and read boundaries reject mismatched copy, and `model-commentary` cannot carry an Orchestrator receipt correlation/action phase.
- Workstation request/acknowledgement Console turns retain a unique `(correlation_id, action_phase)` marker so replay can restore either missing phase without duplicating the other; ordinary chat retains empty backward-compatible marker fields.
- A syntactically valid Mission Commander Workstation action that reaches Python but is rejected still appends durable `request` then `rejection` Console phases. A transport failure that never reached Python has no backend audit claim and is eligible only for bounded browser continuity.
- Workspace Queue items retain Mission ids, originating message/item ids, and exact proposal/decision correlation ids. Legacy items with missing projected ids derive them only from one uniquely matching validated receipt; an ambiguous chain fails persistence, while an absent chain remains unreceipted and React suppresses effect claims. Mission Draft entries retain their Mission and originating identities separately.
- Queue inspection retains canonical resolved history for replay/audit, but the workstation Queue projection is intentionally pending-only. Hiding resolved items or standing creation forms is a React projection rule and never deletes `workspace-queue.json` or `mission-drafts.json` state. Issue Assignment similarly filters canonical snapshot rows by parsed `work_type`/`tracker_status` without mutating tracker/runtime stores. See the [2026-07-12 acceptance correction](../Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md).
- Mission Draft lifecycle receipts retain the exact request, prior draft state, effect draft, acknowledgement, and ordered revision. Current receipts must derive one canonical draft chain; exact replay rejects changed reasons/effects, coherent receipt substitution, missing predecessors, and downgrade from the current receipt contract. Confirmation recovery uses the receipt's immutable accepted-Issue identity while allowing later separately governed Issue fields to evolve.
- Workstation session task packets and the Mission-level `workstation_actions` ledger bind a correlation id to the normalized request in the same runtime write as the authoritative mutation. `workspace-preferences.json` then records the acknowledgement receipt. Exact retry uses either layer to finish/replay acknowledgement and reconcile audit side effects; reusing the correlation id with a different boundary is rejected. Workspace Queue stores its request and acknowledgement together in its own atomic receipt.
- `archived_issue_ids` is a validated unique subset of the Mission's known Issue Slice ids. Only the private archive/restore Workstation transaction changes it, under the runtime lock and with a required correlated marker that exactly binds its action, Mission Commander actor, Mission, expected revision, Issue Slice target, and normalized request. Every Mission in the catalog is bound to the primary Mission's single canonical `workspace-preferences.json`; the mutation seam independently reads and compares that shared revision and rejects missing, incomplete, stale, malformed, or mismatched authority before changing archive state. Ordinary stale runtime persistence preserves the latest canonical archive set so it cannot revive a restored Issue Slice. Archiving never removes the Issue Slice, its Local Agent sessions, Evidence Packages, Activity Journal entries, or inspection identity.
- The repair task-packet preview is derived from the canonical prior session and review decision at snapshot time. It repeats only the inherited goal, acceptance criteria, allowed paths, command policy, evidence requirements, assigned actor, and review reason; it does not become a second writable task packet or expose host paths/artifacts.
- Dependency satisfaction is lifecycle-only: `pr-ready` and `complete` satisfy a blocker. An approved or launch-authorized follow-up is not a reviewed outcome and cannot unblock its dependent Issue Slice.
- Session summaries expose a launch correlation only when the task-packet marker matches one exact Queue or Workstation request/acknowledgement receipt and its Mission, Issue, item, action, and session boundaries. Review correlations additionally require the exact persisted Review request, one matching workspace event, and one matching Activity Journal decision. Unvalidated nested runtime strings project as empty rather than UI authority.
- Valid Evidence Packages persist `evidence:<mission-id>:<session-id>` on the owning session in the same runtime write. Exact replay of the same package idempotently reconciles its matching Activity Journal phase, repairing a runtime-first/journal-write interruption without duplicating a completed phase; trying to reuse that identity for changed evidence fails closed. Legacy or tampered evidence without the exact Mission-qualified stored identity remains reviewable in its older data path but does not produce a receipt-styled evidence chronology claim.
- Activity entries link Mission, Issue Slice/Ad Hoc Delegation, session, queue decision, command, and evidence identities without becoming canonical lifecycle authority.
- Shell command records retain the submitting Mission, exact request correlation, approval/denial boundary, and durable `executing | outcome-unknown | completed | failed` state so approval/completion remains attributed correctly after an Active Mission switch. An execution marker is persisted before process start; a dead owner is durably converted to `outcome-unknown` and never automatically re-executed. Missing request/decision/final audit phases reconcile before unrelated Console or Journal entries advance.
- Each contextual path request has a unique request id and binds one canonical non-symlink path, correlation, Mission, access level, fixed duration, reason, affected action, and validated timestamp. Grant and denial records reference that request id; creation accepts only the exact pending boundary, while replay, changed authority, malformed timestamps, and duplicate ids fail closed. Projection derives `pending | granted | denied` from the append-only request plus terminal decision records.
- Session artifact registrations retain backend paths only inside authoritative runtime/session state. UI projections receive opaque references and bounded content, never a host path field.

## Indexes

No database indexes exist. Stores use stable ids and in-memory dictionaries/lists. Runtime/session lookup is dictionary-based; histories and journals preserve sequence order. The bounded local scale does not currently justify an embedded database.

## Migration History

- 2026-08-15 added versioned Local Inference Profile/Lease contracts, bounded raw Ollama HTTP admission, `/api/ps` runtime digest and processor evidence, persisted receipt and lease ledgers, exact-session/workstation projections, and typed Mission Work telemetry. Existing runtime documents decode missing `inference_turns` as empty; no model output is promoted without complete bounded schema, timing/usage, and running-model validation.

- Legacy unstarted `launched` sessions migrate to executable `queued` state on load.
- Missing optional fields use backward-compatible defaults when records are decoded.
- JSON schemas remain versioned at projection boundaries; malformed or non-contiguous state fails with structured persistence/contract errors instead of being silently repaired.
- 2026-07-11 added Mission-qualified action/message identities, runner process identity/recovery metadata, Shell submitting Mission plus executing/outcome-unknown recovery, unique atomic temporary files, lock-safe transactions, default-empty Workstation/Queue/Mission-Draft receipt collections, Mission-runtime Workstation recovery markers, correlated causal audit phases, and opaque review-artifact projections.
- 2026-07-12 added typed append-only `path-grant-requests.json`, exact request-linked grant/denial replay validation, and timestamp-backed session activity. React also keeps a bounded workspace-scoped browser record for transport-failed action display; that local UI continuity is explicitly non-authoritative and is not part of the Orchestrator schema.
- 2026-07-12 corrected Markdown metadata parsing across an H1/blank-line header and added `work_type` to Issue Slice summaries. No JSON store migration was required; the fields originate from tracker Markdown and only refine desktop projection.
- 2026-08-03 added exact controller action-outcome messages, Queue proposal/decision projection identities with unique-receipt read backfill, persisted Mission-qualified evidence ids, and receipt-validated session launch/review projections. Existing version-1 records remain readable, but any authority that cannot be proven from an exact canonical chain projects without an effect claim.
- 2026-08-03 added `wayfinder-state.json` schema version 1. Missing state means no active Wayfinder flow; one shared typed loader rejects malformed state as a structured persistence error rather than falling back to controller memory or allowing a direct production launch.
- 2026-08-06 added optional `archived_issue_ids` to existing Mission runtime documents. Missing state decodes as an empty archive; malformed, duplicated, or unknown ids fail closed. No Issue Slice/session/evidence migration is needed because archival groups existing canonical records rather than moving or deleting them.
- 2026-08-09 replaced fixed-count abandoned-owner requeue with schema-versioned deterministic runner supervision. Existing sessions decode new boundary fields with safe empty/zero defaults; legacy running sessions without exact process/worktree proof produce a Mission Commander decision rather than an automatic rerun.
- 2026-08-13 added the schema-versioned `ExecutionRequest`/`ExecutionReceipt` boundary and per-Mission `execution-receipts.json` ledger. Existing Local Agent and Shell authorization/canonical projections remain authoritative; raw input/output stays transient and rejected on journal/runtime reload, exact terminal replay is provider-free, protected writable roots and temporary-path symlink escapes are rejected, undeclared Bubblewrap host-read mounts are rejected even for injected providers, and dead in-flight effects become reconciliation-required `outcome-unknown` rather than being retried. Pre-effect sandbox, command, authorization, and provider-start failures are typed `start-failed`; child PID/start identity is bound before capture setup; automatic runner recovery waits for reconciliation when a Local Agent receipt is uncertain or its session projection lacks a matching ledger record; terminal chronology uses one lock order and atomic JSON markers are directory-durable; public Shell start-failure replay remains `126` while the typed receipt invariant remains `127`. Legacy Shell fallback is read-only and idempotent, and Darwin process-group cleanup fails closed if matching identity/quiescence cannot be proven. Non-POSIX host effects fail closed.
- 2026-08-09 added schema-versioned per-session Preservation Budget and Retirement Unit state plus app-local Retirement Snapshot manifests/payloads. Missing legacy fields acquire a conservative bound reservation and active phase; malformed budgets, phases, runner boundaries, or preserved-without-snapshot state fail closed.
- 2026-08-09 extended Retirement Unit state with grace, retiring/retired, and retirement-blocked phases, durable preservation intent, three-attempt retirement accounting, verified removal receipts, managed-directory snapshots, and restart reconciliation. The additive schema remains version 1 so existing active/preserved records decode conservatively without migration.
- 2026-08-09 added Snapshot Payload retention metadata, pin/disposition fields, aggregate `retirement_storage`, reclamation intents/attention, strictly action-shaped correlated blocked-action receipts, retry/export/discard intents, and the `discarded` Preservation Budget outcome. Malformed action receipts fail closed during runtime load. Existing verified snapshots without storage fields remain readable and are conservatively upgraded from their preserved timestamps or immutable manifest metadata.

- 2026-08-15 added the app-local Rust shadow eligibility record and production-equivalent cohort evidence. Missing or failed parity, store-integrity, crash-cut, state-version, packaging, release-gate, or stage evidence keeps Rust disabled.
- 2026-08-30 cut qualified Local Agent effects over without adding or destructively converting a canonical store. Existing `ExecutionReceipt.provider` identity now remains stable across executing, terminal, cancellation, and uncertainty records; providerless version-1 receipts continue to decode as `python` and are not rewritten on read. The effect child's PID/start identity—not the Rust adapter PID—is persisted before completion. Exact completed or uncertain replay remains provider-free, and unresolved Rust receipts continue to block automatic Local Agent recovery for Mission Commander reconciliation.
If a future workload outgrows bounded JSON stores, migration must preserve Orchestrator authority, expected-revision semantics, append order, Mission isolation, and artifact separation before replacing this design.

### Native terminal advisory model metrics

Worker evidence optionally retains `model_metrics` with unsigned nanosecond
`total_duration`, `load_duration`, `prompt_eval_duration`, `eval_duration` and token
`prompt_eval_count`, `eval_count`. The provider accepts bounded fields only from
explicit final frames. Evidence digests cover this metadata, but task success still
requires the existing check/candidate contract. Older evidence omitting the field
loads as absent. Task schema remains v8; conversation schema is v2 and skips
transient metrics on save/restart. Plan timing is not stored in Plan receipts.

Conversation v2 adds optional `task_view` (`visible`, `selected`, `query`). Query
is at most 200 bytes without controls; a selected ID must be positive. These are
presentation preferences resolved against the current task snapshot, never authority.
Readers accept v1 without the field and v2, reject v1 carrying view metadata, and
reject future versions. The first v2 save preserves exact v1 bytes in a
`conversations-<digest>.v1-backup` sibling under the conversation owner lock.
Conflicting backups prevent replacement. Evidence/plan/activity overlays, scrolling,
model timing and dispatch intent are not restored.

### Advisory native mission identity

Each opened workspace/mission namespace may contain `identity.json` with version 1,
canonical `workspace` and `mission`. The namespace SHA-256 must match the existing
serialized workspace/mission tuple. Records are at most 8 KiB, mission names 120
bytes and workspace paths 4,096 bytes; controls and unsupported versions are refused.
Publication writes/syncs a temporary file, atomically hard-links the complete record
without replacement and syncs the directory. Identical concurrent writes converge;
conflicting/corrupt records are preserved and startup reports a discovery warning.
This file grants no task or workspace-session authority and changes neither task v8
nor conversation v2 schemas.

Discovery reads at most 1,024 entries/16 MiB, skips symlinks and invalid identities,
and matches only the acknowledged workspace. Without a sidecar, a bounded v1–v8
task journal can supply a name hint; receipt validity remains normal loading's job.
No discovery read creates locks or state. Conversation-only legacy namespaces have
no recoverable mission label until manually opened once and registered.

### Native mission identity admission

`mission.json` is a separate version-1 canonical identity record containing workspace
and mission, bounded to 8 KiB after JSON escaping. It is distinct from advisory
`identity.json`. Start New checks absence of mission/task/conversation data under the
namespace transaction lock, syncs a temporary file and atomically links it without
replacement. Concurrent creation has one winner. An uncertain acknowledgment instructs
explicit Resume of the same name; no identity duplicate or automatic action replay.
Resume validates the existing identity, or valid legacy task/conversation state;
an advisory cache alone is insufficient. Legacy resume rewrites no source data.
Unsupported/corrupt identities refuse unchanged. No task v8/conversation v2 migration
or inferred planning, scope or execution approval is introduced. Discovery prefers
mission.json over advisory records and legacy task hints.

### Native Shared Understanding journal v1

`rust-understanding-v1/<canonical-workspace-sha256>/understanding.json` stores schema,
workspace, revision, draft revision, the four-field brief, confirmed state and ordered
request/actor receipts. Fields: destination, scope, constraints, uncertainty (required,
2 KiB each). Mission Commander Draft resets confirmation; Confirm must name the exact
pending draft. Every reload replays bounded receipts to prove the projection; versions,
identities, actors, correlations and expected revisions validate before admission.

Scope state is shared across missions under one configured runtime. Lock order is
understanding then task namespace. New guarded task transactions and owner claims
check the current gate; exact task replays, cancellation, Finish, Review and accepted
Branch handoff do not require fresh scope confirmation. Scope confirmation never
invokes another action. A synced atomic journal replacement retains at most 256
receipts / 1 MiB; draft admission reserves one worst-case confirmation receipt and
its encoded bytes. Old native workspaces without a journal are outside this explicit
flow. Task v8 and conversation v2 schemas remain unchanged. Desktop Wayfinder state
is separate and has no inferred migration into this journal.

## Scope-bound native plans

The planner reads the workspace scope journal off the UI thread and supplies its
brief and revision to the model as reference data. The model cannot set provenance:
the adapter retains the captured binding alongside committed repository context in
the preview and saved Plan receipt. Activity shows the captured scope revision.
Pending-scope previews remain discussion only. Confirming or replacing their scope
requires generating a fresh plan before publication.

Task schema v9 adds optional Plan.scope (workspace, revision, draft revision, brief,
confirmed). Publication and planned worker owner/Start admission compare the complete
binding with current scope under the scope lock. A new agreement revision invalidates
an older unstarted plan even when its text is identical. Exact acknowledged requests
still replay without new effects. Existing runs can finish and be reviewed. Older
plans lack binding; they remain readable and usable outside the explicit scope flow,
but need replacement plans after that flow starts. No agreement is inferred during
migration; v1–v8 data gets an exact version-named backup before the first v9 mutation.
Manual tasks and repair proposals still have their own explicit policy/approval flow;
this binding does not yet provide scope provenance for every task-formation path.

## Native understanding journal v2

The existing `rust-understanding-v1` namespace now accepts schema 1 or 2. V2 adds an
optional flow (Chart/Work-through and originating prompt) and an Enter action with
fixed `wayfinder-alfredo` actor. Only entry can create a flow, at revision zero; later
Draft/Confirm receipts remain Mission Commander actions and retain that flow. Receipt
replay verifies mode, prompt, brief, confirmation and revision. Entry cannot confirm
its placeholder brief; a separate four-field Draft must precede Confirm.

The first mutation of v1 preserves the exact original bytes in
`understanding-v1-backup.json`; conflicting or non-regular backups refuse unchanged.
Read-only loads and exact replay do not migrate. Missing state implies no active flow;
no desktop Wayfinder state or agent acknowledgment is inferred. Existing v1 native
readers reject v2 instead of bypassing the gate. Receipt/byte bounds and scope-before-
task lock ordering remain. Task v9 and conversation v2 are unchanged; Plan scope
bindings still use the complete current brief/revision and require fresh generation
after any new agreement revision. Desktop/native migration remains a separate workflow.

## Native response attribution and conversation v3

Conversation schema v3 stores a bounded `sources` map on each Session, keyed by the
assistant-message index. A source is either the requested model name or the Wayfinder
adapter, optionally with the original scope receipt correlation/revision. Provider
messages remain role/content only; source metadata never enters Ollama's wire payload.
Only the application adapter sets Wayfinder attribution. Model prose cannot set it,
including text that imitates a Wayfinder acknowledgment. Attempt/status guards reject
late attribution, and retry removes only the interrupted response's source.

The transcript uses these structured labels. Missing legacy metadata displays
`Assistant · source unrecorded`; no source or receipt is inferred from text. Source
references describe historical response provenance and grant no scope, task, review
or execution authority. The current scope/task journals remain authoritative.
Source metadata is validated for assistant indices, bounded names/correlations, known
kinds and receipt revisions; it is not a cryptographic proof of authorship.

V1 and v2 load without new attribution. Before upgrading, the store preserves the
exact old file in the corresponding `.v1-backup` or `.v2-backup`; conflicting backups
and schema downgrades refuse unchanged. Old schemas cannot carry source metadata.
The 12-MiB conversation limit and per-session/message bounds remain. Scope v2 and
task v9 schemas are unchanged. Conversation v3 readers are required after migration.

## Native saved reading position — conversation v4

V4 adds optional Session `reading`: an `anchor` containing logical transcript
`line`/wrapped `row`, and `last_offset` matching the saved numeric `scroll`. A
non-following viewport persists this presentation state; follow-latest omits it.
Pending key navigation remains transient and is removed from checkpoints. Bounds,
version and offset consistency are validated before loading or saving. Restoring
uses the logical anchor despite hidden stream growth or different terminal height;
width changes clamp its wrapped row rather than preserving an exact character.

Versions 1–3 remain readable with numeric-offset fallback and no invented anchor.
The first v4 write keeps an exact `.vN-backup` for the source version. Conflicting
backups, invalid anchors and downgrades refuse unchanged. The transcript's logical
line structure is part of this presentation format; changes to that structure must
consider anchor migration. No timing, pending navigation, inference or task effect
is restored. Response sources remain as in v3; scope v2 and task v9 are unchanged.

## Requested generation settings in worker evidence

New worker evidence records `generation` when preparing a schema-constrained model
request: requested thinking mode (`auto`, `on` or `off`), `num_predict` and temperature.
The verified evidence view shows these values. The record describes configuration;
it does not prove HTTP dispatch, server compliance or model qualification. Failures
before model-request preparation can have no generation record.

The field is optional and included in the existing evidence-byte digest. Legacy
evidence stays unchanged and displays “Requested generation: unrecorded”; no default
is inferred for past runs. Unknown thinking modes, unknown metadata fields and
out-of-bound numeric values reject deserialization. `auto` is an explicit recorded
choice, distinct from absent legacy data. This additive evidence field does not
change task receipts, scopes, permissions or snapshot schema versions.

## Plan draft restart continuity

Conversation schema v5 introduced retention of a complete task-plan draft and its original
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
versions carrying plan data refuse without overwriting the original. Those v5 continuity changes did not alter task/scope schemas; see the current
acceptance-criteria migration below. Full Mission Draft/Issue Graph formation is still open.

## Native check intent v2 and result checkpoint v1

Native run artifacts remain under the private task namespace's `runs/<run>/`,
outside its worker-mounted `worktree/`. They are independent of the desktop
execution ledger and do not change task schema16, conversation schema17 or scope
schema2. The immutable start marker keeps its existing schema1 contract.

| Artifact | Version and retained fields | Publication boundary |
|----------|-----------------------------|----------------------|
| `execution-boundary.json` | `schema_version: 1`, `task`, `run`, `baseline` | Before worker preparation |
| `check-launch-intent.json` | `schema_version: 2`, `contract_version: 1`, `task`, `run`, `baseline`, `mission`, `system_roots`, `request_digest`, complete `request` | Before invoking the check provider |
| `check-result.json` | `schema_version: 1`, `intent_sha256`, `request_digest`, complete `receipt` | After provider return, inside the same blocking closure and before async worker finalization |

Contract version1 compares the request with the exact canonical managed worktree,
approved files and check argv, Mission/run/baseline authority and fixed native
Bubblewrap/prlimit policy. It records the ordered supported system-mount subset
chosen at launch; recovery does not infer those mounts from current host defaults.
The environment is exactly `PATH=/usr/bin:/bin`, `HOME=/tmp`, `CI=1`; input and shell
execution are absent. Limits remain 60 seconds, 16 KiB aggregate output, 8 GiB
address space, 32 MiB file size, 1,024 open files, 128 processes and one second of
descendant grace. A later builder policy must use an explicit new contract version.

`intent_sha256` hashes the exact saved intent bytes. Request and receipt digests,
schema/effect/provider identity, derived receipt identity, timestamps, owner/child
PID and start identities, status/exit/effect combinations, output byte counts and
hashes are validated independently. A matching `rust-shadow` label or successful
JSON decoding alone grants no recovery authority. Native retained stdout/stderr are
bounded private evidence, unlike the desktop ledger's redacted output records.

Readers and writers reject symlinks, special files, unsupported versions, malformed
or oversized artifacts and mismatched identities. Serialization is bounded before
exclusive creation; files and their parent directory are synchronized, and existing
bytes are never overwritten. The start marker is capped at 4,096 bytes; intent at
999,424 bytes; result at 176,128 bytes; reconstructed evidence at 180,224 bytes.
The larger bounds account for approved request maxima and JSON escape expansion.
Partial publication remains uncertainty; a publication error prevents worker success.
Digests detect corruption or substitution but do not authenticate against a same-user
actor coherently replacing all private records.

Under the stopped worker owner lock, explicit `/recover ID` first prefers valid
saved final evidence and its original terminal outcome. Existing malformed final
evidence blocks reconstruction and remains untouched. With absent final evidence,
a matching start marker and absent intent/result retain the pre-check Failed path.
A matching schema2 intent and validated terminal result allow only **Failed:
interrupted after check; candidate not finalized**, retaining the exact receipt and
output with empty patch and no candidate. A zero exit cannot establish finalization
or acceptance. Evidence is exclusively published before the existing deterministic
`finish:<run>` transaction; retries return the same acknowledged result.

Eligible receipt states are completed, failed, cancelled, timed-out, output-limit
and start-failed with their exact provider combinations. A structurally valid
outcome-unknown checkpoint may retain normal worker diagnostics, but neither it nor
any reconciliation-required result permits after-check recovery. Executing receipts,
legacy schema1 check intents, missing post-launch results and corrupt/partial files
remain ineligible. Old intent files are never upgraded by inferring a request.

Recovery invokes no Git, model or check and respawns no worker. It preserves partial
work, signals no process and grants no worktree reuse, retirement or cleanup.
Owner release and a terminal check do not prove quiescence of later helpers.
Repairs are new separately approved tasks; the failed original still blocks its
dependencies. See the [active slice](../Tasks/native-check-result-recovery.md) and
[verification record](../Reports/2026-09-27-check-result-recovery.json).


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

### Human-review repair exclusivity

A repair child awaiting human review counts as unresolved. New `/repair` proposals
from its parent refuse until that hold is explicitly resolved. This guard applies
under the task transaction lock, including after restart, without changing task
schema v12. Older v12 receipts that already created a sibling remain readable and
exactly replayable; the application does not discard or reinterpret saved work.
Resolving the held child's review can then support a separately proposed repair
with fresh approval.

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

## Local Agent repair continuity

Each newly requested worker conversation records a mission-local Local Agent
identity in its evidence. After a complete model response, Alfredo retains the
exact user/assistant exchange in `agent-conversation.json` beside the run evidence,
with a SHA-256 reference in that evidence. This is application-managed conversation
continuity through ordinary chat messages; it does not depend on hidden server
memory or claim reuse of an inference process.

On `/repair`, the first rejection can continue the prior Local Agent: the next
request includes its retained user/assistant messages followed by the current
repair prompt. The second or later terminal rejection in the repair ancestry
starts a fresh Local Agent and sends only the current prompt plus verified repair
evidence. Needs repair reviews do not themselves count as rejections. Every repair
still requires separate approval and uses the original baseline with current exact
file/check permissions. Historical messages grant no additional permissions, and
prior patches are not automatically applied.

A changed model, legacy run without recorded conversation, or incomplete prior
model exchange starts a fresh conversation with an explicit reason. Referenced
history needed for continuation is checked for regular-file identity, bounds,
hash, run/model/agent binding and alternating roles before claiming the repair run.
Missing or damaged referenced history refuses; it is not silently replaced.

History is bounded to eight messages and 512 KiB of content, reserving the existing
128-KiB response budget before each request. Reaching the history budget explicitly
starts a fresh Local Agent while preserving the current repair prompt. Serialized
conversation files are limited to 4 MiB and written exclusively and durably. The
original evidence/context/check limits remain in force. A disconnect before model
completion does not create a complete retained exchange or replay effects.

Evidence review shows fresh/continued identity and the reason. Legacy evidence
shows conversation unrecorded. Task schema v13, conversation-set schema v6 and
scope v2 remain unchanged; the separately retained agent transcript has schema v1.
Review-triggered automatic repair proposal/launch, Architect revision routing,
qualified model selection and retention/storage lifecycle remain unfinished.

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

### Explicit repair resolution (task schema v15)

`ResolveRepair { task }` selects one accepted repair for its failed, rejected or
cancelled ancestor chain. Receipt replay derives the mapping without changing
original outcomes or dependency IDs. One immutable selection closes the family:
no unresolved branch or human hold may remain, and later repair/review mutations
refuse. Exact historical request retries still return their receipts. Contracted
repairs require recorded passing criterion review; legacy boolean acceptance
cannot supply missing proof. Transactions reread source and ancestor evidence;
dependent preparation verifies actual Git candidates before execution.

`DependencyInput.task` retains the declared ID. Optional `source_task` records an
accepted replacement only when different; absence retains legacy meaning. New
ordinary Starts bind the current resolution, while repairs retain their parent's
exact baseline and input vector. Schemas below v15 reject resolution receipts and
replacement input fields. Upgrades preserve exact version-named backups.


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

### Native conversation schema v8: task observation references

Conversation schema v8 adds bounded `Session.task_receipts` records (`after_messages`, `revision`, `task`, `correlation`) separately from inference `messages`. Anchors are complete turn boundaries; revisions are strictly increasing per conversation and correlations unique. The reader refuses malformed references and references asserted under schema v1–v7. Upgrade preserves the exact previous file as its versioned backup and refuses conflicting backup bytes. Task authority remains schema16; references neither replay nor authorize actions. Rendering rechecks canonical receipt identity. Unknown historical chat/action ordering remains in Activity.


### Native conversation schema v9: stable reading blocks

`reading.block_anchor` optionally records a typed message/receipt key plus block-local line and wrapped row. The numeric anchor remains for older-file migration and compatibility. Only schema9 accepts block metadata; the owning message or saved receipt must exist and line/row bounds must validate. Upgrade preserves exact v8 bytes under the existing versioned backup contract. Reading preferences grant no task authority.


### Native conversation schema v10: persisted command intents

A Session stores command records separately from model messages: immutable typed intent, fingerprint id, originating message boundary, shared presentation sequence, bounded text, attempt and pending/submitted/unknown/refused state. Task observations gain an optional sequence (legacy zero remains ordered before new entries). Commands reserve outcome capacity against a normalized immutable base. Exact successful Autosave acknowledgment of the complete Pending record, including attempt, is required before in-memory dispatch. Pending/submitted commands restore as unknown without replay; receipt styling is derived only from matching canonical task or scope receipts. Older schemas refuse command/sequence metadata and preserve exact migration bytes.


### Native conversation schema v11: worker lifecycle reading bounds

Schema11 expands only Run command reading bounds for a separate result line. No new task authority, run execution or receipt data is persisted by this change. Start/Finish phases derive from current canonical receipts and the saved Run intent. The resolver requires the exact Start identity, derived run id, baseline/inputs, and a later deterministic Finish with matching task/run/evidence digest. Older snapshots retain exact versioned backups and migrate without replay.


### Native conversation schema v12: planner command provenance

Schema12 admits `Intent::Planner` with a bounded, immutable planner request and `CommandState::Planner` with a generated, stopped or failed outcome. Requests distinguish generation, revision, explicit Architect revision and cancellation; revision/cancellation bind the preceding draft digest or active generation identity. Generated outcomes retain the draft SHA-256 and step count. These records describe draft operations and never substitute for task or scope receipts.

A saved completed draft may retain its exact planner request as `origin`. Restoration validates that binding against the draft and can reconcile its originating unresolved command from the retained draft. Pending/submitted operations otherwise restore as unknown without automatic inference. The state preserves prior completed draft provenance across failed or cancelled revisions. Older schemas reject planner intent/outcome/origin metadata and retain exact versioned migration bytes.

Planner failures are bounded to 256 UTF-8 bytes; generated results contain a 64-character digest and 1–16 steps. Existing conversation byte limits and reserved command-outcome capacity apply before dispatch. The regular command block retains one outcome line; Run commands retain their separate Start/result lines. Automatic Architect generation extends this planner-command lifecycle through schema15 below. Explicit execution controls use schema13 below.


### Native conversation schema v13: controller command provenance

Schema13 admits `Intent::Control` with exact correlation, process-controller identity and typed operation, plus `CommandState::Control` outcomes `CancellationRequested` or `DispatchChanged { enabled }`. A cancellation request binds task, worker Start correlation and Start receipt revision; a dispatch request binds the desired state, expected process-local dispatch epoch and the observed scope revision when enabling. These presentation records do not become task, scope or execution receipts. Older schemas reject controller intent/outcome metadata and retain exact migration bytes.

The exact saved Pending entry gates dispatch as for other prepared commands. Restored pending/submitted commands remain unknown without replay. A terminal dispatch outcome describes only its originating controller; the new controller still starts with dispatch off. Controller identities and dispatch epochs prevent a stale explicit retry from toggling a different or superseding live process state. Cancellation requires the exact local worker owner and Start identity before signaling.

The cancellation command has a local request outcome and a fixed, separate canonical worker-result line. It skips the Start claim in presentation and resolves Finish through the same exact task/run/baseline/input/evidence binding as a Run command. Neither Finish nor worker disappearance rewrites the local request outcome. Schema13 expands cancellation command reading bounds by one logical line. Task schema16 remains unchanged; automatic worker launch provenance is captured by schema14 below.


### Native conversation schema v14: automatic worker launch provenance

Schema14 admits `Intent::DispatchRun` containing the exact worker Start correlation, expected task revision, task id, approval receipt revision and source dispatch-enable controller request. The originating Session must already contain the matching earlier controller command with its acknowledged enabled outcome. This link is presentation provenance; task approval and execution admission remain separate checks against canonical state and the live controller. Older schemas reject automatic launch metadata and retain exact versioned migration bytes.

Automatic admission reserves the same command/outcome capacity and waits for exact saved Pending acknowledgment. It preserves the Session's composer and reading position. Launch blocks have separate Start and Finish slots, using the same exact run, baseline, inputs and evidence binding as explicit Run commands. Task schema16 is unchanged.

Restored unresolved launches become unknown without replay, and the newly created controller starts with dispatch off. The source request retains its process identity and epoch, so a historical enable cannot authorize a new controller or a superseding dispatch configuration. New operation intent is required to resume automatic work.


### Native conversation schema v15: automatic Architect draft provenance

Schema15 admits `Intent::ArchitectDraft` with a bounded inner planner request for an Architect operation and the exact triggering `ReviewArchitecture` task request. Its origin binds task, review revision, failed run and evidence digest. The originating Session must contain the matching earlier Task command; canonical review receipt validation at admission remains necessary, independent of the parent command's local presentation state. Older schemas reject the new intent and preserve exact migration bytes.

Automatic Architect records use the existing bounded planner outcomes and one outcome line. The same inner planner identity cannot appear in multiple explicit or automatic command wrappers across the snapshot. Shared planner-origin lookup binds current saved drafts and incoming outcomes to one exact command. Neither the review source nor generated summary substitutes for a separate Plan save or task approval. Task schema16 remains unchanged.

Generation waits for exact saved Pending acknowledgment. Restored pending/submitted entries become unknown and never resume inference. A matching retained completed draft can reconcile its origin without regeneration; historical source metadata does not authorize replay. Automatic insertion preserves composer state and stable block reading anchors under the existing conversation size and outcome reservation limits.


Withdrawing a pending automatic Architect operation changes its existing command to Refused and removes only its in-memory dispatch token. A previously launched save completing later cannot restore that token. This adds no persisted authority or schema fields. Final planner guards run after shared model-slot admission and before the HTTP request, checking the expected task revision and exact Architect task/review/run/evidence identity against canonical state. A stale queued request fails without model invocation, using the existing bounded planner outcome. The guard does not reread captured reference evidence, refresh scope binding, or hold the task lock throughout HTTP.


### Native conversation schema v16: turn-bound Wayfinder scope intents

Schema16 admits `Intent::Wayfinder` with an exact understanding request and the index of its originating user message. The index must identify an existing user message, and the command boundary is exactly two messages later, after that turn's assistant slot. The request must match the source turn's deterministic entry, four-field draft or explicit revision confirmation. The compact action text does not duplicate the full prompt; model messages remain unchanged.

A delayed Wayfinder preparation may append a newer sequence at an earlier message boundary. Validation admits this bound exception without relaxing ordinary command chronology; rendering orders references by message boundary and sequence. Stable command keys continue to preserve reading anchors. The existing command count, byte limits and terminal-outcome reservation apply. Older schemas reject the new intent and retain exact versioned migration bytes.

Exact saved Pending acknowledgment gates scope dispatch. Receipt reconciliation requires full request equality in the current validated scope journal, not a matching correlation or reply string alone. Restored pending/submitted entries become unknown without replay; an exact existing receipt may acknowledge them. Task schema16 and scope schema2 are unchanged, and scope confirmation cannot approve or start a task.

### Native conversation schema v17: selection origin and arrival

Schema17 admits `Intent::Selection` and `Intent::SelectionArrival` with an immutable
selection request, plus `CommandState::Selection` with a bounded local outcome.
The request binds correlation, startup or exact source workspace/mission/conversation/
session, repository Open/Create choice, mission Resume/Start New choice and target
conversation namespace. Source commands require a conversation origin; arrivals
accept startup or another conversation origin and only prepared/selected outcomes.
Older schemas reject these fields and the first upgrade preserves exact v16 bytes.

Selection and arrival use the existing command sequence, capacity reservation and
fixed four-line reading block. Appending either preserves messages, draft and reading
state. The source command and destination arrival describe one immutable request;
they do not become task receipts or model messages. Journal reconciliation requires
full request equality and matching conversation identity, not correlation alone.
Missing or invalid proof remains unknown. Restored unfinished dispatch does not
recreate repositories, missions or an in-memory handoff.

### Native selection journal schema v1

`<state-dir>/rust-selection-v1/selections.json` stores independent `{request, outcome,
dispatched}` records, including startup before a conversation exists. The journal
allows at most 512 records and 12 MiB, with a maximum 32 KiB serialized request and
reserved outcome space. Failure text is bounded to 256 UTF-8 bytes. Exact requests
validate absolute paths, namespace bounds, source session and Create/Start New
compatibility. Duplicate correlations, malformed content and future schemas refuse
without replacing the original. Updates use an exclusive transaction lock, synced
temporary file, rename and directory sync; symlinked journal ancestors refuse.

Admission saves `Admitted` with `dispatched=false`; consuming its one-use token saves
dispatch before effects. A saved request cannot recreate that token. The outcome
records completed `RepositoryReady`, `MissionReady`, `TargetLoaded`,
`HandoffPrepared`, `Selected` or `AlreadyCurrent` observations and an optional failure.
Handoff preparation is saved before swapping; selection is recorded afterward.
An interrupted dispatched record remains uncertain, since an effect may precede
its next observation. Partial artifacts remain for inspection and an explicit new
Open/Resume decision. Task schema16 and scope schema2 remain unchanged.

### Native inference admission ledger

The independent coordinator ledger uses schema1 under canonical
`/tmp/alfredo-inference-<effective-uid>/<sha256-normalized-origin>/`. It records the
HTTP(S) origin, configured capacity, next sequence, foreground grant streak and
bounded waiting/active ticket entries. Ticket metadata contains identity, sequence,
foreground/background class and state; prompts and model responses are absent.
Directories and files require the current owner and private 0700/0600 modes.

An owner-file lock establishes a ticket's live eligibility. A retained entry alone
does not authorize HTTP or replay after restart. Transactions clean unowned entries
and retain endpoint directories to avoid splitting live locks. A live capacity
conflict refuses; the capacity can change only once prior owners drain. The ledger
allows 256 total queued/active tickets and 512 KiB, with a bounded 576-entry directory
scan. This is client scheduling metadata, not a canonical task/scope receipt or
qualified inference audit.

Queue observations and client clocks are skipped in Session serialization and
cleared from conversation checkpoints. Conversation17, task16, scope2 and selection
journal1 remain unchanged. Restart never restores a queue position, running timer
or inference permission; automatic coordinator namespace retirement remains open.

### Native inference diagnostic report schema1

Standalone qualification writes a new user-selected report with a 4-MiB ceiling
and retains fixture workspaces/evidence in the sibling `<report filename>.artifacts`
directory. It refuses existing report/artifact paths. The independent schema1 report
contains a manifest and checksum, exact scenario schedule, fixture-definition and
executable digests, selected endpoint/model/settings, bounded runtime observations,
case phases/results and at most 128 recorded generation attempts. Its own checksum
binds retained structure; it is not publisher or model-server authentication.

The schedule pairs baseline and foreground-8192/background-16384 context profiles
for four scenarios and 1–3 repetitions, at fixed shared capacity one. Pending,
running and finished checkpoints are distinct. Finished cases retain failure and
missing-proof outcomes; a complete schedule does not imply successful qualification.
Reload validates the schedule, identities, bounds and eligible outcomes without
opening missions or replaying inference. No promotion state is written.

Request records retain actual serialized body hashes, canonical profile hashes,
ordered message hashes/byte lengths and reusable prefix identity; raw prompts and
streams are absent. Optional per-request `runtime_after` or bounded `runtime_error`
records inspection after the terminal generation frame while capacity remains held.
`generation_ms` ends before inspection; `runtime_probe_ms` measures inspection and
`total_ms` covers the instrumented request. Dropped futures preserve interruption
and missing proof rather than disappearing from an attempted case. Hashes cannot
reconstruct source bytes or attest upstream execution. Runtime binary pinning and
exact templated token headroom remain unverified.

This diagnostic report is independent of task16, conversation17, scope2, selection
journal1 and the inference admission ledger; those formats remain unchanged.

Accepted cases require a complete ordered request set for their planner, worker(s)
and optional foreground discussion. Retained attempt counts, explicit classes,
request identities and the fixed discussion prompt must agree. Foreground queue
observations reference their own background request sequence and are captured only
while that generation is pending, excluding the subsequent runtime metadata probe.
Canonical review references preserve the exact receipt task (the child for a
compound repair) separately from the reviewed parent task. Source and repair
lineage relationships are revalidated on reload.
