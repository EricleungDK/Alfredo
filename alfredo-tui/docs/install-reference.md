# Alfredo terminal install reference (detailed)

This archive contains a native Rust terminal for Linux x86-64. It is a development
candidate with incomplete orchestration and release acceptance, not a production
release. The binary was built on the GNU/Linux host recorded in BUILD.json;
compatibility with older glibc versions is not established.

## Install

Verify the downloaded archive against its accompanying SHA-256 file, then extract
it. Checksums detect corruption; they are not publisher authentication.

```sh
sha256sum -c alfredo-tui-*.tar.gz.sha256
tar -xzf alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz
cd alfredo-tui-0.1.0-x86_64-unknown-linux-gnu
mkdir -p "$HOME/.local/bin"
install -m 755 alfredo-tui "$HOME/.local/bin/alfredo-tui"
export PATH="$HOME/.local/bin:$PATH"
alfredo-tui --version
alfredo-tui --help
```

Keep the previous binary separately before replacing it. Remove only the installed
binary to uninstall; runtime state is separate. Older binaries may reject newer
state schemas. Retain state backups before upgrading; do not delete state to force
a downgrade.

The current state formats are task schema16, conversation schema17, scope schema2,
selection journal schema1 and retained agent transcript schema1. Later sections record when individual
features first introduced their fields. Conversation schema15 added saved automatic
Architect drafts linked to their exact review command; generation remains separate
from `/plan-save` and approval. Schema16 adds saved Wayfinder scope actions bound to
the original user turn. Their exact intent saves before a scope change; confirmation
never approves or starts tasks. Schema17 adds exact workspace/mission selection and
destination arrival history, independent of task receipts. Older snapshots retain exact versioned backups on
upgrade. Restart keeps dispatch off and does not replay workers or planners.
`/plan-cancel` can withdraw an automatic draft still awaiting its intent save;
queued planners recheck their captured task revision and Architect origin before
sending the model request.

## Run

Open an interactive terminal:

```sh
alfredo-tui --model qwen3:14b
```

Run `alfredo-tui --doctor` to check storage, the restored model catalog entry and
Git/worker-tool prerequisites without a TTY. Exit 2 reports failed checks. It may
initialize private state directories/lock files but does not change task receipts
or saved conversations. It sends no inference and does not test sandbox execution.

## Optional inference diagnostics

The standalone qualification command runs real model requests and isolated coding
fixtures. It requires the same Git/Bubblewrap/prlimit worker prerequisites and a
selected installed model. Choose a new report filename in an existing directory:

```sh
alfredo-tui --model qwen3:14b --qualify-inference ./inference-check.json
alfredo-tui --inspect-qualification ./inference-check.json
```

Use `--qualification-repetitions 1` or `2` for a shorter diagnostic; the default is
three and the supported range is 1–3. Four scenarios cover small edits, required-source
multi-file work, repair and queued foreground work. Each runs with the unchanged
baseline and an experimental context profile: 8,192 tokens for foreground requests
and 16,384 for background workers. All experiments use one shared client slot,
independently of the normal default of two. An active terminal using a conflicting
capacity causes refusal; the experiment does not bypass the shared coordinator.
Omit workspace/mission and doctor flags; qualification creates its own fixture state.

The cohort permits at most 24 scenario executions and 128 generation HTTP requests,
with a 1,800-second cohort deadline followed by required cleanup. Bounded runtime
metadata reads are separate from the generation count. Existing generation deadlines,
thinking/output settings and production defaults remain unchanged. The command
retains workspaces, evidence and fixture state in `inference-check.json.artifacts`
beside the report. Existing report/artifact paths are refused. Incomplete checkpoints
remain inspectable and never resume inference; inspection sends no model request.
Completion of a cohort does not mean that every fixture succeeded.

Reports distinguish canonical reviewed outcomes from generated or merely checked
output. Each recorded generation probes bounded runtime metadata while retaining its
shared slot and before reporting completion. Generation time and inspection overhead
are separate; scenario end-to-end time includes diagnostic overhead. Missing metadata,
drifting identity, failed requests and incomplete cases remain visible. Retained
hashes identify observed wire bytes and settings, but cannot reconstruct prompts or
attest the upstream model/runtime. The upstream binary pin and exact token headroom
remain unverified. These diagnostics neither promote a profile nor establish a
general quality or speed improvement. Integrated verification is pending.

## Interactive runtime

The binary needs no checkout, Rust, Node, Python, Tauri or browser runtime. Ollama
must be available at http://127.0.0.1:11434 to send prompts; use --endpoint for
another origin and /models to discover installed models. A check you approve may
need its own interpreter. Coding workers require Git, /usr/bin/bwrap and
/usr/bin/prlimit on Linux and a repository with a commit. No packages are installed
automatically. Worker commands run in isolated worktrees with network disabled.

Terminals owned by the same user share model-request capacity for the same normalized
endpoint origin, independently of `--state-dir`. The default is two; configure 1–8
with `--parallel-models`. Conflicting capacities refuse while requests are active
or queued. Use the same endpoint spelling: host aliases are not resolved together.
Foreground discussion/planning and background worker requests are FIFO within their
class; a waiting background request gets a grant after at most three foreground
grants. Active requests are not preempted.

**Queued for Alfredo** means waiting for shared client capacity. Observed queue
position and active/configured slots describe Alfredo requests only. **Waiting for
model server** means admission completed; it does not establish server loading or
GPU state. Queue position can change. Cancelling a queued request prevents its HTTP
dispatch; releasing a running client slot does not prove server-side cancellation.
The ten-minute total deadline includes queue time, and model discovery bypasses it.
The client timing field labelled `queue` spans request start through admission,
including preparation and validation; it does not isolate shared-capacity wait time.
Workers recheck cancellation and their exact Running task/run, policy and model
after waiting. Unrelated task-store revisions do not invalidate that work. Before
recording Finish, the worker waits for its provider future to stop and release its
client ticket or permit; this does not prove server-side cancellation.

The private coordinator is at
`/tmp/alfredo-inference-<effective-user-id>/<sha256-origin>/`, independent of `TMPDIR`
and mission state. Live owner locks govern eligibility; stale records do not restart
inference. Do not remove or replace scheduler directories or `ledger.lock` while
Alfredo runs; the coordinator never unlinks that lock. Owner-proof checks do not
protect coordination against the same user replacing its namespace. Endpoint
directories remain retained; namespace retirement is unfinished. Queue telemetry and client timing are
transient, with no conversation/task schema migration. This does not qualify model
quality, server capacity or latency improvement.

F1 opens commands; F2 switches chat/tasks; Ctrl+N opens a conversation; Esc cancels;
Ctrl+Q quits. /task proposes work; /permit ID JSON specifies exact files and check
argv; /approve ID authorizes it; /run ID starts it. /evidence ID shows saved results;
/accept ID records review, retaining changes in the isolated worktree. /repair ID
reason proposes a repair requiring fresh approval. /after 1,2 description proposes
dependent work; accepted candidate commits are verified and composed into its
isolated baseline. Conflicts block dispatch. After acceptance, /branch ID creates
a local review branch from verified evidence and prints its git switch command,
leaving HEAD and working files unchanged. Remote push, automatic routing and
complete crash recovery remain unfinished.

F2 opens the Mission Work tree: recorded Plans and Manual tasks form groups,
with repairs beneath their original tasks. Dependencies are labelled edges, so a
task shared by several dependents appears once. Counts distinguish tasks from
local workers; recorded runs without current observations remain explicit.
Up/Down selects a task or group. Alt+Left collapses a branch or moves to its parent;
Alt+Right expands a branch or moves into its first child. Plain Left/Right edits
the prompt. Groups show counts and guidance and have no task action target; F3 and
commands without a task ID require a selected task row. Explicit task IDs still work.

`/tasks QUERY` filters task text, status, model and readiness while retaining
matching paths through Plan and repair ancestors. `/tasks #ID` selects that exact
task and `/tasks` clears the filter. Filtering never substitutes another task for
a hidden selection. The last selected task ID and filter restore after restart;
group focus and collapsed branches are local, so restart expands branches and
restores the task anchor. A missing or hidden saved task remains without a shorthand
target until a task is chosen. These view changes record no task actions.
At 32×10 the selected tree row, scrollable inspector and prompt remain reachable;
PageUp/PageDown pages details using the visible panel height, preserving one row
of overlap when possible; F2 returns to chat.

Runtime state defaults to $HOME/.local/state/alfredo. --state-dir selects another
location outside the coding repository. --workspace and --mission select the task
namespace. --conversation selects a named conversation set; transcripts and drafts
restore without replaying interrupted requests. Never treat an interrupted worker
as safely rerunnable without inspecting its evidence.

BUILD.json records the compiler, source fingerprints, dirty-worktree status and
payload checksums. Cargo.lock records dependency resolution. DEPENDENCIES.json
records the target-filtered resolved graph (including build/dev dependencies), locked
crate checksums and byte ranges/digests in THIRD_PARTY_NOTICES.txt. Notices are copied
from checksum-verified cached crate archives, including nested notices. Keep these
files with the candidate. This is not a linked-code inventory or a completed license
compatibility audit. Alfredo’s own code is MIT licensed; the accompanying LICENSE contains its terms.
Third-party components retain their respective terms.

Conversation snapshots now write version 2 and restore task/chat view, selection and
search for each named set. Version 1 is readable and retained as an exact-byte
`.v1-backup` sibling on the first upgraded save. Stop all terminals before rollback;
restoring that backup loses later conversation changes. Dispatch always starts off.

Startup asks for an exact existing Git repository root (Enter) or a new unused
repository path (F2, then Enter), followed by a mission name. The starting directory
is a suggestion. Repository and mission choices are collected without creating
either; Escape before final mission confirmation leaves no selection effects.
After exact saved admission, new repositories receive an empty initial commit,
without staging project files. This provides a baseline for planning and isolated workers; scope
and task approvals remain separate. Existing repositories are never auto-committed. Supply both `--workspace DIR --mission NAME`
to resume an existing selection directly, or use `--new-mission NAME` to create one. No inference or mission state opens before
selection. Failed creation retains partial files for inspection; it never overwrites
an existing target. Repository creation, mission preparation, target loading and
handoff are separate observations. A prepared handoff is not recorded as selected
until the actual switch. `--doctor` remains noninteractive with cwd/default fallbacks.

Selection requests and outcomes are retained at
`<state-dir>/rust-selection-v1/selections.json`, including failed startup without
an open conversation. The default state directory is `$HOME/.local/state/alfredo`.
Inspect the matching request, last observed outcome, repository path and mission
state before recovery. Created artifacts survive later failure; an incomplete
record may leave effects unconfirmed. Use an explicit Open and Resume after
inspection; use a fresh mission name for a new mission, never overwrite a retained
directory. Saved requests are never replayed automatically. Keep malformed journals
for inspection and restore a known-good state backup with terminals stopped instead
of deleting history to force a retry. In-process failure retains the source work;
the exact source and destination entries preserve drafts and reading positions.

The mission step lists saved names for the chosen repository. Tab fills a name;
Enter opens it. Discovery is bounded and advisory; omitted records can still be
opened by typing the name. Newly opened missions get an `identity.json` hint;
legacy task journals are also discoverable. Older conversation-only missions need
one manual named opening before they appear. A discovery warning does not discard
saved work or bypass its normal validation.

Open mode lists saved repository paths as well as the subsequent saved mission names.
Tab fills a path and Enter validates it; stale paths are never accepted automatically.
This lets you resume work from a different starting directory. Suggestions do not
create repositories, change branches or start agents, and are disabled in Create mode.

Mission selection now has separate Resume and Start New modes (F2 switches them).
Start New refuses existing mission/task/conversation data; Resume never silently
creates an empty mission. CLI --mission resumes, while --new-mission creates; passing
both is rejected. New identity is saved atomically in mission.json, without task or
scope approval. If creation acknowledgment is lost, resume the same name explicitly.
Legacy task/conversation state can still resume without rewriting it. Older versions
ignore mission.json and cannot enforce these admission rules; avoid mixing versions
while creating missions.

For the explicit native understanding flow, `/scope` inspects project-wide state;
`/scope JSON` drafts destination, scope, constraints and uncertainty after inspection.
`/scope-confirm REVISION` confirms the reviewed draft without approving or launching
work. Pending scope blocks new task planning/publication and run claims across
missions in the same workspace/runtime. Discussion, transient plan previews and
existing-work reconciliation remain available. Automatic first-contact routing and
planner brief binding are not implemented by this gate yet.

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

## Process-group cleanup identity

On Unix, cancellation cleanup retains an unreaped leader through the configured
SIGTERM grace interval. Its PID/start identity therefore remains verifiable before
forced group cleanup, even when the leader has exited and another group member
ignores SIGTERM. Existing identity/group checks and SIGTERM/SIGKILL paths are retained;
cleanup still requires a reaped leader and quiescent group before success. Failure to
prove cleanup remains outcome-unknown. This may use the full configured grace interval;
no timeout bounds or execution permissions are expanded.

## Change the active workspace or mission

Use `/workspace` to reopen repository and Resume/Start New Mission selection without
exiting. Esc returns to the current work. The header shows the active mission and
repository. Finish/cancel active conversations and coding workers, turn dispatch off,
and save or cancel the plan draft before switching. Pending operations must finish.

The destination loads and the current conversations save before the handoff. Opening
or saving errors preserve the current work. Each mission restores its own transcripts,
drafts, models and task-view preferences. Dispatch remains off after a handoff; old
events never replay into the selected mission. Choosing the same mission changes
nothing. Workspaces with active operations cannot currently run in the background
while another workspace is selected.

## Conversational Shared Understanding

New-project and consequential architectural requests enter durable Wayfinder Chart
mode. Explicit Wayfinder map/ticket/issue references enter Work-through. Read-only
questions use the normal discussion path, and existing scope continues across missions.
The same entry guard applies to new-project /plan, /task and /after shortcuts.

Provide four labeled lines (Destination, Scope, Constraints, Uncertainty) to save a
draft, review /scope, then send `confirm shared understanding N` for the exact draft
revision. Confirmation records agreement and ends the turn. It starts no model
request, task plan, delegation or worker. Use /plan separately afterward. Entry's
placeholder fields cannot be confirmed until replaced by a real scope draft.

Understanding schema v2 adds a receipt-validated flow and originating prompt in the
existing rust-understanding-v1 namespace. Before the first v1 mutation, the exact old
bytes are saved in understanding-v1-backup.json; backup conflicts refuse unchanged.
Older v1 readers reject the newer schema. Do not restore an old backup over newer
scope history as a rollback; use a compatible binary and inspect retained receipts.
At introduction, these scope changes left task v9 and conversation v2 unchanged. Desktop Wayfinder migration, complete
Mission formation and governed skill/graph execution remain unfinished.

## Response attribution and conversation v3

New responses display the requested model name or Wayfinder's application response,
with a scope receipt reference when acknowledged. These structured labels survive
restart and workspace switching. Model wording cannot create a Wayfinder label;
older messages without metadata display `Assistant · source unrecorded`.

Conversation schema v3 stores source metadata separately from provider role/content
messages. Before upgrading v1 or v2 history, the exact original file is retained in
its `.v1-backup` or `.v2-backup`. Conflicting backups and schema downgrades refuse
without replacing the current history. Attribution requires at least a v3-compatible reader; current saved histories
require v4 as described below.
Source references describe past responses and never authorize task execution or
replace current scope/evidence checks. At introduction, this change left scope v2 and task v9 unchanged.

## Reading streamed responses

PageUp holds your place in older output while the response continues. PageDown
advances toward the latest output and resumes following at the bottom. Conversation
switches and task/model panels retain the live reading point. New or retried turns
follow immediately. Newline-heavy bounded responses can display and navigate their
actual tail beyond the terminal widget's 65,535-row scroll range.

Conversation v4 retains logical reading anchors as described below. Exact
character-level anchoring through word reflow remains open.

## Saved reading position — conversation v4

Conversation v4 saves the logical line and wrapped-row position when you scroll
away from the latest output. Restarting, changing missions and receiving hidden
background output retain that anchor. Window resizing keeps the logical line and
clamps the wrapped row; it does not yet track an exact character through reflow.
PageDown to the bottom resumes following new output. Starting/retrying a turn also
returns to the latest output. Pending key navigation and live timing are not saved.

Versions 1–3 remain readable using their numeric bottom offsets until first render.
The first v4 save retains exact original bytes in a `.vN-backup` sibling for the old
version. Conflicting backups, malformed anchors and downgrades refuse unchanged.
Use a v4-compatible reader for newly saved histories. Restore an older backup only
with all terminals stopped; doing so loses subsequent history changes.

## Installed-model completion

At the end of `/model PREFIX` or `/assign ID PREFIX`, press Tab to choose from
the installed model catalog. An empty prefix lists available names. Up/Down or
Tab cycles; Enter fills the draft and a second Enter submits the command. Esc
closes the picker without changing the draft. The task ID is retained. Completion
never selects a model, records assignment or grants approval on its own.

Use `/models` to refresh an empty or outdated catalog. Completion performs no network
request. Existing model-selection and assignment checks still apply at submission,
including restrictions on active/interrupted work and fresh approval after assignment.
Move the cursor to the end before completion; arbitrary command arguments keep
normal conversation switching. Installed names do not establish model qualification.

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

## Recovery at recorded check boundaries

When a worker stops, inspect its task and use `/recover ID` explicitly. Recovery
requires the stopped worker's owner lock. Valid saved final evidence takes
precedence and retains its recorded outcome, even if a check checkpoint is damaged.
Existing malformed final evidence is preserved and blocks recovery.

Without final evidence, recovery can record **Failed** at either proven boundary:

- Before check launch: the version-1 `execution-boundary.json` matches the task,
  run and baseline, and both check intent and result are absent.
- After a recorded terminal check: the version-2 `check-launch-intent.json` binds
  the exact authorized request and canonical digest, Mission, task, run and
  baseline. Contract version 1 fixes the worktree, approved files/check argv,
  recorded system mounts, environment and resource policy. The version-1
  `check-result.json` binds the exact intent bytes and request digest to the
  provider receipt, including bounded output, byte counts, hashes and identities.

The worker creates each artifact exclusively and syncs the file and parent
directory. Intent publication finishes before check launch; result publication
finishes inside the execution closure before worker finalization continues.
Publication failure prevents a success claim. These immutable records live outside
the worker-mounted worktree. Digests detect corruption and substitution; they do
not authenticate private state against a same-user actor replacing all records.

After-check recovery retains the original check receipt and output and reports
**interrupted after check; candidate not finalized**. Even a zero-exit check yields
a Failed task: recovery reconstructs neither patch nor candidate. Original partial
work and artifacts remain intact. Repeating recovery returns the same Finish
acknowledgment without repeating an effect. `/repair` proposes separate work that
requires fresh approval; dependencies remain blocked on the failed original.

An old unbound check intent, missing or partial result after launch, unsupported or
corrupt artifacts, mismatched identities, and uncertain or reconciliation-required
receipts grant no after-check recovery. Existing runs gain no proof by default.
These separately versioned artifacts require no task, conversation or scope schema
migration. Recovery invokes no Git, inference or check, respawns no worker, and
does not signal surviving processes. Owner release and a terminal check receipt do
not establish quiescence of later worker helpers or authorize worktree reuse,
retirement or cleanup. Full automatic runner recovery remains separate work.


Task-panel navigation: PageUp/PageDown uses the current panel's visible height,
retaining one row of overlap when possible and advancing at least one row in tiny
panes. Paging clamps at the displayed content boundary, including after resize,
so narrow inspectors do not skip rows. Task details, scope, Activity, plan and
evidence panels share logical-line slicing so multi-line content can extend beyond
65,535 rows.
Activity and Models use the focused compact layout at narrow terminal widths,
keeping their content and the composer reachable at the 32×10 minimum size.


Evidence rendering retains one width-specific wrapping index for the immutable
verified view and copies only the visible logical lines on redraw. Resizing the
width or replacing the evidence rebuilds the index; height changes reuse it. This
cache is transient presentation state and adds no persistence schema or authority.


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
