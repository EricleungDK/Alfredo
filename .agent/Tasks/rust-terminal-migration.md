# Rust terminal migration

User authority: 2026-09-13 persistent goal requests a ralph-tui-like multi-agent
control terminal, a Rust rewrite, repaired disconnections/latency/layout, all-issue
regressions, and a polished product ready for GitHub launch.

The existing GitHub #10 desktop PRD and completed #56 modernization are prior
behavioral requirements, not proof of this new terminal product. Existing open
#11 distribution and #19 human accessibility acceptance remain relevant. GitHub
remains the issue tracker; this file is an implementation and evidence plan.

## Required delivery

1. Native Rust terminal: keyboard-first agent list, selected session transcript,
   composer, observable queued/loading/streaming/error/cancelled states, responsive
   resize and narrow-terminal behavior, bounded output and input, clean shutdown.
2. Direct asynchronous provider transport: connection/read deadlines, typed NDJSON
   parsing, partial-output preservation, explicit retry, cancellation, isolated
   concurrent sessions, model availability and actionable errors. Never replay
   a possibly effective tool call automatically after a connection loss.
3. Rust-owned durable orchestration: workspace/mission/task identities, dependency
   graph, model/role assignment, revision and correlation guards, approvals,
   worktree isolation, tool execution, evidence/review/repair, recovery, retirement.
   Port existing contracts with parity tests before replacing their authority.
4. Production terminal workflow: prompt-to-task delegation, multiple coding agents,
   live tool output and diffs, approval/review controls, command discovery, complete
   restart continuity, accessible palette and keyboard navigation.
5. Full regression inventory mapped to authoritative GitHub issue requirements;
   run legacy Python/frontend/Rust gates and new terminal/provider/runtime tests.
   Preserve failure signatures and fix product defects without weakening guards.
6. Real-model and PTY acceptance: delayed/disconnected stream, parallel agents,
   cancellation, restart, small terminals, Unicode/paste and sustained output;
   record startup, input response, first-token and completion timings separately.
7. Launch: reproducible locked release build, supported OS matrix, binary install
   smoke, CI gates, migration/rollback docs, security/dependency review, changelog,
   honest limitations and human visual acceptance. Public publishing/visibility
   follows review of the concrete verified candidate.

## Current slice

`alfredo-tui/` now contains concurrent conversations and a durable coding task
workflow: explicit file/check policy, fresh approval, durable run claim, detached
Git worktree, structured model file plan, shared Rust Bubblewrap execution, and
verified evidence/review receipts. Schema-v1/v2/v3/v4 migration preserves original bytes
and never upgrades scheduling approval into execution rights. Accepted patches
remain isolated. Dependent execution now composes verified accepted candidates
into its isolated baseline and refuses conflicts before dispatch. This does not replace all requirements in items 3–7.

Noninteractive `--doctor` reports storage, restored-model catalog and Git/worker-tool
prerequisites with actionable failures, without inference or task execution.

A canonical Mission Work tree groups Plans, manual work and repair ancestry while
keeping dependency edges separate. Exact task selection, group-safe shorthand and
F3 evidence access support keyboard control. Evidence review shows
readable check results, colored unified diffs and saved output; opening evidence
selects its task so review shorthand targets the inspected work. The prompt supports
grapheme-aware cursor editing, word deletion, bounded prompt-history traversal,
dismissible slash-command completion and a cell-measured cursor viewport.
`/models` discovers installed models asynchronously; `/model NAME` explicitly
selects a conversation model while active/interrupted turns and existing tasks
retain their assignments. Live local worker stages,
elapsed timers, bounded live check stdout/stderr, byte counts and first-content latency
are visible; completion still requires durable receipts. Recovery now acknowledges
complete retained results after OS owner-lock release, without repeating effects.
Missing/invalid evidence and legacy owners remain uncertain. Next: uncertain-child
reconciliation and governed retry; then active-branch merge integration,
automatic repair routing, complete workspace/view continuity, model capacity/assignment and retirement. Linux
coding is verified. Native Linux x86-64 development archive packaging and installed
PTY acceptance now exist; cross-platform qualification and production publication
remain open.

Chats, planners and coding workers now share same-user, cross-process admission
for a canonical endpoint, with default capacity two and `--parallel-models N` from
1 to 8. Foreground work has bounded priority; cancellation removes queued
eligibility. Model discovery bypasses admission. Queue, first-content and client
completion timings remain distinct. This controls Alfredo client slots and does
not prove GPU capacity or upstream cancellation. See the
[shared-admission evidence](../Reports/2026-09-26-shared-inference-admission.json).
The [bounded qualification harness](../Reports/2026-09-27-native-inference-qualification.json)
measures paired native scenarios; its corrected live cohort accepted only one of
eight cases, so no production profile or quality claim is inferred.

Use a bounded channel between asynchronous inference and synchronous terminal
rendering. Every event carries session and attempt identity; cancel/retry isolates
late events. Errors preserve partial content and never report completion without
the provider's terminal marker. Tests use real local HTTP servers and Ratatui's
buffer backend; live Ollama inference requires a separately observed run.

`/repair ID reason` proposes a linked repair for a failed, rejected or cancelled
run with verified retained evidence. It inherits the parent model, dependencies and
file/check policy, requires fresh `/approve NEW_ID`, and then `/run NEW_ID` starts
a new isolated worker. One unresolved repair per parent is allowed; exact request
replay returns the same child. The worker uses the parent's committed baseline and
includes the original task, outcome, patch and check receipt as bounded reference
data (128 KiB maximum evidence). It generates complete corrected files; the prior
patch is not executed or assumed applied. Original runs remain unchanged. Missing,
tampered or oversized evidence blocks the repair workflow; uncertain runs need
reconciliation first. Automatic repair routing and escalation remain unfinished.

Successful worker results now retain a Git candidate commit for the exact reviewed
diff, parented by the recorded baseline. A `refs/alfredo/candidates/<commit>` ref
keeps the object reachable; the workspace branch, HEAD and working files stay
unchanged. Untracked check/build output is excluded. Evidence stores the optional
commit ID under its existing digest. Verification checks its sole parent and exact
binary diff against that evidence, without relying on mutable retained worktree
files. Older evidence remains readable with no inferred candidate. Snapshot
failure prevents a new result from becoming review-ready. Candidate refs are
retained; retirement and storage budgeting for them are not implemented yet.

For `/after 1,2 description`, each parent must be Accepted and retain a candidate
commit that verifies against its saved evidence. Before dispatch, the worker
combines those commits with the current committed workspace baseline using Git's
object-only merge-tree operation. Ancestor/diamond inputs are reused, not applied
twice. Conflicts, missing/tampered/legacy candidates and unaccepted parents leave
the child unstarted. Custom merge configuration requires qualification and is
refused before merge execution. Git must support `merge-tree --write-tree`.

The composed baseline is pinned under `refs/alfredo/bases/<commit>`. Preparation
can create immutable Git objects and managed refs, but moves no branch or working
files and starts no model/check. It has a 60-second deadline and observes worker
cancellation. The subsequent run claim records each exact parent task/run,
evidence digest and candidate ID; the store rechecks input identities and evidence
at that transaction. The child's isolated worktree starts from the composed
baseline, and its reviewed diff contains only its changes relative to that baseline.
The review view lists accepted input tasks. Publishing these changes to the user's
branch, automatic conflict repair, and managed-object retirement remain separate
unfinished workflows.


After accepting a result, `/branch ID` (or `/branch` for the selected task) creates
its local review branch, named `alfredo/task-ID-COMMITPREFIX`. The candidate must
still verify against saved evidence. The command creates the ref only when absent,
refuses a different existing target or symbolic ref, and records the verified
branch/commit in task activity. Repeating it verifies the same ref and avoids a
duplicate receipt, including reconciliation after Git succeeded but storage failed.
It leaves HEAD, the index and dirty working files unchanged. The result provides a
`git switch` command for deliberate checkout; no remote push or PR is performed.
Recorded branch links are historical; `/branch` rechecks the live target before
confirming it. Normal Git review/merge/push remains a separate user action.

## All-issue coverage

The [regression inventory](rust-terminal-regression-inventory.md) maps 81 live
GitHub issues, 297 extracted source acceptance criteria and six native parent/child
chains to related legacy/Rust tests and explicit terminal gaps. It does not infer
parity from closed issue state or aggregate counts. Durable named conversation snapshots now restore transcripts/drafts/models without
replaying saved active requests. Complete workspace/mission/view continuity remains
incomplete, followed by uncertain-effect recovery and automatic repair routing,
active-branch merge integration, retirement, and qualified inference scheduling.

## Evidence

Conversation, durable tasks and isolated coding workers are implemented. The current
terminal suite passes 86 deterministic tests (three live/subprocess fixtures ignored),
formatting, strict Clippy, release build and debug/release PTY coding journeys.
A real qwen2.5-coder:14b two-file edit passed its sandboxed check in 2.17 s, one warm
observation. Shared execution compatibility passes legacy Rust (65 passed, one
ignored) and focused Python execution (83 ran, one skip). Earlier full Python
(797 tests, three skips) passed. Frontend completed 324 passed / one retirement
inspection failure (`/proc/372/cwd`, EACCES); TypeScript passed. See the
[implementation report](../Reports/2026-09-13-rust-terminal-foundation.md) for
precise evidence and limitations. No launch or speed improvement is established.

Task journal admission reserves receipt slots and worst-case serialized Finish
space for Running workers under the storage lock. Boundary regressions cover
receipt exhaustion, escaped JSON byte growth and refused new claims. Existing
hard bounds and schema v5 remain; archival and already-overcommitted journal
recovery remain open.

Branch handoff preflights journal capacity and request identity before creating a
Git ref. Existing recorded handoffs remain replayable at capacity. Concurrent writes
can still change available space after this read-only check; the final storage
transaction and exact-ref reconciliation remain necessary.

Prompt-to-plan checkpoint: `/plan REQUEST` generates a strict 1–16-task draft using
the selected model as Frontier Architect and Local Agent worker model. `/plan`
reopens it; `/plan-cancel` aborts without task mutation. `/plan-save` records the
reviewed batch atomically as Proposed via schema-v6 Plan receipt; original prompt,
planner, policies and rebased dependency IDs persist. Approval and launch remain
explicit separate actions. Invalid/incomplete/cancelled outputs never save, and
stale revisions preserve the draft without partial task creation. Drafts are
transient; repository-aware planning, model-role overrides, Shared Understanding /
Plan Grill gates and automatic graph dispatch remain required for full delivery.

Worker reassignment checkpoint: `/assign ID MODEL` uses installed-model catalog
admission and the common task transaction to replace an unstarted worker model,
preserve policy/dependencies and reset approval. Started tasks are immutable. Exact
acknowledgment replays offline, stale lookups refuse, and generic retry cannot bypass
catalog refusal. Schema v7 retains assignment receipts with v1–v6 backup migrations.
The selected conversation model/Frontier Architect and qualified role profiles remain
separate; profile qualification and registry-driven role selection are still open.

Repository planning checkpoint: /plan reads a bounded pinned Git context and retains
its exact selected source inputs in schema-v8 Plan receipts. Working edits are
excluded, omissions are explicit, and capture failure stops inference. Workers
refuse grounded tasks after committed baseline changes. Earlier plans gain no
inferred context. This replaces the initial repository-unaware draft path; dynamic
retrieval, comprehensive instruction coverage and uncommitted/new-repository planning
remain open alongside qualified roles and automatic dispatch.

Dispatch checkpoint: explicit /dispatch on|off now admits ready Approved tasks through
the common worker path, after Accepted dependencies, with the existing four-worker
limit and shared model queue. Per-process approval-attempt memory prevents automatic
retry of failed/uncertain starts. Shutdown/restart defaults off; no durable approval,
review or recovery is inferred. Cross-process dispatch leases and durable scheduler
intent remain open.

Task-supervision checkpoint: bounded /tasks search and exact #ID navigation constrain
selection/shorthand actions to visible rows, with safe empty results and exact
hidden-task evidence targeting. Readiness details name policy/approval/dependency
gates; dispatch remains global. Search preference is transient. Native Issue Graph
hierarchy, complete view continuity and human accessibility acceptance remain open.

Timing checkpoint: optional bounded Ollama final-frame metrics now distinguish server
load, prompt evaluation and generation in conversations, plan previews and retained
worker evidence. A live qwen2.5-coder:14b READY request completed in 18.948 seconds
(load 18.71 seconds); its repeat completed in 0.438 seconds (load 0.14 seconds).
These two small observations identify loading as the initial delay, without proving
sustained coding throughput or changing residency/qualified-profile policy.

View-continuity checkpoint: conversation-v2 snapshots restore task/chat mode, task
search and selected ID within each workspace/mission/named conversation set. V1
migration retains exact original bytes and refuses conflicting backups. Selection
resolves only through current visible tasks; no overlays, dispatch or effects replay.
Interactive workspace/mission selection and full planning-draft continuity remain open.

Startup-selection checkpoint: cwd is now an editable Starting Location, with exact
repository-root validation or explicit unused-path Git creation before mission
selection. Explicit workspace/mission flags use the same validation; doctor stays
noninteractive. No task/conversation state or model inference opens before selection.
New repositories need an initial commit for coding. Recent-work discovery, switching
within a running terminal, versioned Workspace Session acknowledgments and full
mission formation remain open.

Creation-preflight correction: a failing fixture proved that runtime aliases could
hide overlap until after Git init. Existing-ancestor resolution now rejects that
case before effects and refuses unresolved runtime ancestry. Actual PTY acceptance
also exercises Create mode, existing-target refusal, empty Git initialization and
mission entry without modifying the prior workspace's task journal.

Saved-mission discovery checkpoint: startup asynchronously lists matching names from
advisory namespace identities or legacy task journals. Tab fills a name and Enter
selects it; corruption/limits are visible and manual selection remains. New
conversation-only missions register identity hints; older ones require one manual
opening. This is name discovery, not a recent-work ranking, workspace-session receipt,
or completed mission-formation workflow.

Recovery ownership correction: a deterministic fork fixture reproduced a released
worker claim still reported active through an inherited descriptor. Claims and
recovery probes now use scoped explicit unlock, sharing the journal-lock primitive.
The test holds the child descriptor open across recovery and preserves live-owner
refusal plus unknown-outcome behavior. This confirms a lifetime defect, while the
original intermittent assertion's exact error remains unavailable.

Repository-discovery checkpoint: saved paths now appear in Open mode before mission
selection, sharing bounded name discovery. Tab fills only; Enter revalidates the
exact root and refreshes matching mission names. Paths deduplicate across missions,
with manual fallback and stale-path rejection. Full mission formation and switching
workspaces while workers are running remain open.

Mission-identity checkpoint: explicit Resume versus Start New now precedes formation.
Immutable mission.json records admission under the namespace lock, prevents duplicate
creation and preserves legacy-state resume. CLI --mission resumes; --new-mission
creates. Identity admission grants no scope approval. Conversational Mission Draft,
Shared Understanding, project-level gates and their complete receipt chain remain open.

Explicit-understanding checkpoint: /scope stores destination, scope, constraints and
uncertainty in one project-wide receipt journal; /scope-confirm requires the exact
pending draft. Pending gates new task writes and run claims across missions while
preserving read-only/existing-work reconciliation. Confirmation invokes no next step.
Automatic Chart/Work-through entry, planner brief/provenance binding, complete mission
formation and gate-aware old-client migration remain open.

Scope-readiness follow-up: task views now observe the project gate during foreground
and background refresh, explain pending/unavailable scope and include it in search.
Observed blocked scope turns dispatch off without consuming a task start attempt;
confirmation does not resume dispatch. Actual transactions retain authoritative gate
checks. Cross-mission confirmation/corruption and rendered blocker coverage added.

Plan-scope checkpoint: planner requests now include a captured workspace scope brief;
Plan receipts preserve that binding in task schema v9. Save and planned worker admission
refuse a different current scope revision. Legacy schemas v1–v8 remain readable and
back up exact bytes before mutation; no scope authority is inferred. Automatic
first-contact routing and complete manual/repair formation provenance remain open.

Worker scope handoff: the actual coding request now contains the acknowledged Plan's
scope binding, alongside approved source context. A real HTTP regression first proved
the binding absent, then verifies the exact serialized binding, unchanged policy,
passing isolated edits and unchanged source workspace. No schema migration added.

Client-timing checkpoint: native conversations separate observed admission wait,
first nonempty text, streaming and total intervals. Terminal outcomes freeze timing;
retry resets it and restart does not restore monotonic clocks. This improves latency
visibility without asserting a model speedup. Controlled sustained performance cohorts
and rendered-action latency qualification remain open.

Cleanup checkpoint: the intermittent cancellation failure reproduced under concurrent
library tests and a controlled leader-exit/surviving-member fixture. Unix grace waiting
now retains leader identity until existing forced cleanup checks complete. The fixture
proves no helper cleanup is needed, and mismatched live identity still refuses signals.
100 concurrent library-suite attempts and shared compatibility tests pass.

Thinking-progress checkpoint: local real-model checks passed, with cold request timing
dominated by model loading. Native transport now recognizes thinking-only frames and
emits bounded, payload-free progress to conversation/planner/worker views. Reasoning
bytes count toward the existing output budget and never become answer text. Repeated
observations and deterministic verification follow; no model speedup is inferred.

Stale-plan readiness checkpoint: observed scope revision changes now produce a separate
selected-task blocker and searchable explanation for unstarted planned tasks, including
legacy plans without a scope binding once explicit scope begins. Manual start refuses
before recording an approval attempt; dispatch skips those tasks and can choose a later
eligible plan. The journal still revalidates the full binding under its lock; UI
observation does not grant authority or rewrite history.

Workspace handoff checkpoint: `/workspace` switches repositories/missions within the
running terminal, with cancelled/failed selection retaining current work. Target
loading plus ordered source save precede replacement, old owners release, fresh
event channels isolate restored IDs, and dispatch stays off. Active operations and
unsaved plan drafts must be resolved first. The header shows mission/workspace.
Unit regressions cover independent restored state, task-history preservation, owner
contention, final-save failure and quiescence. Native 146 passed / three optional
ignored, strict Clippy and release pass. Expanded PTY: two passed in 5.939 s,
including cancellation and switching repositories/missions in the same process.

Wayfinder first-contact checkpoint: native conversation routing now records one
Chart/Work-through flow per workspace and continues it across missions/restarts.
Read-only entry exclusions match the legacy vocabulary. Explicit new-project task/plan
commands also route through entry; four labeled scope fields create a reviewed draft
and exact-revision Commander confirmation ends the turn without inference/task effects.
Understanding v2 retains entry actor/prompt with exact v1 backups; pending gates and
Plan bindings remain authoritative. Routing receipts finish independently of cancelled
model replies, and switch/normal quit wait for them. Focused migration/concurrency,
cancellation and historical-confirmation tests pass. Long-history regression also
drove pinning client/server timing above the transcript. Native 152 passed / three
optional ignored; strict Clippy and release pass. Expanded PTY: two passed, 6.153 s.
Automatic governed graph/skill execution, structured per-message capability attribution,
complete Mission Draft receipts and desktop/native migration remain open.

Response-attribution checkpoint: conversation v3 records per-assistant-message source
metadata (requested model or Wayfinder with optional scope receipt reference). The
wire payload remains role/content only; raw model wording cannot label itself as an
application receipt. Legacy authorship stays unrecorded, old v1/v2 bytes are backed up
exactly, invalid source data/downgrades refuse, and attempt-bound retry clears only
the replaced reply's source. Native 155 passed / three optional ignored, strict
Clippy/release pass; release PTY two passed in 6.139 s. Complete cross-capability Mission/Workspace action chronology
and profile/digest qualification remain open.

Reading-continuity checkpoint: per-session logical-line anchors preserve old text
while a reply streams, and PageDown returns toward follow-latest. Hidden task/model
views no longer change or paint the conversation viewport. Slicing logical lines
before widget scrolling fixes the 65,535-row tail limit. Focused regressions cover
stream growth, compact resize, session switches, returning to live output and a
66,000-line response. Numeric saved offsets/schema v3 remain; exact restart/reflow
anchor persistence is still open. Native 157 passed / three optional ignored, strict
Clippy/release pass; controlled-stream PTY two passed in 6.298 s.

Saved-anchor checkpoint: conversation v4 preserves a logical reading position across
hidden background output, restart and different terminal height. Wrapped rows clamp
on width changes; exact character reflow is still open. V1/v2/v3 upgrade with exact
backups and old offset fallback. Native 159 passed / three optional ignored; strict
Clippy, release and installed PTY (two tests, 6.226 s) pass. User chose MIT; root
LICENSE, crate metadata and seven-member archive reflect it. Nine notice integrity
tests pass. Dependency compatibility/security and full launch gates remain open.

Dependency security checkpoint (2026-09-14): fresh RustSec audit found newly published
RUSTSEC-2026-0285 in rustls 0.23.44. Upgraded only that locked package to 0.23.45;
all 213 locked packages now report no vulnerabilities/warnings. CI uses pinned
cargo-audit 0.22.2, no suppressions, fails on warnings and verifies rejection of a
known-vulnerable fixture. Native 159 passed / three optional ignored, strict Clippy,
nine packaging fixtures and installed PTY acceptance pass. Evidence is in
[the audit report](../Reports/2026-09-14-native-dependency-audit.json). Application
security, license compatibility and full product/launch qualification remain open.

Model-argument completion checkpoint (2026-09-14): Tab after /model PREFIX or
/assign ID PREFIX opens filtered, sorted, deduplicated installed names; Enter
only fills the draft. Missing names/catalog produce a notice without switching
sessions or fetching. Mid-draft completion refuses replacement. Existing submit
validation still protects active/interrupted model selection and resets approval
on reassignment. Native 160 passed / three optional ignored; strict Clippy and
release pass. Installed PTY acceptance passes, including no model selection or
assignment receipt until the completed draft is submitted. Unicode, invalid IDs,
empty catalogs, no-match and cycling are covered. Full capability routing and
model qualification remain open. Candidate /tmp/alfredo-completion-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 c7b76388e08eb46aa6e257fb9e01db00c1bd8eb54a1171e09ca5cc123b66a932.

Live worker acceptance checkpoint (2026-09-14): added an explicitly ignored test
for two independent real coding workers, checking strict port parsing and merging
3,375 interval combinations without input mutation. Both share the actual provider
capacity and must retain source workspaces while producing review-ready commits.
Fixture commands were encoded as single-line Python invocations to satisfy the
existing policy, without weakening checks. Both outcomes are collected before failure.

The live qwen3:14b trials did not qualify: with two slots, merge_intervals hit the
60-second loading deadline. One strict_port sample passed in 84.963 seconds, first
content 81.471 seconds, model load 0.154 seconds. These observations are consistent
with queueing, but do not by themselves prove its cause. During the one-slot control,
systemd stopped Ollama and upgraded 0.30.6 to 0.34.0 at 22:03:23 +02:00. The resulting
connection loss makes that control inconclusive. Logs confirm server model capacity
one; external model download activity followed. No server settings, client defaults,
model selection or timeouts were changed by this work. Repeat on a stable recorded
server before changing admission behavior.

Deterministic worker regression: 18 passed, two optional ignored; strict Clippy passes.
The new live test remains explicitly ignored, with failed live evidence preserved
in [the acceptance report](../Reports/2026-09-14-live-worker-acceptance.json). No new
runtime artifact is needed for this test-only change. Full performance/reliability
and product/launch qualification remain open.

Generation-limit checkpoint (2026-09-14): paired capacity-two and capacity-one live
tests on unchanged Ollama 0.34.0 PID 26146 both failed. Capacity two had a loading
timeout plus EOF without completion; capacity one had two EOF-without-completion
failures. Serial admission therefore did not validate a fix. External model download
activity was recorded. Do not apply the prepared default-admission experiment.

A tiny metadata-only structured-output probe returned done_reason=length after
128 generated tokens with default thinking; think=false returned stop. This does
not establish a cause for the larger EOF failures, nor qualify a thinking policy.
It did expose ignored generation-limit metadata. The Rust provider now retains
partial text and bounded metrics but reports Failed for length exhaustion. Normal
stop and reason-less legacy completion remain. No defaults, budgets or server
settings changed. Tests cover text, syntactically valid JSON and thinking-only
truncation, split frames and normal completion. Installed PTY verifies partial text
and Failed persist through save/restore. Native 161 passed / four optional ignored;
strict Clippy and installed acceptance pass. Candidate /tmp/alfredo-length-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 e81b30c90bb34327741e6d3c5752f03fffaa29e42473cc75301bb5cc48c2b44f.

[Full paired results and frame metadata](../Reports/2026-09-14-admission-and-generation-limits.json)
retain the failures. Larger structured-response EOF diagnosis, model quality and
latency, and full product/launch qualification remain open.

Structured-response workaround checkpoint (2026-09-14): a loopback relay forwarded
one original strict-port request unchanged. Ollama 0.34.0 returned 1,948 thinking-only
frames, 7,144 thinking bytes, no content bytes, no completion frame and normal HTTP
EOF after 55.240 s. Metadata only was retained; reasoning text was not saved. Thus
Alfredo's EOF rejection was correct. Repeating the original independent checks with
only think=false changed yielded two review-ready candidates.

Schema-constrained planner/worker calls now request think=false by default. Explicit
--structured-thinking auto/on/off selects server default/true/false for these calls;
ordinary chat omits the field. Configuration is invocation-local and follows provider
clones/workspace handoffs. No deadlines, generation budget, admission limit, task
permissions, server settings or automatic retries changed. Live-test OLLAMA_HOST and
ALFREDO_SMOKE_CASE selectors support isolated diagnostics without altering production
endpoints. Wire regressions cover default/off/on/auto behavior and plain chat; installed
checks validate CLI values and actual planner/worker request fields.

Direct live acceptance without the relay and with normal capacity two passed both
cases: intervals 4.270 s, port 6.753 s. Both produced candidate commits and left source
workspaces unchanged. This is limited synthetic evidence, not general model quality
or sustained workload qualification. Native 162 passed / four optional ignored, strict
Clippy, release and installed PTY acceptance pass. Candidate /tmp/alfredo-thinking-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 06ab0ecef3298b9ae7bec82608b29102f80c2fd3c018337d7dbc7431a93cbe87.
See [metadata and exact outcomes](../Reports/2026-09-14-structured-thinking-workaround.json).
Full model profiles/qualification, formation/chronology/retirement and launch remain open.

Generation provenance and repeated-worker checkpoint (2026-09-14): worker evidence
now records requested thinking mode, token limit and temperature at request preparation.
Verified review displays the settings; missing legacy records remain unrecorded.
Malformed settings are rejected, and changing settings in saved evidence fails the
existing digest check. This does not prove dispatch or server compliance and adds
no task authority or snapshot version change.

Native 163 passed / four optional ignored; the subsequent explicit-on/tamper worker
regression, strict Clippy, release packaging and installed PTY passed (two tests,
7.530 s). All 35 packaged source hashes match the current checkout. Candidate:
/tmp/alfredo-generation-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz;
SHA-256 acc65ccc44196d23cadf5062adbe199a3dac4f02c8df2990e389313a1210c2cb.

Five sequential pairs of live qwen3:14b coding tasks passed all independent checks
and produced review-ready candidates without changing source workspaces. Server
version and model digest match before/after. First-pair completions took 12.100 and
16.370 s, with approximately 9.25 s model loading. Warm port tasks took 2.690–2.735 s;
warm interval tasks took 6.544–6.587 s, including upstream queueing. Ten small tasks
are repeated-workload evidence, not broad role qualification or a long-duration soak.
See [exact identities, samples, source hashes and verification logs](../Reports/2026-09-14-generation-and-repeated-workers.json).
Full formation/chronology/retirement, broader model workloads, cross-platform and
launch acceptance remain open.

Plan revision checkpoint (2026-09-15): `/plan-revise REQUEST` sends the previous
unsaved task list, accumulated user requests and freshly captured committed context
and scope to the original planner model. A validated replacement still requires
explicit `/plan-save`, then separate task approval. Failed/incomplete revisions
restore the old draft and original revision; stale saves remain rejected. Cancel
removes both drafts and ignores late output. Invalid requests preserve the draft
without dispatch. Reference tasks are bounded to 64 KiB and prompt history to 8 KiB.

Four added regressions cover actual HTTP context, selected-model stability, explicit
save/approval, malformed/incomplete output, stale restoration, cancellation and
invalid requests. The installed PTY now revises before saving, verifies no earlier
task mutation and checks the saved replacement. Restored the ordinary review notice
that the existing terminal regression expected; revised completion is distinct.
Native 167 passed / 4 optional ignored, strict Clippy and installed
archive acceptance pass. Candidate /tmp/alfredo-plan-revise-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 65baef3807452cbb9a5a67023b8f9d12055d9966ed14b969093a54746d35e5d8.
All packaged source hashes match the current checkout. Exact logs are in the
[verification report](../Reports/2026-09-15-plan-revision.json).

The explicit user continuation resolved the automatic approval rejection. Draft
restart persistence and full Mission Draft/Issue Graph formation remain open;
this refinement workflow does not imply full issue or launch completion.

Plan continuity checkpoint (2026-09-15): conversation v5 persists complete review
plans with their original task revision. Checkpoints during refinement retain the
previous complete plan; no partial first draft or inference is resumed. Quiescent
workspace/mission handoff preserves valid drafts and restores their review view.
Malformed or oversized plans and old-version plan fields are rejected. V1–v4 remain
readable and exact-byte backups precede the first v5 write; conflicts/downgrades
refuse unchanged. Task v9 and scope v2 are unchanged.

Migration tests prove v4 backup conflict handling, exact preservation and malformed
payload refusal. Workstation tests prove restart, workspace isolation, switch-back,
stale-save refusal and cancellation removal. The installed PTY exits with a revised
plan, restores it without another model request, and explicitly saves the replacement.
Native 169 passed / 4 optional ignored, strict Clippy and installed
archive acceptance pass. Candidate /tmp/alfredo-plan-persistence-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 434f45a978b9336bb88dea428b71b34082225bd98491c9f8a04d9a44a1ee4660.
All packaged source hashes match the current checkout. See the
[verification report](../Reports/2026-09-15-plan-continuity.json).
Full mission formation, chronology, recovery/retirement and launch remain open.

Capability discovery checkpoint (2026-09-15): `@wayfinder REQUEST` addresses the
native scope adapter. Tab/arrow/Enter completion fills the draft without submitting;
Escape closes the picker and F1 lists the capability alongside commands. Unknown
leading capability names and empty requests fail without model dispatch or draft
loss. Explicit discussion can enter Chart/Work-through, and prefixed four-field
briefs/confirmation use the existing exact receipts. No task approval is inferred.

New tests exposed a real 32×10 layout failure: the session list consumed the entire
content area. Completion now gets that area while open and hidden conversation
rendering cannot update its reading anchor. The original narrow assertion passes.
Installed PTY verifies unknown-name refusal, completion/dismissal/no early action,
explicit scope entry, brief and confirmation without model inference or task effects.
Native 171 passed / 4 ignored; strict Clippy and installed archive
acceptance pass. Candidate /tmp/alfredo-wayfinder-capability-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 397d8f92ae790c9c7e31b82af4add123679e4e978b0742aee9864d6baa261d03.
All packaged sources match the checkout; [exact evidence](../Reports/2026-09-15-wayfinder-capability.json).
Only the existing Wayfinder adapter is exposed; general skill routing, full mission
formation/action chronology and remaining production launch requirements stay open.

New-workspace baseline checkpoint (2026-09-15): explicit Create repository now
initializes an empty root commit after sanitized Git init. The selector discloses
this before creation. Git mktree uses null stdin, commit-tree uses the Alfredo
identity with signing disabled, and update-ref publishes main only if absent.
No project files or index are staged; existing repositories are never auto-committed.
Partial creation remains inspectable and never reports successful acknowledgement.

Selection tests verify a root commit with empty context, no source files, no mission
state and preservation of existing unborn repositories and their untracked files.
A real HTTP worker test creates the first files in an isolated worktree, passes
its approved check and retains a review-ready candidate while source HEAD/tree
remain empty and unchanged. The first fixture expected old source text; corrected
to require explicit new-file context without weakening existing-source tests.
Installed PTY verifies disclosure before creation and the actual empty Git tree.
Native 173 passed / 4 ignored; strict Clippy and installed archive
acceptance pass. Candidate /tmp/alfredo-new-workspace-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 6d2957e9dfda76066f2d4689a3e7cbfaf39cc029ec7670284e92ea23d8c87fd0.
All packaged sources match; [verification report](../Reports/2026-09-15-new-workspace-baseline.json).
Uncommitted context import, full workspace receipts/mission formation and remaining
production launch acceptance stay open.


Recovery update (2026-09-20): synced task/run/baseline markers allow explicit
recovery of a stopped worker as Failed only when no check-launch intent or final
evidence exists. Real subprocess crash and refusal/idempotency/repair tests pass;
native suite 176 passed / 5 ignored, strict Clippy and installed PTY pass. Partial
work remains retained. Possible check launch remains unknown without ordinary
terminal evidence; this does not prove helper quiescence or authorize retirement.
[Evidence](../Reports/2026-09-20-precheck-recovery.json).


Terminal panels update (2026-09-20): Models and Activity content remain reachable
at 32×10; shared task-panel logical-line slicing reaches evidence past 65,535
rows. Page navigation clamps after end/resize. Native 178 passed / 5 ignored,
strict Clippy and installed PTY pass. This advances terminal usability coverage;
full chronology and release acceptance remain open.
[Evidence](../Reports/2026-09-20-terminal-panels.json).


Evidence-render update (2026-09-20): cached wrapping of immutable evidence and
visible-page cloning reduce the optimized synthetic 100-redraw workload from
539.962 ms to 17.425 ms. Unicode/style/resize buffer equivalence and work-count
regressions pass; native 180 passed / 6 ignored, strict Clippy and installed
PTY pass. This does not qualify model or end-to-end latency.
[Evidence](../Reports/2026-09-20-evidence-rendering.json).


Acceptance-contract update (2026-09-20): #4 native planned task packets now retain
bounded explicit acceptance criteria through review, save, restart, worker requests
and separately approved repair descendants. Task v10/conversation v6 migrations
retain exact originals; absent legacy criteria stay unrecorded. Fixed criteria
displacing live progress and discovery omitting the new schema. Native 183 passed
/ 6 ignored, strict Clippy and installed PTY pass. Full mission formation,
criterion-level review and chronology remain open.
[Evidence](../Reports/2026-09-20-plan-acceptance.json).


Criterion review update (2026-09-20): #7/#39 native acceptance/rejection now supports
explicit reasons and ordered per-criterion evidence notes via /review. Contracted
acceptance requires all met; historical boolean decisions gain no inferred notes.
Task v11 migration, exact replay, evidence tamper refusal, visible acknowledgment
and separately approved repair context pass. Native 185 passed / 6 ignored,
strict Clippy and installed PTY pass. Limited approval, human escalation and tiered
automatic routing remain unfinished.
[Evidence](../Reports/2026-09-20-criterion-review.json).


Dependency gate refresh (2026-09-20): current RustSec database (1,251 advisories)
reports no known vulnerabilities or warnings across 213 locked dependencies, with
no ignored advisories. The vulnerable fixture failure gate and nine notice tests
pass. Current criterion-review candidate source/payload hashes and its exact audited
lockfile match; no product changes or rebuild were needed. Application security,
license compatibility, remote CI and full release acceptance remain separate.
[Evidence](../Reports/2026-09-20-native-dependency-audit.json).

## 2026-09-20 explicit review outcomes

Native review now records all five outcomes, including limited approval and a
human-review hold that requires explicit resolution. Both approvals require the
original successful check and all recorded criteria met. Schema v12 preserves
exact older-store backups and historical reviews. Regression coverage exercises
all outcomes with isolated workers, failed-run approval bypass refusal, dependent
dispatch blocking, note replacement and the installed terminal journey.

Validation: 189 native tests passed, six intentionally ignored; final affected
suites, strict Clippy and installed release smoke pass. MIT and third-party notices
are bundled. Exact candidate/hash evidence is in
`.agent/Reports/2026-09-20-review-outcomes.json`. Automated tiered repair/routing,
formation/chronology, recovery/retirement and production qualification remain open.

## Native repair routing requirements after the five-outcome port

GitHub #7 was re-read on 2026-09-20. Its checked boxes describe the legacy product,
not native acceptance. The Rust port still needs the following connected work:

- Persist review failure classification and rejection lineage so critical,
  security and merge-risk failures route to the user, ordinary first/second
  rejections distinguish same-agent/fresh-agent repair, and repeated architecture
  failures return to Architect revision.
- Define Local Agent identity independently of model name and task/run identity.
  Current explicit repairs inherit the model and reference evidence, but every run
  starts a fresh model request. Model reuse alone does not prove same-agent repair.
- Make the routed action visible and receipt-bound across restart, then connect it
  to a governed proposal/approval/dispatch flow. A displayed recommendation alone
  does not fulfill automatic routing. User/Architect escalation must block worker
  dispatch until the relevant decision is resolved.
- Prove complete rejection sequences and escalation paths with real worker/model
  transports, durable replay and installed terminal journeys, retaining evidence,
  policy boundaries and exact-once child creation throughout.

The repair-hold regression is a prerequisite: a held child cannot be bypassed by
proposing a new sibling from its original parent. It does not complete this route.

## 2026-09-20 typed risk escalation

GitHub #7's critical/security/merge-risk route now operates on explicit native
reviews. Any declared risk on a nonapproving decision holds the task for human
review and blocks repair/dependent dispatch. Approval carrying risk refuses;
explicit resolution retains original successful-check and criterion requirements.
Task v13 preserves exact old-store backups and unclassified reviews; older schemas
cannot claim classified risk. Activity and evidence show the persisted escalation.

Validation: 192 native tests passed / six ignored, strict Clippy and installed PTY
pass. The real worker matrix covers all three risk classes on successful and failed
runs, restart/retry/conflicts, blocked bypasses and resolved dependent dispatch.
[Evidence](../Reports/2026-09-20-risk-escalation.json). Same/fresh-agent and Architect
routing, automatic model review, full formation/chronology, recovery/retirement and
production launch qualification remain open.

## 2026-09-20 Local Agent conversation continuity

New worker evidence binds a bounded retained user/assistant conversation and
mission-local agent identity. First repair sends the prior exchange; second/later
terminal rejection in ancestry starts a fresh agent with current repair evidence.
Needs repair does not count as rejection. Current policy and fresh approval remain
required. Missing/corrupt referenced history refuses before run claim; legacy or
incomplete history, changed models and budget rollover are explicitly fresh.

Native 197 passed / six ignored, strict Clippy and installed PTY pass. Real HTTP
regressions verify exact history, identity, restart and refusal paths. An independent
subagent reviewed correctness and added edge coverage. Artifact transcript schema1;
task13/conversation6/scope2 unchanged. [Evidence](../Reports/2026-09-20-agent-continuity.json).
Automatic review-triggered repair proposal/launch and Architect routing, model
qualification, full formation/chronology, recovery/retirement and launch remain open.

## 2026-09-20 atomic review-to-repair proposals

No-risk NeedsRepair/Rejected reviews now atomically record the parent outcome and
an inherited Proposed repair child. Fresh approval is still required. Risk holds
create no child. Parent and child Activity searches expose the receipt, and parent
readiness links the child. Task v14 preserves exact backups and old review meaning.

Native 200 passed / six ignored, strict Clippy and installed PTY pass. Subagent
coverage verifies complete atomicity/refusal/replay and actual continued/fresh model
requests, including a failed parent with an already human-held child. The installed
journey verifies the single review command and absence of immediate inference.
[Evidence](../Reports/2026-09-20-atomic-review-repair.json).

The design review identified an outstanding graph gap: accepting a repair does not
resolve dependents referencing the rejected original task. Governed replacement,
Architect routing and the broader product/launch gates remain unfinished.


## 2026-09-20 explicit repair dependency resolution

`/resolve-repair ID` selects one accepted repair for its unsuccessful ancestor
chain. Future consumers keep declared dependency IDs and record actual accepted
sources; original outcomes remain intact. Resolved families cannot reopen through
new repairs or reviews. Outstanding sibling work/human holds block selection.
Repairs retain their parent's exact baseline and inputs. Task schema15 rejects
new authority in older schemas and backs up v1–v14 bytes before migration.

Two subagents implemented dependency/dispatch/readiness integration, supplied five
real-worker regressions and independently reviewed the store. Review found replay
and post-resolution hold bypasses; both now have guards and regression coverage.
Native 206 tests passed / six ignored; strict Clippy and installed archive acceptance pass. [Evidence](../Reports/2026-09-20-repair-resolution.json). Architect
routing, full formation/chronology, model qualification, recovery/retirement and
production launch remain open.


## 2026-09-20 Architect repair revision routing

Repeated explicitly classified architecture failures now invoke Frontier Architect
with verified bounded lineage evidence and the pinned source commit. Human risk
holds take precedence. Trusted origin survives refinement and restart; no inference
replays automatically. Adoption creates one linked Proposed repair with revised
policy/model/criteria, fresh approval and agent; dependencies and baseline remain
fixed. Superseded branches cannot bypass revision. Cancelling an unstarted adoption
restores the source route for explicit replacement.

Two subagents implemented planner/context capture, supplied five real-worker/HTTP
regressions and reviewed bypasses. Native 214 passed / 6 ignored,
strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-20-architect-routing.json).
Full multi-task mission-graph revision, automatic execution authorization, formation/
chronology, model qualification, recovery/retirement and production launch remain open.


## 2026-09-20 background work visibility and safe Activity navigation

Local coding work and actionable canonical outcomes remain visible while chatting,
including narrow terminals. F4 opens receipt-backed Activity without moving the
conversation reading position or discarding drafts. Resolved/handled ancestors
avoid duplicate alerts; pending repair approval remains visible. Running records
without a local worker preserve process-ownership uncertainty.

Independent review reproduced a failed-save/refresh path that could discard an
unsaved plan; draft cleanup now requires the exact acknowledged Plan receipt.
Two subagents supplied UI/layout and projection/draft tests plus review. Native
221 passed / 6 ignored, strict Clippy and installed controlled-worker
acceptance pass. [Evidence](../Reports/2026-09-20-work-awareness.json). No schema
changes. Unified conversation/action chronology, full formation, model qualification,
recovery/retirement and production launch remain unfinished.


## 2026-09-20 large-history work-status redraw regression

A bounded 256-task/4,095-receipt repair-chain fixture reproduced ~138ms redraws.
The Architect requirement predicate scanned ancestors before checking whether a
route receipt existed. Reordering pure eligibility checks reduced warm redraws to
~3.7ms (~37.5x in this optimized local fixture) with identical counts. Human
holds, terminal-child routes and cancelled-adoption reopening retain their meaning.

Two subagents supplied the explicit ignored benchmark and semantic review. Native
221 passed / 7 ignored; benchmark executed before/after, strict Clippy
and installed archive pass. [Evidence](../Reports/2026-09-20-work-awareness-performance.json).
This is component rendering evidence, not model latency or full product qualification.
Unified chronology, formation, recovery/retirement and launch gates remain open.


## 2026-09-20 observed task receipt chronology

Canonical task acknowledgments now interleave with conversation presentation at saved local observation boundaries. Exact revision/task/correlation identity gates phase rendering, model requests exclude references, and initial history remains Activity. Schema8 migration preserves exact previous bytes. Retry stream growth preserves later receipt reading positions. Three subagents implemented and reviewed the slice; 229 native tests pass, 7 opt-in tests ignored, strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-20-observed-chronology.json). Full causal command/planner/Workspace/Mission chronology remains open.


## 2026-09-20 stable transcript reading blocks

Typed message/receipt reading keys replace numerical insertion correction, preserving viewed entries across earlier content insertion and retry growth. Conversation schema9 preserves exact v8 backups and refuses invalid keys. A direct UI regression compares against numeric-only behavior. Focused39 and full232 tests pass, 7 opt-in skipped; strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-20-stable-block-reading.json). Next: [durable command lifecycle](native-command-chronology.md); its plan explicitly retains every adapter and crash/uncertainty requirement. Exact character reflow and wider launch requirements remain open.


## 2026-09-20 durable prepared command intents

Schema10 records typed command intent/attempt before dispatch and derives acknowledgment from exact canonical receipts in the original session. Prepared task/scope/run/branch/recovery adapters capture immutable targets. Explicit restored retry retains origin, and history reserves capacity for unknown outcomes. Three subagents implemented and reviewed the slice; 245 native tests pass / 7 opt-in skipped, strict Clippy and installed archive pass. The PTY forces failed intent publication and proves no task creation, then retries a restored command from another session without changing task state. [Evidence](../Reports/2026-09-20-durable-command-intents.json). Remaining adapters and full completion boundary stay in [the command plan](native-command-chronology.md).


## 2026-09-20 worker command lifecycle

Explicit Run commands now retain distinct exact Start and same-run Finish phases in their originating conversation, with canonical identity/digest checks and no status/ownership inference. Real workers cover success, failure, cancellation, restart and retained-evidence recovery. Schema11 preserves v10 bytes and expands reading bounds. Focused43/full249 tests pass (7 opt-in skipped), strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-20-worker-command-lifecycle.json). Background dispatch without a command and the remaining [causal adapters](native-command-chronology.md) remain open.


## 2026-09-20 planner command lifecycle

Explicit planner operations now retain saved exact request identities, distinct draft outcomes and origin-bound restart recovery. Revision/cancellation and Architect source checks refuse changed targets; no automatic inference replay or task approval is introduced. Schema12 preserves v11 bytes. Focused65/full254 tests pass (7 opt-in ignored), strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-20-planner-command-lifecycle.json). Remaining causal adapters and production requirements stay open.


## 2026-09-26 controller commands

Active cancellation and dispatch toggles now use saved process-bound intent and distinct local outcomes. Same-run Finish remains authoritative for worker results; stale controller, worker, scope and epoch checks prevent retargeting/restart replay. Live dispatch status appears in wide chat headers. Schema13 preserves v12. Focused84/full262 pass (7 ignored), strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-controller-command-lifecycle.json). Automatic launch and other causal adapters remain open.


## 2026-09-26 automatic launch admission

Automatic workers now use saved exact intents linked to the enabling dispatch command in its origin session. Schema14 preserves v13; controller/scope/approval/revision checks prevent stale starts, and restored entries never replay. Automatic insertion preserves reading/composer. Focused79/full267 pass (7 ignored), strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-automatic-launch-admission.json). Remaining causal adapters and full production goal stay open.


## 2026-09-26 automatic Architect admission

Automatic draft generation now uses saved exact intents linked to its triggering review in the original session. Source/revision checks also run after queued model admission; pending cancellation cannot be undone by a late save. Schema15 preserves v14 and retains drafts without inference replay. Focused127 and full274 pass / 7 ignored, strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-automatic-architect-admission.json). Remaining causal adapters and full production goal stay open.


## 2026-09-26 Wayfinder saved admission

Scope entry, draft and confirmation now use exact saved turn-bound intents before canonical writes. Cancellation/restart never replay unsent work; explicit retries retain identity across sessions. Schema16 preserves v15, and scope receipt headers require canonical proof. Focused83 and full286 pass / 7 ignored; strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-wayfinder-command-admission.json). Workspace/mission events and full release requirements remain open.


## 2026-09-26 workspace selection continuity

The picker now gathers the full choice before creation. Exact startup/source admission, partial-creation observations, destination capacity and source/target owner retention are implemented. Inference admission across terminals is the next proposed runtime slice. Focused 104 and full 305 pass / 7 ignored; separation guard RED/GREEN, strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-selection-continuity.json). Full production requirements remain open.


## 2026-09-26 shared inference admission

Same-user terminals now share normalized-endpoint client capacity, with bounded foreground priority, conflicting live configuration refusal and cancelled/dead-owner cleanup without replay. Transient queue telemetry distinguishes application admission from upstream waiting. A deterministic worker cancel/grant race was reproduced and fixed; exact Running identity rechecks allow unrelated revisions, and Finish waits for provider shutdown. Final327 native tests pass / 7 ignored, strict Clippy and installed suites (existing2 + concurrent1) pass. [Evidence](../Reports/2026-09-26-shared-inference-admission.json). [Runtime/context qualification](native-inference-qualification.md) is the next proposed slice; no speed improvement or full production acceptance is claimed.


## 2026-09-27 bounded native inference diagnostics

The explicit paired diagnostic runner now binds actual requests, per-request runtime
observations, independent checks and canonical review outcomes. Report inspection never
replays work. The full native suite passed 357 tests / 7 ignored, followed by 48 final
focused checks, strict Clippy and all 5 installed tests. Live testing exposed a fixture
contract omission, corrected without weakening its oracle. The corrected 8-case cohort
completed all 16 generations but only one required-source candidate reached canonical
acceptance; failed checks and refused repair plans remain visible. No model/default or
promotion change follows. [Evidence](../Reports/2026-09-27-native-inference-qualification.json).
The next proposed user-visible slice is the [Mission Work tree](native-mission-work-tree.md);
full formation, recovery/retirement, model quality and launch acceptance remain open.

## 2026-09-27 native Mission Work tree

Canonical Plan groups, manual work and repair ancestry now form the task tree,
with dependency edges kept separate. Exact task anchors survive filtering and
restart; group rows cannot target task shorthand. Evidence request identities
prevent stale view replacement, and incoming reviewer text follows the newest
acknowledged snapshot. Wide overview and focused/narrow review layouts preserve
the composer and exact inspector.

[Verification and shipped preview](../Reports/2026-09-27-native-mission-work-tree.json)
records native379/7, final focused32, installed5, formatting/Clippy and archive
hashes. Counts overlap. The [next recovery plan](native-check-result-recovery.md)
remains proposed; broader rewrite and production launch requirements remain active.
