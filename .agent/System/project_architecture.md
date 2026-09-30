# Project Architecture — Alfredo

**Last Updated**: 2026-09-27

## Overview

The 2026-09-13 user request starts a native Rust terminal rewrite in
[`alfredo-tui/`](../../alfredo-tui/README.md). Its initial conversation path uses
Ratatui/Crossterm with asynchronous Reqwest/Tokio Ollama streaming. A bounded channel isolates input/rendering
from inference, and session/attempt identity prevents late cancelled replies from
changing current state. The terminal owns versioned conversation snapshots plus a separate
durable Rust task queue. Named conversation sets hold an exclusive owner lock;
bounded asynchronous snapshots restore drafts/models/cursors and mark saved active
requests interrupted without replay. Normal shutdown orders the final save after
any pending checkpoint. Task proposals, dependencies, approvals and cancellations
flow through one locked revision/receipt transaction off the input thread. F2 and
slash commands render its acknowledged snapshot. `/plan` runs a cancellable,
strictly parsed Frontier Architect draft through the same bounded Ollama admission
pool. The preview exposes worker model, dependencies, exact paths and check argv;
it has no task effect. `/plan-save` publishes the reviewed batch as Proposed through
one schema-v6 Plan receipt. Original prompt/planner and exact steps are replayable;
no approval or launch is inferred. `/assign ID MODEL` validates an
installed model asynchronously and records an unstarted task's worker reassignment
through schema-v7 receipts. Assignment preserves policy/dependencies, resets approval
and rechecks the task revision after catalog lookup. Exact receipt replay needs no
live catalog; missing-model retries cannot bypass catalog admission. Profile/role selection and project-level planning gates remain open. `/activity` projects searchable
revision-ordered activity directly from the same persisted receipt ledger; navigation
creates no second journal and no inferred events. Workers now durably claim explicitly permitted tasks, request structured file
plans, edit detached Git worktrees, invoke the shared Rust execution provider with
network-isolated Bubblewrap, and retain digest-bound review evidence. The execution provider
is `alfredo-tui/src/execution.rs`. Four workers per terminal run off the input thread; snapshots reject late
revision rollback. Each active worker also publishes a process-local Tokio watch
snapshot for stage, elapsed time and received bytes; this bounded advisory stream
never mutates durable task state or blocks a worker on rendering. An optional
shared-provider output callback receives best-effort bounded chunks without
blocking capture threads; terminal observations retain 8 KiB per stdout/stderr
stream and render live check output. Finished workers use saved evidence instead. Per-task OS locks
cover claim through result publication; explicit recovery acknowledges valid final
evidence or records Failed from a proven check boundary only after the owner stops,
without effect replay. Unknown/legacy claims remain unresolved. `/repair ID reason` creates receipt-linked proposed work with
inherited policy and fresh approval; its worker revalidates parent evidence and
uses the parent baseline plus bounded evidence as reference data. Original runs
are preserved. New successful evidence also binds a retained Git candidate commit
with exact parent/diff verification and a managed object ref, without moving the
workspace HEAD. Accepted dependency candidates are verified and composed with the current committed
workspace baseline using merge-tree and managed base refs. Ancestry avoids duplicate
diamond inputs; conflicts or invalid parents refuse dispatch. Run receipts retain
exact parent/run/digest/candidate provenance. Verified evidence is projected once into a readable check summary,
colored unified diff and separate output sections. Opening evidence binds selection
to that task for review shorthand; the store rechecks evidence on review. The explicit `/branch` handoff verifies an accepted candidate, confirms or creates
its deterministic local ref without checkout/overwrite, and records a Branch
receipt. Exact Git ref state supports reconciliation if receipt storage was
interrupted. Active-branch merging, uncertain-child reconciliation, retirement and complete workspace/mission/view
continuity are required next, as
tracked by the [migration plan](../Tasks/rust-terminal-migration.md). The
[terminal report](../Reports/2026-09-13-rust-terminal-foundation.md) records exact
evidence and gaps. Native chats, planners and workers share endpoint admission across
same-user processes (default two, `--parallel-models` 1–8). Foreground discussion and
planning receive bounded priority over background worker model requests; queued
cancellation removes dispatch eligibility. Discovery bypasses admission. This bounds
Alfredo client slots, with no proof of GPU capacity or server-side cancellation.
A separate native Linux x86-64 candidate archive contains the binary, installation
instructions, lockfile and build provenance. Its installed-binary PTY gate uses a
restricted PATH outside the source checkout. Cross-platform qualification and publication remain open.

The native [Mission Work tree](../Tasks/native-mission-work-tree.md) derives Plan
groups, manual work and repair ancestry from canonical task receipts. Dependency
edges remain separate from hierarchy. `mission_work.rs` provides the bounded
read-only projection; `task_control.rs` caches it by snapshot, scope and filter
state and retains exact task identity across view changes. Group focus grants no
task action target. Collapse state remains local, with no canonical schema change.
Asynchronous evidence requests carry local identities; delayed results cannot
replace a later view choice and use reviewer text from the newest admitted
snapshot. [Implementation evidence](../Reports/2026-09-27-native-mission-work-tree.json)
records this bounded slice and its remaining supervision gaps.

The native [check-result recovery slice](../Tasks/native-check-result-recovery.md)
uses `run_boundary.rs` around the real shared provider. Its schema2 check intent
binds the complete authorized request and canonical digest under contract version1;
the schema1 result binds the exact saved intent bytes and returned receipt. Intent
publication precedes launch, and result publication stays inside the blocking
execution closure before async worker finalization. Fixed request policy, recorded
system mounts, receipt identities/status combinations, output bounds and hashes
are validated locally without changing the provider contract.
All artifacts use bounded exclusive creation and file/parent synchronization.
Publication failure prevents a successful worker outcome.

`TaskStore::recover` holds the stopped worker owner lock and prefers intact final
evidence. Existing damaged final evidence is preserved. Otherwise, a verified
terminal checkpoint allows only Failed interruption evidence retaining check output,
with no reconstructed patch or candidate; the existing `finish:<run>` transaction
acknowledges it exactly once. The legacy proven-before-launch path remains. Old
unbound intents, missing/partial checkpoints and uncertain receipts cannot grant
after-check recovery. The reader performs no Git, inference or check and does not
respawn or signal a worker. A terminal check does not prove that later helpers are
gone, authorize worktree reuse/retirement, or establish complete Runner Quiescence.
These artifacts do not migrate task/conversation state; hashes detect corruption
without authenticating coherently replaced same-user state. Exact validation and
crash coverage belong to the
[verification record](../Reports/2026-09-27-check-result-recovery.json); full automatic
runner recovery and native production parity remain separate requirements.

Native task-panel PageUp/PageDown uses the currently rendered panel height with one
row of overlap where possible and at least one row of progress. Resize updates that
height, so the compact 32×10 inspector remains reachable without skipping rows.
Paging is local presentation state and grants no task authority.


### Native terminal startup selection

The Rust startup selector keeps Starting Location separate from an accepted Coding
Workspace. Read-only validation collects an exact repository and Resume/Start New
mission choice before any creation. CLI choices enter the same saved-admission
boundary. `selection_command::Request` binds a startup or conversation origin,
repository choice, mission choice and conversation namespace. Startup has no
conversation owner, so `selection_store` independently syncs the request under
`<state-dir>/rust-selection-v1/selections.json` before issuing a one-use admission.
In-process selection also requires the exact saved source conversation entry.

New repository preparation uses an unused directory, sanitized Git init with an
empty template, and an empty root commit published to main only if that ref is
still absent. The selector discloses this before creation. No project files are
staged, and existing repositories are never auto-committed. Existing targets/nesting
are refused; partial artifacts survive later failure for explicit inspection.
The empty commit enables committed-context planning and isolated workers. Selection
never grants task or scope authority; full mission formation remains unfinished.

Native new-repository creation resolves the runtime state's existing ancestors and
appends only its unresolved suffix before checking workspace separation. Aliased
runtime overlap, dangling symlinks and invalid ancestors refuse before create-dir or
Git init. The post-creation TaskStore validation remains; preflight is not a promise
of cross-process filesystem atomicity.

Saved repository suggestions and per-repository mission names share one bounded,
read-only namespace scanner (1,024 entries/16 MiB). Repository paths deduplicate
across missions and remain advisory even if moved or deleted. The startup selector
fills Open-mode input on Tab, validates on Enter, clears the old discovery result
and scans names for the acknowledged repository. It never infers workspace authority
from cached paths; no additional registry schema or persistence is introduced.

### Native plan scope handoff

After a scope-bound Plan passes current-scope validation at worker admission, the
coding request includes the exact binding from the acknowledged Plan receipt. It
contains destination, scope, constraints and uncertainty as reference alongside the
approved source files. The worker does not re-read mutable project scope mid-run.
File/check enforcement remains independent of model instructions. This closes the
planner-to-worker context gap without changing task v9 or evidence serialization.

## Process-group cleanup identity

On Unix, cancellation cleanup retains an unreaped leader through the configured
SIGTERM grace interval. Its PID/start identity therefore remains verifiable before
forced group cleanup, even when the leader has exited and another group member
ignores SIGTERM. Existing identity/group checks and SIGTERM/SIGKILL paths are retained;
cleanup still requires a reaped leader and quiescent group before success. Failure to
prove cleanup remains outcome-unknown. This may use the full configured grace interval;
no timeout bounds or execution permissions are expanded.

### Native workspace handoff

`/workspace` reuses the repository and Resume/Start New Mission selector inside the
running terminal; Esc returns to the current work. The header shows current mission
and repository. Workstation owns each mission's App, TaskControl and conversation
owner. Saved selection admission precedes repository and mission preparation.
The journal separately records repository readiness, mission readiness and target
loading. The destination owner remains held while arrival history and the latest
source checkpoint save. `HandoffPrepared` records that preparation; only the later
in-memory swap permits `Selected`. A failed publication after swapping cannot claim
rollback. Errors before swapping preserve source work, while any created repository
or mission remains identified. Selecting the same identity records `AlreadyCurrent`
before attempting to acquire its already-held owner.

Switching requires inactive conversations and workers, dispatch off, completed task
operations/model discovery, and a saved or explicitly cancelled plan draft. It never
cancels work or infers task completion. Fresh conversation/catalog event channels and
TaskControl isolate old asynchronous results from restored session IDs. Conversation
schema17 stores source and arrival commands outside model messages and preserves
drafts and reading anchors. On restore, matching journal records reconcile the
exact request; unfinished or unavailable proof stays unconfirmed without replay.
Task schema16 and scope schema2 remain unchanged. The provider admission queue
is shared across same-user processes for each normalized endpoint. Full concurrent supervision across workspaces, mission
formation and production qualification remain open.

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

## Native transcript reading continuity

PageUp moves away from the latest output and anchors the selected conversation to a
logical transcript line and wrapped-row offset. Appending stream text preserves that
anchor. PageDown advances from the current reading point; reaching the bottom resumes
following live output. Starting or retrying a turn also follows the latest text.
Each session retains its own transient anchor, and hidden task/model views do not
render or change the conversation viewport. This also removes underlying chat text
from otherwise blank task-panel rows.

Resize preserves the logical line and clamps its wrapped-row offset to the reflowed
line. Client/server metadata remains pinned independently of transcript scrolling.
Logical lines before the reading point are omitted before applying the widget's
16-bit within-line scroll, so newline-heavy bounded output can show its actual tail
and remain navigable beyond 65,535 rows.

The existing serialized numeric bottom offset remains compatible with conversation
v3. Live geometry and logical anchors are transient and excluded from checkpoints.
Thus exact reading-position restoration across a restart with different geometry,
or after unseen background growth, remains a separate continuity gap; no new schema
or full character-level reflow anchor is claimed here.

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

Selection verification: [native selection evidence](../Reports/2026-09-26-selection-continuity.json) records journal admission, retained-source handoff and installed terminal acceptance.

### Native endpoint-shared inference admission

`inference_admission::Coordinator` keys a private local ledger by effective user and
normalized HTTP(S) URL origin. It uses canonical `/tmp` beneath
`alfredo-inference-<uid>/<sha256-origin>/`, independent of mission state and `TMPDIR`.
URL normalization does not resolve DNS aliases. Different users, machines, origins
and non-Alfredo clients remain outside a coordinator's aggregate bound.

Do not remove or replace scheduler directories or `ledger.lock` while Alfredo runs.
The coordinator never unlinks its transaction lock. Missing or changed owner proof
refuses admission, but these advisory locks do not prevent the same user from
replacing the scheduler namespace and breaking shared coordination.

Each request owns a locked ticket for its entire queued/active lifetime. The
transaction ledger admits up to the configured 1–8 active slots, rejecting a
different capacity while live tickets exist. FIFO applies within foreground and
background classes; with background waiting, at most three foreground grants
precede the oldest background ticket. Grants do not preempt active requests.
Discussion and planning are foreground; worker inference is background. Model
discovery remains outside admission. The original total deadline includes queueing.

Dropping a queued request withdraws eligibility before HTTP; owner death releases
client capacity. Unowned ledger entries never recreate request execution. A client
permit does not prove server residency, GPU headroom or cancellation of an already
dispatched HTTP request. Planner/task guards still run after capacity admission and
before HTTP, separately from scheduling authority. Workers recheck cancellation and
exact current Running task/run, policy and model after waiting; unrelated task-store
revision changes remain valid. Worker finalization aborts and joins the provider
future before publishing Finish, so its client queue ticket or permit has dropped.
This still does not prove that the model server stopped processing a disconnected request.

Changed coordinator observations reach Session, planner and worker projections as
transient queue metadata. Admission clears queued position and begins upstream
waiting. The client timing field labelled `queue` measures request start through
admission, including preparation and validation; it is not an isolated shared-slot
wait measurement. Optional server timing remains distinct from these client observations.
There is no canonical task, conversation or scope migration. Full Issue #69 profile,
digest, residency, token/context and mission-audit qualification, paired live latency
evidence and namespace retirement remain unfinished.

### Native diagnostic inference qualification

`--qualify-inference REPORT [--qualification-repetitions 1..3]` is a standalone,
opt-in cohort over isolated repositories/missions; `--inspect-qualification REPORT`
only validates and summarizes retained evidence. The cohort pairs baseline requests
with context-only candidates (foreground 8,192, background 16,384), using fixed
capacity one without changing ordinary capacity-two defaults or generation settings.
It runs four governed scenarios up to three repetitions (24 scenario executions),
caps generation dispatches at 128 and applies a 1,800-second cohort deadline plus
required cleanup. Runtime metadata GETs do not consume generation tickets or the
generation count. Existing live capacity conflicts still refuse.

`inference_profile` supplies immutable cloned settings and an optional shared bounded
request recorder. After shared admission and canonical-state validation, the provider
hashes and sends the same serialized body. Records distinguish absent wire options
from explicit values and retain ordered message byte lengths/content hashes, exact
serialized-prefix hashes and canonical profile identity without raw prompt text.
Profile identity includes endpoint, model, class/capacity, deadlines, format and every
current generation option. Prefix equality measures client bytes, not server cache hits.

For recorded calls, a terminal generation frame captures `generation_ms` before
read-only runtime inspection. `/api/version`, `/api/tags` and `/api/ps` have bounded
parallel reads using the same no-proxy/no-redirect client. The shared permit remains
held through inspection, and `Done` follows a finalized record. This prevents worker
shutdown or the next cooperating model request from cutting off the observation.
`runtime_probe_ms` isolates diagnostic overhead; `total_ms` includes it. Ordinary
unrecorded calls perform no probes and retain their existing completion behavior.
Interrupted futures finalize their record as interrupted with missing runtime proof;
a completed generation with failed inspection remains completed but non-qualifying.

The fixed independent oracle compares typed child-process results in its parent;
printing a success marker or exiting early cannot bypass those assertions. Fixture-v2
criteria carry the complete observable worker contract independently of generated
task titles. Accepted reports require all ordered planner/worker/discussion request
records and exact canonical review relationships. The
[verified diagnostic checkpoint](../Reports/2026-09-27-native-inference-qualification.json)
retains both the superseded fixture-contract observation and the corrected live run.
Only 1 of 8 corrected scenarios reached canonical acceptance; no profile is promoted.

Each scenario uses native planner/task/worker/check/review boundaries. Explicit fixture
review is confined to isolated missions; model output and ReviewReady cannot substitute
for canonical acceptance. Versioned reports bind manifests, fixture definitions,
executable/configuration identity, actual request observations and retained outcomes.
Artifacts remain beside the new report; checkpoints survive interruption without
inference replay. No conversation/task schema changes or profile promotion follow.

Runtime observations remain non-atomic and lack a verified upstream binary pin.
Templated `/api/chat` also lacks proven token headroom; the legacy raw-prompt byte bound
does not apply. Recorded hashes cannot reconstruct prompts or authenticate upstream
execution. Missing/drifting proof and failures remain visible, and end-to-end reviewed
latency includes diagnostic overhead. Integrated verification is pending; this slice
makes no model-quality or speed-improvement claim and does not complete Issue #70.
