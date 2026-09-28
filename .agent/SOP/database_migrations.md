# Persistence migrations — Alfredo

**Last Updated:** 2026-09-27

Alfredo uses versioned JSON runtime stores, not SQL, Flutter, or sqflite. The
previous contents of this SOP were an unrelated template; the current schema
source of truth is [Persistence Schema](../System/database_schema.md).

## Store ownership

The existing desktop's Python Orchestrator owns its canonical mission, workspace,
permission and evidence stores. The new Rust terminal owns only the separately
namespaced task and conversation stores described in [Rust terminal commands](../../alfredo-tui/README.md).
Do not infer migration compatibility from a similar field name. GitHub issue
migration is a separate historical operation and is not runtime state migration.

## Changing a schema

1. Read the owning store, its current version and all reader/writer call sites.
2. Define the old-to-new mapping, retained identity, defaults, and explicit failure
   behavior. Missing authority cannot be defaulted to approval or completion.
3. Add old-version restart fixtures and malformed/future-version rejection tests.
4. Keep original bytes intact until conversion and validation both succeed.
5. Publish the converted state under the owning cross-process lock using a synced
   sibling file and atomic replacement. Preserve a recoverable original when a
   destructive conversion is unavoidable; do not remove it from weak age heuristics.
6. Update the schema documentation, migration evidence and orchestration context.

## Rust task store v1/v2/v3/v4/v5/v6/v7 to v8

State stays at `rust-tasks-v1/<workspace-mission-sha256>/tasks.json` outside the
coding workspace. Readers accept task schemas 1 through 16; future versions fail unchanged.
Before the first mutation of a prior task schema, the writer saves the exact original bytes to
`tasks-vN-backup.json` for the source version using exclusive creation and sync. An existing backup must
match. Publication then uses the same locked, synced atomic replacement.

Version 1 approval records scheduling intent only. Version 2 adds explicit
file/check policy, run identity/baseline, evidence digest and execution/review
statuses. Old tasks receive no inferred policy or run. Setting policy returns a
task to Proposed and requires fresh approval. Receipt replay validates both
versions; v1 cannot carry v2 action kinds. Version 3 adds optional `repair_of`
and the Repair receipt. Earlier versions cannot carry Repair receipts; old tasks
retain absent repair lineage and all existing identities/permissions. Version 4 adds dependency inputs to Start receipts and TaskRun records. Existing
runs retain empty inputs; older schemas cannot claim composed dependencies. The
migration regression covers each source version. Version 5 adds accepted-candidate
Branch receipts, with no inferred branch links in older state. This is not a Python-store importer.
The migration regression proves exact backup, retained identity, denied bare
approval execution and approval reset. Worker evidence is stored separately and
verified by its digest before completion/review acknowledgement.

## Native run checkpoint compatibility

The check-result slice versions private run artifacts independently: the start
marker remains schema1, new `check-launch-intent.json` files use schema2 with
`contract_version: 1`, and `check-result.json` uses schema1. Task schema16,
conversation schema17 and scope schema2 are unchanged. See the
[exact artifact contract](../System/database_schema.md#native-check-intent-v2-and-result-checkpoint-v1).

Only new execution records may bind a complete authorized request and its canonical
digest. Do not rewrite a legacy schema1 check intent, recreate a missing intent from
current worker defaults or worktree contents, or supply missing receipt/output
fields. Contract version1 validates its recorded system-mount subset without
reinterpreting history from the current host; a changed builder policy needs an
explicit new contract. Old start markers and valid final evidence retain their
existing meanings. Merely installing a newer reader grants old runs no new proof.

Intent/result/evidence publication is bounded and exclusive, followed by file and
parent synchronization. Never delete or replace partial, unsupported, mismatched,
symlinked or malformed artifacts to make recovery eligible. Publication failure
prevents success. Under a stopped-owner guard, prefer valid final evidence and
preserve damaged existing evidence. Only when final evidence is absent may a
verified terminal checkpoint publish Failed interruption evidence with retained
check output, empty patch and no candidate, then reuse deterministic `finish:<run>`
reconciliation. Zero exit is not worker finalization. Uncertain or
reconciliation-required receipts and incomplete post-launch results stay unresolved.

Recovery is explicit and performs no model, check or Git work, worker respawn or
process signaling. It does not authorize worktree reuse, cleanup or retirement and
does not establish complete Runner Quiescence. A repair is new separately approved
work; dependencies remain blocked on the failed original. Hashes are corruption and
binding checks, not authenticity against a same-user actor replacing private state.
No destructive runtime conversion or inferred task approval is part of this slice.

Compatibility verification covers legacy start/intent records, intact final-evidence
precedence, damaged final preservation, request/receipt corruption, publication
failure, process-death cuts, fresh-store and concurrent recovery, exact Finish replay
and a normal full-worker checkpoint/final-evidence binding. Consult the
[active plan](../Tasks/native-check-result-recovery.md) and
[verification record](../Reports/2026-09-27-check-result-recovery.json) for results and
limitations rather than inferring production parity from artifact compatibility.

## Rust conversation snapshots v1 to v2

Readers accept v1 conversation state with default task-view preferences. New v2
snapshots retain task/chat mode, selected task ID and bounded search; this grants
no scheduling, evidence or review authority. Version 1 cannot carry those fields.
The first v2 save validates both states under the existing owner lock, preserves
exact old bytes in a `.v1-backup` sibling, then publishes a synced atomic snapshot.
A differing existing backup, malformed original or future schema refuses unchanged.
Interrupted conversations still restore without replay. Rollback requires stopping
the terminal and restoring the backup; v1 readers cannot consume v2, and subsequent
conversation changes would be lost if replaced with the older backup.


## Required verification

Exercise valid old state, unsupported future state, malformed bytes, lost-response
receipt replay, conflicting requests, concurrent writers, write failure and
restart. Confirm that failed reads/conversions do not erase or replace the source.
Run focused store tests first, followed by the relevant runtime/terminal journey
and broader regression gates in [Development Workflow](development_workflow.md).

Task schema v6 adds atomic Plan receipts that expand validated ordered steps into
Proposed tasks only. Versions 1–5 gain no inferred plan provenance or authority.
Migration tests cover all five source versions; a downgraded state containing a Plan
receipt is refused unchanged. Published task plans restore, while transient model
drafts are not conversation snapshots or durable task records.

Task schema v7 adds Assign receipts for unstarted worker-model changes. It preserves
policy, dependencies and historical plan assignment while resetting approval.
Versions 1–6 receive no inferred reassignment and cannot carry Assign receipts;
migration fixtures cover all six source versions with exact-byte backups.

Task schema v8 adds optional committed repository context to Plan receipts. Old plans
retain absent context; no prior repository inspection is inferred. Readers reject
context-bearing plans in schema 1–7. Migration tests cover all seven source versions
with exact-byte backups; grounded plan replay retains the original selected inputs.

## Advisory mission discovery identities

Opening a native mission may create an immutable version-1 `identity.json` name hint
inside its existing workspace/mission namespace. This is optional discovery metadata,
not a task or conversation schema migration. Existing task journals supply name hints;
old conversation-only namespaces require one manual named opening before registration.
Readers validate version/bounds/namespace hash, skip malformed hints and never repair
or overwrite them automatically. Startup warns on failed registration while preserving
normal state validation and manual selection. Discovery itself creates no files.

## Explicit native mission admission

New Start New requests publish mission.json version 1 under the task namespace lock.
This identity record does not migrate task/conversation schemas or grant approvals.
Existing legacy task or conversation state remains resumable without generating a
new identity record. Any existing mission/task/conversation data prevents Start New;
corruption is not treated as absence. Resume never creates identity from an advisory
cache alone. Preserve invalid files; uncertain creation acknowledgment is reconciled
by explicit Resume, not by overwriting or generating another name automatically.
Older binaries ignore this admission record; concurrent mixed-version creation does
not provide the new exclusion guarantee.

## Native understanding journal v1

The explicit /scope flow introduces a separate workspace-scoped journal; it does not
upgrade old task/conversation or desktop Wayfinder state. Missing native state is
outside the flow, not confirmed. Pending and confirmed projections must replay from
revisioned Mission Commander receipts. Unknown versions or corrupt state refuse new
governed work unchanged. Lock ordering is understanding before task namespace; exact
replays and completion/review/cancellation remain distinct from new planning authority.
Draft admission reserves count and encoded-byte capacity for its confirmation.

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

## Rust conversation snapshots v1/v2/v3 to v4

V4 persists bounded logical reading anchors alongside numeric bottom offsets. Old
snapshots default to no anchor and use their numeric offset until first render.
Upgrade retains the exact source file in `.v1-backup`, `.v2-backup` or `.v3-backup`
under the existing owner lock; a differing backup prevents publication. Downgrades
and invalid anchor/version/offset combinations refuse unchanged. Restore does not
replay interrupted inference. Tests cover unseen output plus restart/resize, exact
v3 backup conflicts and invalid anchors; older migration coverage remains active.

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

Accepting a repair child still does not resolve dependency IDs referencing its
rejected parent; governed replacement/resolution is unfinished. Architect failure
classification and revision routing, review-triggered execution authorization and
full launch qualification remain open.


Task v15 introduces immutable ResolveRepair receipts and optional dependency source_task. Versions 1–14 preserve exact-byte backups before mutation; older schemas reject the new authority fields. Historical inputs and original outcomes remain unchanged.


Task v16 adds typed architecture failure routing and Plan.architecture origin; conversation v7 adds saved Architect draft provenance. Prior versions reject new authority/provenance and retain exact version-named backups before mutation. Historical review receipts do not infer failure classifications or trigger inference.

## Planner command provenance (conversation schema v12)

Conversation schema12 adds typed planner intents/outcomes and optional exact origin
on a retained draft. The first write preserves exact v11 bytes. Older schemas refuse
these fields. A draft origin must have a matching saved command; any matching recorded
Generated outcome must agree with its digest and task count. Pending operations restore
as unknown without inference replay, except when the retained draft proves the exact
origin's generated result. Terminal planner outcomes cannot be retried under the same
identity; submit a new command. Task schema16 and approval authority are unchanged.

## Process-bound controls (conversation schema v13)

Schema13 admits typed Control requests and matching local outcomes. Every request
binds a process controller. Cancellation additionally binds the worker's Start
correlation and receipt revision; toggles bind the controller epoch and enabling
scope revision. These records never authorize replay during restoration. Unfinished
commands restore as unknown; terminal outcomes remain historical and cannot be
retried under the same command identity. Older schemas reject control metadata, and
first publication retains their exact versioned backup. Cancellation blocks add one
fixed logical result slot, resolved separately from canonical Start/Finish binding.
Task schema16 is unchanged.

## Automatic launch provenance (conversation schema v14)

Schema14 admits DispatchRun requests containing the exact enabling Control request,
worker correlation, task revision, task and approval revision. Validation requires
the matching enabled Control outcome earlier in the same Session. Cross-session,
orphan, mismatched, disabled and unsupported-schema references refuse unchanged.
Automatic entries share the Run claim/result reading bounds. Exact v13 bytes are
preserved before first upgrade publication. Restored unfinished launches are unknown;
no in-memory dispatch token or controller is restored, and automatic entries cannot
be retried under their historical identity. Task schema16 remains unchanged.


## Automatic Architect provenance (conversation schema v15)

Schema15 adds ArchitectDraft: an exact inner Architect planner request and its
triggering ReviewArchitecture request. The earlier matching Task command must be
in the same Session. Inner planner correlations must be unique across explicit
and automatic wrappers, and retained draft origins use that same identity.
Older schemas reject the new metadata; first publication preserves exact v14
bytes. Unfinished operations restore as unknown without inference replay.

Pending automatic withdrawal uses Refused state and removes the transient dispatch
token; it adds no schema field. Final post-slot planner admission guards likewise
use existing failure outcomes. Task schema16 is unchanged.


## Turn-bound Wayfinder scope actions (conversation schema v16)

Schema16 adds an exact scope request and existing user-message index in Wayfinder
intents. The source must be a user message, match the deterministic request, and
anchor the command after that turn's two-message pair. Delayed preparation may
append this bound entry at an earlier boundary; ordinary command ordering remains
checked. Renderer ordering uses the boundary and shared presentation sequence.

First publication preserves exact v15 bytes; older schemas reject Wayfinder
metadata. Pending/submitted requests restore as unknown without replay, while an
exact canonical receipt can reconcile the originating entry. Message content and
response-source metadata retain their existing formats. Task schema16 and scope
schema2 remain unchanged; confirmation grants no execution authority.

## Workspace and mission history (conversation schema v17)

Schema17 adds exact source Selection and destination SelectionArrival intents and
bounded selection outcomes. These use one fixed outcome slot outside model messages;
older schemas must reject the new metadata. First publication preserves exact v16
bytes, with the existing refusal for conflicting backups. Source and arrival identity
validation and the existing command byte/count reservations apply before handoff.
Task schema16 and scope schema2 are unchanged.

The independent schema1 journal is `<state-dir>/rust-selection-v1/selections.json`.
It captures startup and in-process admission before effects; migration does not
infer historical selection or grant replay. Restored unfinished dispatch remains
unknown with its last observation. Matching terminal records may reconcile exact
conversation entries without another handoff. Do not delete a journal or partial
repository to force a retry. Inspect the exact request, outcome and retained paths;
then explicitly Open/Resume or choose a fresh mission. Preserve malformed state and
restore a known-good backup only with terminals stopped. A later failed handoff
does not undo earlier repository or mission creation.
