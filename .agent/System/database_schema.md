# Persistence Schema

**Last Updated:** 2026-08-02

Alfredo has no relational database and no database migration layer. Authoritative configuration begins in local Markdown/JSON files, while runtime projections are versioned JSON documents stored below the configured app-local runtime root. Each Python authority store listed below uses atomic sibling-file replacement; append or read-modify-write stores additionally use `flock`, and expected-revision action families share a cross-process coordinator lock.

## Tables

There are no SQL tables. The table-like JSON stores are:

| Store | Owner | Key fields | Purpose |
|---|---|---|---|
| `workspace-sessions.json` | `WorkspaceJourneyStore` | Starting Location, Coding Workspace, revision, Active Mission, known Missions, mission catalog, selection and choice receipts | Canonical pre-session workspace/Mission journey and exact restart restoration |
| `runtime.json` | `AlbertMission` | mission, issues, sessions, reviews, delegations, command policy, `workstation_actions`, timeline | Canonical mission runtime, Local Agent lifecycle, and mutation-coincident Workstation recovery markers |
| `workspace-preferences.json` | `WorkspaceSnapshotService` | revision, active mission, conversation scope, operations view, events, `workstation_receipts` | Canonical desktop projection preferences, ordered updates, and idempotent Workstation action acknowledgements |
| `agent-console-history.json` | `AgentConsoleHistoryService` | message id, sequence, role, content, scope, outcome, source, optional correlation id/action phase | Durable unified controller/workstation chronology and idempotent Workstation audit phases |
| `workspace-queue.json` | `WorkspaceQueueService` | revision, queue items, `receipts` | Pending proposals, confirmations, Ad Hoc Delegations, and idempotent proposal/decision acknowledgements |
| `mission-drafts.json` | `MissionDraftService` | revision, drafts, ordered lifecycle receipts | Proposed mission composition, exact replay boundaries, and confirmation recovery before accepted state |
| `working-context-curation.json` | `WorkingContextService` | revision, dispositions | Eligible source pin/exclude choices |
| `activity-journal.json` | `ActivityJournalService` | revision, contiguous entries | Attributed meaningful actions and evidence links |
| `shell-terminal.json` | `ShellTerminalService` | revision, commands, execution owner identity, grants, grant denials | Metadata-only governed commands, crash-safe execution state, expiring path authority, and exact grant/denial decisions |
| `path-grant-requests.json` | `ShellTerminalService` | request id, correlation id, Mission, canonical path, access, duration, reason, affected action, requested time | Append-only typed contextual authority requests that survive restart independently of mutable terminal decision state |

Bulky raw command output, worker prompts/responses, test logs, and `review.diff` files live under per-session artifact directories and are registered on the owning session rather than embedded into the JSON stores or Markdown tracker. Review projections replace eligible host paths with opaque app-local references. The bounded Session Artifact reader resolves only an exact Mission/session/reference tuple and returns text without creating an artifact-content JSON store.

Finalized Agent Console user/controller turns remain in the durable full history store so conversation continuity survives restart. Only the Working Context extraction used for one model turn is windowed and content-bounded.

## Non-Authoritative Continuity

- The launcher keeps best-effort `recent-workspaces.json` and `launch-context.json` below the runtime root. A Starting Location is never added to recent workspaces merely because Alfredo was invoked there, and selecting a recent entry is an explicit relaunch only. Authority for restart continuity comes from `workspace-sessions.json`, not either launcher convenience file.
- React uses workspace-scoped browser keys for selected controller, local card/detail continuity, and at most 100 terminal negative Workstation action turns with per-turn content bounds. Corrupt, pending-only, or accepted-only local records cannot create canonical state.
- `MissionSessionSummary.last_activity_at` is derived from validated session timestamps (terminal end/cancel/start fallback) rather than stored as a separate mutable authority record. `runner_started_at` is projected separately from its exact durable session field for R6 evidence. Malformed timestamps fail projection validation; absence renders as `Not recorded`.
- Production measurement JSON Lines and correctness-gate files are append-only, non-authoritative evidence outside the runtime stores above. They bind fixture bytes, clean source archive, packaged artifact, variant, cohort, correlation, and monotonic stage marks but cannot mutate or replace Mission truth.

## Relationships

- `project_key + mission_id` isolates each `runtime.json` namespace.
- Workspace preferences reference one active Mission from the in-memory mission catalog; Mission summaries reference their own sessions and queue attention.
- Every Local Agent session retains its Issue Slice or `ADHOC-*` id, assigned agent, task packet, worktree, evidence, artifacts, and runner ownership identity.
- Agent Console messages retain the exact Mission-qualified Conversation Scope captured for their originating prompt.
- Workstation request/acknowledgement Console turns retain a unique `(correlation_id, action_phase)` marker so replay can restore either missing phase without duplicating the other; ordinary chat retains empty backward-compatible marker fields.
- A syntactically valid Mission Commander Workstation action that reaches Python but is rejected still appends durable `request` then `rejection` Console phases. A transport failure that never reached Python has no backend audit claim and is eligible only for bounded browser continuity.
- Workspace Queue items and Mission Draft entries retain Mission ids and originating message/item ids.
- Queue inspection retains canonical resolved history for replay/audit, but the workstation Queue projection is intentionally pending-only. Hiding resolved items or standing creation forms is a React projection rule and never deletes `workspace-queue.json` or `mission-drafts.json` state. Issue Assignment similarly filters canonical snapshot rows by parsed `work_type`/`tracker_status` without mutating tracker/runtime stores. See the [2026-07-12 acceptance correction](../Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md).
- Mission Draft lifecycle receipts retain the exact request, prior draft state, effect draft, acknowledgement, and ordered revision. Current receipts must derive one canonical draft chain; exact replay rejects changed reasons/effects, coherent receipt substitution, missing predecessors, and downgrade from the current receipt contract. Confirmation recovery uses the receipt's immutable accepted-Issue identity while allowing later separately governed Issue fields to evolve.
- Workstation session task packets and the Mission-level `workstation_actions` ledger bind a correlation id to the normalized request in the same runtime write as the authoritative mutation. `workspace-preferences.json` then records the acknowledgement receipt. Exact retry uses either layer to finish/replay acknowledgement and reconcile audit side effects; reusing the correlation id with a different boundary is rejected. Workspace Queue stores its request and acknowledgement together in its own atomic receipt.
- Activity entries link Mission, Issue Slice/Ad Hoc Delegation, session, queue decision, command, and evidence identities without becoming canonical lifecycle authority.
- Shell command records retain the submitting Mission, exact request correlation, approval/denial boundary, and durable `executing | outcome-unknown | completed | failed` state so approval/completion remains attributed correctly after an Active Mission switch. An execution marker is persisted before process start; a dead owner is durably converted to `outcome-unknown` and never automatically re-executed. Missing request/decision/final audit phases reconcile before unrelated Console or Journal entries advance.
- Each contextual path request has a unique request id and binds one canonical non-symlink path, correlation, Mission, access level, fixed duration, reason, affected action, and validated timestamp. Grant and denial records reference that request id; creation accepts only the exact pending boundary, while replay, changed authority, malformed timestamps, and duplicate ids fail closed. Projection derives `pending | granted | denied` from the append-only request plus terminal decision records.
- Session artifact registrations retain backend paths only inside authoritative runtime/session state. UI projections receive opaque references and bounded content, never a host path field.

## Indexes

No database indexes exist. Stores use stable ids and in-memory dictionaries/lists. Runtime/session lookup is dictionary-based; histories and journals preserve sequence order. The bounded local scale does not currently justify an embedded database.

## Migration History

- Legacy unstarted `launched` sessions migrate to executable `queued` state on load.
- Missing optional fields use backward-compatible defaults when records are decoded.
- JSON schemas remain versioned at projection boundaries; malformed or non-contiguous state fails with structured persistence/contract errors instead of being silently repaired.
- 2026-07-11 added Mission-qualified action/message identities, runner process identity/recovery metadata, Shell submitting Mission plus executing/outcome-unknown recovery, unique atomic temporary files, lock-safe transactions, default-empty Workstation/Queue/Mission-Draft receipt collections, Mission-runtime Workstation recovery markers, correlated causal audit phases, and opaque review-artifact projections.
- 2026-07-12 added typed append-only `path-grant-requests.json`, exact request-linked grant/denial replay validation, and timestamp-backed session activity. React also keeps a bounded workspace-scoped browser record for transport-failed action display; that local UI continuity is explicitly non-authoritative and is not part of the Orchestrator schema.
- 2026-07-12 corrected Markdown metadata parsing across an H1/blank-line header and added `work_type` to Issue Slice summaries. No JSON store migration was required; the fields originate from tracker Markdown and only refine desktop projection.

If a future workload outgrows bounded JSON stores, migration must preserve Orchestrator authority, expected-revision semantics, append order, Mission isolation, and artifact separation before replacing this design.
