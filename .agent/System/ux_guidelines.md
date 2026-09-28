# UX Guidelines — Alfredo Workstation

**Last Updated:** 2026-09-27
**Implementation reports:** [Alfredo one-shot workstation correction](../Reports/2026-07-11-alfredo-one-shot-workstation.md), [install and Queue acceptance correction](../Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md), [Wayfinder Shared Understanding Gate](../Reports/2026-08-03-wayfinder-shared-understanding-gate.md), [deterministic runner supervision](../Reports/2026-08-09-issue-65-deterministic-runner-supervision.md), [Retirement Storage and Blocked Outcomes](../Reports/2026-08-09-issue-68-retirement-storage.md)

## Design Philosophy

Alfredo is a prompt-first coding-agent workstation. The dominant Agent Console should feel as direct as Codex or OpenCode: type a discussion, command, skill request, or coding task; see the user turn immediately; then follow the controller, command, approval, and Local Agent outcomes in one chronology.

Mission Work is the persistent secondary lane. It provides operational awareness through real canonical subagent sessions, attention, workable Issue Slices, evidence, and typed actions. It must not compete with the prompt for visual dominance, disappear into chat scrollback, or invent accepted state before the Orchestrator acknowledges it.

## Visual System

- Use near-black neutral surfaces, restrained cyan for focus/connection, lime for healthy/launchable state, amber for repair/waiting, and red for destructive or rejected actions.
- Use `Ubuntu Sans` with Aptos/Segoe UI fallbacks for readable interface text and `Ubuntu Mono` with Cascadia/SFMono fallbacks only for commands, identifiers, and artifacts.
- Do not use remote font imports. Avoid micro text below 0.7 rem and avoid decorative uppercase paragraphs.
- Borders, spacing, and shadows communicate grouping; they do not create ornamental dashboard chrome.
- Every flex/grid child that can contain variable content must be shrink-safe (`min-width: 0`) and long identifiers must wrap or ellipsize intentionally.

## Layout and Reflow

- Desktop uses two columns: a wider Agent Console and a narrower persistent Mission Work lane. Each lane owns its own scroll area; the page shell does not create competing nested scrollbars.
- At 1040 px and below, Mission Work stacks below Agent Console. At 680 px and below, headers/forms stack and the prompt composer remains reachable. At 520 px, multi-column controls collapse to one column.
- The shell uses dynamic viewport units and bounded menus/inspectors so browser or desktop chrome cannot push actions off-screen.
- The prompt composer remains pinned beneath the independently scrolling chronology.
- No fixed minimum column wider than the available viewport is allowed.

## Agent Console Interaction

- Before Mission Work exists, Agent Console is the Coding Workspace selection surface. It names the exact Starting Location, explicitly states that no Coding Workspace or Mission is bound, and offers exact existing-repository selection or new-repository creation.
- Pending selection says it is waiting for Orchestrator acknowledgement and must not display the candidate as accepted. Structured rejection displays the exact failure code and message, remains selection-required, and keeps retry available. Only an acknowledged receipt may show the canonical Coding Workspace; the next state says `Mission selection required` and must not fabricate or load a Workspace Session.
- Render user prompts optimistically before persistence or model inference completes. On rejection, restore the submitted draft only if the composer is still empty; never overwrite a newer prompt typed while the earlier save was pending. Explain the failure inline.
- When Python returns a non-`outside` Wayfinder projection, show a named status line with `Wayfinder / Chart mode` or `Wayfinder / Work-through`, the Shared Understanding Gate state, and whether the durable flow is continuing. That line conveys routing state only: it must not receive receipt styling or claim an artifact, delegation, skill invocation, or production action.
- Merge canonical messages, command cards, and consequential workstation actions into one arrival-ordered chronology with stable type-qualified keys. After restart, causally anchor durable proposal/approval/queued milestones after their originating controller turn and before later chat. Receipt-bearing entries display the exact correlation id and phase; controller commentary never inherits that treatment.
- Auto-follow only when the reader is already near the end. A newly submitted optimistic prompt always becomes visible; incoming background events must not pull a reader away from older history.
- Enter submits and Shift+Enter inserts a newline. While persistence is in flight, another Enter must not begin a concurrent prompt mutation and must preserve newly typed text.
- The command/skill palette focuses its first option when opened, supports keyboard selection and Escape, and returns focus to the composer after Close or selection.
- `/help`, `/skills`, and `/status` are deterministic and do not invoke a model. `/run` uses Shell governance. `/use` requires an installed skill. `/task`, narrow remediation imperatives, explicit `ask … subagent to …` prefixes, and `fix … with a subagent` suffixes use a deterministic fast route; questions, explanations, ambiguous checks, and other natural prompts use the controller's typed `discussion | coding-task` route. A malformed, blank, oversized, or invalid model route becomes fixed discussion feedback and cannot preserve success-sounding model prose; slash commands are never redispatched through the model.
- Style `model-commentary` as non-authoritative and place its explicit `No action taken` or `No action has occurred` outcome prominently beside it. Raw model reply prose is never used as an action result; persist the deterministic discussion/coding-route template instead. Queue attention remains in Mission Work and is not duplicated into Agent Console without a receipt. Proposal, Mission Commander decision, queued session, running session, validated evidence, Review Decision, and accepted completion remain separate entries and never collapse into one generic success line.
- Treat the typed controller action message as fixed backend copy, not caller-supplied prose. Suppress Queue proposal/decision/queued chronology when a legacy projection cannot recover its exact correlation, and suppress session lifecycle milestones when nested runtime identity does not validate against its canonical receipt chain.
- Healthy runner supervision is invisible: do not add Agent Console narration, Mission attention, status churn, or repeated success notices for no-change observations. Recovered, late-result-reconciled, and decision-needed outcomes may enter Agent Console only from their exact canonical supervision receipt and must reconcile exactly once.
- Every accepted Workstation action turn carries the correlation returned by the typed acknowledgement and displays `Receipt <id> · workstation-action-acknowledged`. If it differs from the requested correlation, render a failed non-receipt turn and no success claim.
- Automatic approval is allowed only after the canonical proposal exactly matches the originating Mission, Conversation Scope, user goal, acceptance criteria, allowed paths, command policy, proposed eligible worker, and message identity. Eligible workers must explicitly report `assignable: true`, `delegate_only: false`, and `requires_approval: false`, be local/non-cloud and available, and use worker/local-agent routing; missing authority metadata fails closed for both automatic and manual assignment. The chronology must still show the proposal, approval, and queued session as distinct acknowledged events. Gated, delegate-only, unavailable, unsafe, or mismatched work pauses for manual handling.
- Never describe a controller proposal as acknowledged execution. Only a matching canonical Orchestrator receipt may show a task, launch, permission, file/mutation, review, or accepted-state transition as accepted. See the [false-success diagnosis](../Reports/2026-07-24-workspace-selection-false-success-diagnosis.md) and [receipt-binding implementation](../Reports/2026-08-02-conversational-action-receipts.md).

## Mission Work Interaction

- Project Local Agent cards from canonical Mission/session state and qualify card, action, transcript, and continuity identities with Mission id.
- Show task, model, role, status, progress, attention, touched files, latest command/test, evidence, and next action in human language.
- Render last activity from a validated canonical timestamp and show `Not recorded` when none exists; never synthesize time from revision numbers or evidence counts.
- Keep blocked, waiting, failed, and active work discoverable; completed work may be grouped separately. Archive/restore is available only for canonical completed/pr-ready or tracker-merged Issue Slices and groups retained work without hiding its identity, sessions, evidence, or Activity Journal history.
- Treat cancelled/canceled Local Agent work as terminal unsuccessful state, never as done or completed work.
- Approve, launch, retry, cancel, assignment, evidence review, repair, escalation, archive/restore, and queue decisions use typed expected-revision requests and create visible human/orchestrator turns. The inspector states each action's exact consequence before submit and never shows an accepted outcome before a matching acknowledgement.
- A persisted repairable review from Review Workspace, TUI, CLI, or legacy state exposes exactly one `Launch repair` action after reload. Before launch it shows the derived inherited task packet (goal, acceptance, paths, command policy, evidence requirements, agent, and review reason); the preview never creates a second task/session. Ordinary failed work without a repairable review still requires an explicit retry reason.
- For a blocked Issue Slice, show blocker recommendations with the blocker rationale, proposed accepted boundary, assigned actor, and that approving or creating follow-up work does not unblock the original. Only a reviewed `pr-ready` or complete blocker outcome can do so.
- Show a supervision receipt and the `1/1` automatic-recovery budget on the affected Local Agent session. A failed or blocked recovery is Mission Work attention linked to that exact session, never a Workspace Queue item. When the session is terminal failed, expose the existing typed manual Retry action with a required Mission Commander reason; leaving it stopped remains the safe alternative.
- Keep Local Inference visible as a compact secondary telemetry block above the Mission Execution Tree and as exact-session details in the inspector. Distinguish `Idle`, `Queued`, `Running`, `Complete`, and `Non-authoritative` in text. Show queue priority/sequence, resident-model affinity, exact model digest, keep-alive, context/output budgets, thinking/sampling, schema, quantization, residency, requested processor policy plus observed GPU/total bytes, admission/headroom, usage, and load/prompt-evaluation/first-token/decoding timings. A complete schema-valid result may be marked authoritative only when its running-model digest/placement and all timing/usage metrics were also verified; partial, malformed, oversized, timed-out, cancelled, transport, queue, lease, metadata, or digest outcomes must visibly say non-authoritative and must never look like accepted Mission work. The Lease is scheduling metadata only and cannot imply governance authority.
- Qualification Reports are evidence about repeated governed cohorts, not accepted Mission work. Any future inspection surface must show the exact Profile/runtime pin, fixture coverage, quality/reliability, repair/escalation cost, decomposed timings, reviewed latency, context/prefix reuse, promotion blockers, and rollback state as report metadata; it must never render prompts, raw model streams, source-dependent plans, Evidence Packages, or authority decisions as reusable truth.
- Show `snapshot-storage-exhausted` as prominent blocked Mission Work attention whose visible guidance points to Agent Console `/storage`; do not route it through an unsupported Operations Workspace view. A blocked Retirement Unit inspector directly shows phase, blocker reason, runner boundary, Preservation Budget, and any compact Retirement Record, then only actions the backend says are possible: pin/unpin, bounded retry, exact export with an explicit destination, and irreversible discard with both a reason and the exact session id. Direct retained-worktree export and discard stay unavailable while either runner-owner or process-group evidence is live or unproven; snapshot export remains available because it does not touch the retained source. Discard uses danger styling and stays disabled until the exact confirmation matches; no action may claim success before its Workstation acknowledgement and Activity Journal receipt.
- Routine selection, expansion, filtering, sorting, pinning, and diff navigation remain local UI state and do not pollute durable history.
- Mission Execution Tree parent rows expose a separately focusable, labelled expand/collapse control for pointer and touch use. The treeitem row still opens inspectable work and retains roving focus plus Home/End/Arrow navigation; do not nest one button in another. Repair lineage is stated in text and uses its own non-color shape while lifecycle and risk labels remain visible.
- The Mission Execution inspector remains inline in Mission Work above 680 px. At **680 px and below**, it is a modal dialog: focus starts on Close, Tab remains within the dialog, Escape and Close restore the originating tree control, and the dialog owns the only inspector scroll surface. Closing it leaves Agent Console and the composer reachable.
- Detailed Local Agent output is exact-session transient observer data. Show `Subscribing` only until a valid exact response arrives, preserve prior received output on a reader failure, and render the typed failure and safe retry inline. Never route raw output into Agent Console, Activity Journal, or accepted Mission state.
- Evidence controls exist only when the backend provides a registered review-safe artifact reference. Opening one uses the inline Session Artifact viewer with a loading state, bounded text, a truncation notice when applicable, actionable error/retry behavior, Close, and focus restoration. A lower-pane evidence control scrolls the viewer into the nearest visible position and moves focus to it. Never navigate a browser to a raw local-file path or synthesize evidence that does not exist.
- A queued session may be dispatched at most three times with bounded backoff, and only while canonical state still says `queued`.
- Recent workspaces may offer a shell-quoted `cd -- <workspace> && alfredo workstation --agent <controller>` relaunch command and a copy action. Selecting or copying one must not retarget the currently connected backend, and delayed clipboard completion must not report success for a newer selection.
- Issue Assignment projects active AFK/recoverable-session work, not completed/merged history or ready-for-human/HITL checks. Opening a lower detail view such as Queue replaces the assignment region rather than stacking a second operational surface beneath it.
- Workspace Queue is an actionable decision inbox. Show pending governance decisions and pending Mission Draft confirm/abandon decisions; hide resolved history and standing Mission Draft/Ad Hoc proposal forms. Creation originates from the prompt/controller flow. Render `No decisions pending` only after all authoritative decision sources finish loading successfully.

## Context and Scope

- Conversation Scope is implementation context, not the primary user workflow. The main status line says `Context · <label>` and opens the Context Inspector.
- Scope selection and Working Context curation live inside the inspector. Ticket rows and routine issue selection must not expose or mutate scope.
- Scope changes require explicit acknowledgement, remain stable across navigation, and never grant launch, file, command, or review authority.

## Accessibility

- Agent Console is the named main region; Mission Work is a named complementary region. Transcript, composer, cards, assignment board, inspector, queue, review, activity, and command audit use explicit landmarks/labels.
- Keyboard focus uses a visible high-contrast outline across controls, links, cards, inspectors, and decision outcomes. Palette and inspector transitions manage focus predictably.
- Text/background pairs meet WCAG AA contrast. Danger, warning, status, and availability remain expressed in text rather than color alone.
- Motion and transitions are removed under `prefers-reduced-motion`.
- Constrained-width layouts keep prompt, composer, critical status, and decisions reachable without horizontal-only access.

## Loading, Empty, Failure, and Recovery States

- Loading says authoritative Alfredo state is pending; it must not resemble a ready Mission.
- Empty is a valid acknowledged workspace with zero Issue Slices.
- Startup, persistence, transport, stale-action, model, sandbox, timeout, and permission failures show actionable text and preserve the last canonical state.
- Local Inference empty state says that no turn is recorded; queue and active-lease states show bounded runtime progress; a failed/non-authoritative receipt keeps its outcome and error visible without exposing prompts or raw model streams. The UI must not render a model completion as Mission acceptance.
- A dead runner owner is requeued with a bounded recovery count; a canonical queued session receives bounded UI dispatch retries. Terminal failure remains explicit rather than spinning forever.
- Raw token streams and terminal bytes are transient. Durable history contains finalized messages, meaningful decisions, summaries, evidence, and attributed outcomes.
- Command Audit polls authoritative Shell metadata during both direct submission and approval. `executing` is visible while the owner is live; a lost response reloads the same correlation; dead owners become durable `outcome-unknown`. Submit, approval, denial, unknown, and final audit phases are repaired before later Console/Activity entries so chronology remains causal, and unknown commands are never automatically run again.
- Contextual path access is rendered from a typed backend request containing the exact request id, Mission, canonical path, access, duration, reason, and affected action. React must not infer a grant boundary from rejection prose. Grant/deny feedback follows the backend-derived request status.
- Backend-authoritative accepted and pending state remains in canonical stores. React may retain only a bounded workspace-scoped tail of terminal negative Workstation action groups so a transport failure remains visible after refresh; corrupt or non-negative local records must not create accepted state.

## Verification

- `src/styles.test.ts` guards offline typography, minimum text size, scroll ownership, bounded overlays, and responsive breakpoints.
- `src/App.test.tsx` covers semantic regions, keyboard focus, optimistic echo, smart auto-follow, palette behavior, command/task routing, Mission-qualified continuity, dispatch retries, constrained widths, and governance states.
- `src/alfredo-release-seam.test.tsx` proves approve → launch → background execution → validated evidence → Activity → restart through the real Python backend.
- Queue/assignment regressions prove a clean zero-item inbox, replacement rather than stacking, absence of standing creation forms, and active-AFK filtering against the real issue metadata shape.
- `e2e/responsive-layout.pw.ts`, run by `npm run test:layout`, builds the production bundle and checks real Chromium at 1440×900, 1100×760, 820×900, and 390×844. Every viewport opens the capability palette, an enabled evidence-review action, expanded operational detail, and the inline artifact viewer before repeating overflow, containment, panel-separation, and control-overlap assertions. The final 2026-07-13 run passes 4/4; its first unrestricted tablet case caught 84 px of real overflow and an unreachable Send control before the grid-track correction.

## Native terminal task supervision

The Rust terminal separates task ID/status from title in its sidebar. A bounded
/tasks query or exact #ID filters the view; selection and shorthand commands are
constrained to visible tasks. No matching task means no implicit action target.
Evidence acknowledgment reveals/selects its exact target. Details explain policy,
approval and unaccepted-parent blockers separately. Search never narrows dispatch
scope or changes durable task state; the UI states that dispatch covers all tasks.

## Native terminal workspace selection

Native launch treats cwd as Starting Location and presents Workspace selection
required before opening task or conversation state. Enter validates an exact Git
root; F2 toggles explicit new-path creation. Mission selection follows acknowledged
repository validation. The input reuses grapheme editing and a cell-sized viewport;
status text filters terminal controls. Validation/creation runs asynchronously with
an observable pending state. Pending creation must finish before exit so the user
receives its result. Esc exits idle selection and restores terminal mode. Explicit
workspace/mission CLI arguments share validation and skip the corresponding forms.

At native mission selection, bounded asynchronous discovery lists saved names for
the acknowledged repository. Tab fills a name and Enter opens it; discovery never
dispatches work. Omitted/corrupt records remain visible as a discovery notice with
manual-name fallback. Terminal acceptance reconstructs alternate-screen entry/exit
so stale primary-buffer text cannot acknowledge a new product state.

Native task details also show the observed project scope gate, including outside-flow,
pending confirmation, confirmed and unavailable states. Pending or unavailable scope
stops automatic dispatch; later confirmation does not re-enable it. The task filter
includes the scope blocker for proposed/approved tasks. Existing running and review
items retain their own progress/reconciliation explanations. Visible task views refresh
in the background once per second (dispatch uses its existing faster interval), so
cross-mission scope changes appear without a foreground command. This observation is
advisory; authoritative task/worker admission still checks the journal under its lock.

## Native conversation client timing

The selected conversation shows a `Client` timing line. Queue time runs from submission
to the observed shared client-capacity admission event. First-text time runs from admission to the
first nonempty content event; streaming time runs from that event to the terminal
outcome. Total includes all phases. While waiting, the line shows admission pending or
waiting for text; no admission/first-text observation is fabricated on cancellation.
The active clock refreshes at most four times per second without incoming output.

These are monotonic UI-observed intervals, including event-delivery/render-loop delay,
not provider-internal measurements or a speed benchmark. Ollama's optional `Server
timing` remains separate. Completion, failure and cancellation freeze the intervals;
late events and prior retry attempts cannot alter them. A retry starts fresh clocks.
Timings are transient and excluded from saved conversations, so restart never revives
a stale running timer. Saved task/evidence schemas are unchanged.

Stale-plan readiness checkpoint: observed scope revision changes now produce a separate
selected-task blocker and searchable explanation for unstarted planned tasks, including
legacy plans without a scope binding once explicit scope begins. Manual start refuses
before recording an approval attempt; dispatch skips those tasks and can choose a later
eligible plan. The journal still revalidates the full binding under its lock; UI
observation does not grant authority or rewrite history.

### Native workspace handoff

`/workspace` reuses the repository and Resume/Start New Mission selector inside the
running terminal; Esc returns to the current work. The header shows current mission
and repository. Workstation owns each mission's App, TaskControl and conversation
owner. It loads and validates the destination before an ordered final source save,
then replaces the active state and releases the old owner. Target/open/save errors
preserve the current work. Selecting the same identity is a no-op. New mission
identity admission is retained if its later conversation opening fails.

Switching requires inactive conversations and workers, dispatch off, completed task
operations/model discovery, and a saved or explicitly cancelled plan draft. It never
cancels work or infers task completion. Fresh conversation/catalog event channels and
TaskControl isolate old asynchronous results from restored session IDs. Conversation
schema17 records exact source and arrival history; task schema16 is unchanged.
Provider admission is shared across same-user processes for each normalized endpoint.
Full concurrent supervision across workspaces and mission formation remain open.

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

Wayfinder preparation reads scope without mutating it. A prepared Enter, Draft or Confirm request and its originating user turn must be saved before the exact request dispatches. Routing writes are tracked independently of cancellable inference jobs. Cancellation
cannot abandon a pending receipt: polling still observes the outcome; switching and
normal quit wait for routing completion. Dispatch pauses while routing is pending,
and a newly pending gate turns it off. Safe inspection/reconciliation commands remain
available while new task actions wait for the routing result.

### Response source labels

Assistant turns display their structured source: `Model · NAME`, `Wayfinder · scope
receipt N`, or `Wayfinder · no action acknowledged`. Names refer to the requested
model, not a qualified digest/profile. Older messages show `Assistant · source
unrecorded`; their text does not retroactively establish attribution. The labels
survive restart and workspace handoff. They describe a historical response and never
replace current scope confirmation or task evidence/approval.

## Native transcript reading continuity

PageUp moves away from the latest output and anchors the selected conversation to a
logical transcript line and wrapped-row offset. Appending stream text preserves that
anchor. PageDown advances from the current reading point; reaching the bottom resumes
following live output. Starting or retrying a turn also follows the latest text.
Each session retains its own saved logical anchor, and hidden task/model views do not
render or change the conversation viewport. This also removes underlying chat text
from otherwise blank task-panel rows.

Resize preserves the logical line and clamps its wrapped-row offset to the reflowed
line. Client/server metadata remains pinned independently of transcript scrolling.
Logical lines before the reading point are omitted before applying the widget's
16-bit within-line scroll, so newline-heavy bounded output can show its actual tail
and remain navigable beyond 65,535 rows.

Conversation v4 saves the logical anchor and its last observed bottom offset.
Pending navigation remains transient. Restoring with different geometry or unseen
background output retains the logical line, with wrapped-row clamping on resize.
Versions 1–3 retain numeric-offset fallback until first render and receive exact
version-named backups on upgrade. Full character-level reflow anchoring remains open.

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


Native terminal panels (2026-09-20): Models and Activity use the focused compact
layout at narrow widths. Task details, scope, activity, evidence and plan content
share logical-line slicing and page-boundary clamping. Task-panel PageUp/PageDown
derives its step from the current rendered panel height, preserving one row of
overlap where possible and advancing at least one row in tiny panes. Resize updates
that height; compact inspectors must not skip unread rows. The 32×10 composer
remains reachable; evidence with more than 65,535 logical rows is navigable. These
are local view changes and do not write task receipts.


Native evidence retains an immutable styled projection and one width-specific
wrapping index. Composer redraws copy only visible logical lines; width/content
changes invalidate the index, while height changes reuse it. The cache is transient
and has no effect on task or evidence authority.

## Native stopped-worker recovery

Inspection distinguishes an active owner, a stopped worker with valid final
evidence, a proven interruption before check launch, a stopped worker with a
verified terminal check result, and an unknown or unverified outcome. Read-only
inspection must not record a result. For the verified post-check case show
**Worker stopped after check · saved terminal check result** and explain that
explicit `/recover ID` records failure without replay. Do not turn a passing check
badge into a completed task or accepted candidate.

After acknowledgement, the task is Failed and the evidence explains
**interrupted after check; candidate not finalized**. Keep the original bounded
check stdout/stderr and receipt available in `/evidence`, with no reconstructed
patch or candidate. The view must remain readable at normal size and 32×10 using
the current panel's paging geometry. Repeated recovery returns the same result;
the original remains failed and blocks dependencies. `/repair` proposes separate
work requiring fresh approval.

Valid saved final evidence has precedence and keeps its original outcome. Damaged
existing final evidence, legacy unbound intents, missing or partial post-launch
results, mismatched/corrupt records and uncertain receipts remain explicit and
unchanged. Never offer an automatic replay, respawn or inferred successful result.
The schema2 intent/schema1 checkpoint binding establishes retained evidence, not
same-user authenticity or proof that later helpers have stopped. Recovery does not
authorize worktree reuse, cleanup, retirement or claim full Runner Quiescence.
See the [active slice](../Tasks/native-check-result-recovery.md) and
[verification record](../Reports/2026-09-27-check-result-recovery.json).


## Persistent native work awareness

The terminal header shows local coding-worker activity and canonical attention
counts in both Tasks and chat views. Review-ready work, human holds, repair work,
pending Architect routes and accepted repairs awaiting resolution remain visible
without moving the conversation or composer. Completed/resolved and superseded
ancestors do not create stale alarms; cancellation of an unstarted repair restores
its parent's pending action. Recorded Running tasks without a local worker are
labelled recorded runs, never inferred live or dead from a task status alone.

Mission Work presents canonical Plan groups, a Manual tasks group and repair
descendants beneath their original work. Task IDs remain native task identities.
Dependencies appear as labelled edges rather than ownership; a dependency shared
by several tasks appears once. Original unsuccessful lifecycle labels remain
visible after an accepted repair supplies their dependencies. Counts name tasks
and local workers separately, exclude group rows from task totals, and distinguish
filter matches from the full task population.

Up/Down selects a visible task or group. Alt+Left collapses a branch or moves to its
parent; Alt+Right expands a branch or moves to its first child. Plain Left/Right
continues editing the prompt. A task inspector leads with the exact task, lifecycle,
model, readiness and recorded run, followed by current observation, evidence,
actions and recent saved receipts. Missing current observation remains explicit.
A group inspector shows group identity, counts and navigation guidance and has no
task action target. Task shorthand and F3 require a task row; explicit task IDs
remain available through the command surface.

Filtering retains Plan and repair ancestors and reveals matching paths through
collapsed branches. A hidden or missing task anchor never silently selects another
action target; explicit row navigation or `/tasks #ID` chooses one. The last task
ID and filter remain the saved selection preference while group focus and branch
collapse are transient. Restart expands branches and restores that exact task
anchor, leaving shorthand unavailable if it is absent from the view. Navigation,
filtering and disclosure create no task receipts or conversation actions.
At 32×10, a compact selected tree row remains above a scrollable task/group inspector
and the prompt composer stays visible. PageUp/PageDown uses that inspector's visible
height, with one row of overlap where possible, to reach its full content;
evidence, scope, Activity and planner panels retain their focused compact layout.
See the [Mission Work tree slice](../Tasks/native-mission-work-tree.md) for scope
and acceptance criteria, and its
[implementation record](../Reports/2026-09-27-native-mission-work-tree.json) for
verification status.

F4 opens saved task Activity from either view, preserves the draft and conversation
reading position, and hides an open planner panel without cancelling or discarding
its draft. F2 returns to chat. Narrow terminals use compact work/alert counts.
This projection introduces no task authority, unread claim, inferred actor or
persistence schema; unified conversation/action chronology remains separate work.


The work-status header checks explicit Architect route eligibility before expensive
repair ancestry analysis. A 256-task/4,095-receipt synthetic projection benchmark
covers cold and warm chat redraws; run it explicitly with `cargo test --manifest-path
alfredo-tui/Cargo.toml --release --test task_view measure_deep_repair_history_chat_redraws
-- --ignored --nocapture`. It reports timings without environment-dependent CI limits.
No cache can hide updated holds or cancellation state.

### Native observed task receipts (2026-09-20)

The native console interleaves exact task receipt references with conversation turns. Entries say **Observed task receipt**, show the acknowledged phase, task, revision and correlation, and verify all three identifiers against the current canonical task snapshot. An unavailable reference is explicitly unverified. These are local observation positions, not attribution to the selected conversation or proof of the originating command's position. No actor or timestamp is inferred.

Existing receipts on initial load remain in Activity; they are not backfilled into an invented conversation order. Newly observed receipts wait until the selected model turn is inactive, then append at that conversation boundary without entering model input. Saved references retain their positions across restart and handoff. Full causal command/proposal/planner/workspace chronology remains open.


### Native stable transcript reading anchors

Conversation schema9 records a stable block key (message index or canonical task receipt revision), line within that block, and wrapped row when the user reads older history. Rendering resolves the block before applying navigation, so receipt insertion or an earlier retried reply growing cannot silently change which entry the reader is reading. Legacy numeric anchors migrate on rendering without inventing command provenance. The wrapped row still clamps on geometry change; exact character-position reflow remains a separate requirement.

The complete causal command lifecycle is specified in [native command chronology](../Tasks/native-command-chronology.md): save intent before dispatch, bind exact receipts, and retain uncertain outcomes without replay. Observation placement alone does not satisfy that contract.


### Native durable task command presentation

Commands have stable command numbers and display Pending while their exact intent saves. Only after that save succeeds may prepared task/scope/run/branch/recovery operations dispatch. Their receipt phases stay in the originating conversation; inferred success from notice prose is forbidden. Unknown outcomes remain inspectable. `/retry-command SESSION:COMMAND` explicitly retries a saved intent, retaining its origin and increasing the saved attempt before dispatch. `/retry-task` can reuse the selected conversation's unresolved saved intent after restart. Input drafts remain independent of the pending save.

The ordinary prepared path does not yet complete the full [command lifecycle plan](../Tasks/native-command-chronology.md): Workspace events still require a dedicated causal adapter. Explicit controller operations use the separate lifecycle below. Planner operations use the separate draft lifecycle below.


### Native worker command lifecycle

An explicit Run command retains two separate phase lines in its originating conversation: the exact Start claim and its run-bound Finish result. A claim does not imply success, and `ReviewReady` does not imply acceptance. Without a matching Finish the line says `Result not acknowledged`; local worker disappearance and later task review status cannot fabricate completion. Claimed and completed receipts remain available in Activity, while duplicate observer entries are suppressed across conversations. Legacy background dispatch without a saved command keeps observation-only history; new automatic worker launches use the source-bound entry below.

The result slot is fixed-height in logical lines even while unacknowledged. Conversation schema11 allows the expanded Run block bounds and preserves schema10 bytes. Existing message/receipt/command block keys continue to identify history; an old anchor on a Run block's trailing blank may now point to its new result slot.


### Native planner command outcomes

Planner generation, revision, explicit Architect revision and cancellation keep saved intent and bounded outcomes in the originating command block. Dispatch waits for the exact intent save acknowledgment. A generated result says **Draft generated** and distinguishes generation from saving and approval; only the separate `/plan-save` task receipt acknowledges proposed tasks. Generation output never enters model conversation messages or supplies task authority.

Cancellation identifies the active generation and/or exact draft digest. Stopping an active revision retains its preceding completed draft; cancelling an idle completed draft discards that exact draft. The console distinguishes **Draft generation stopped**, **Draft discarded**, and failed operations. A changed cancellation target is refused rather than acting on a newer draft. Outcome text occupies one logical status line, preserving stable block reading anchors as wrapped height changes.

After restart, unresolved operations remain unconfirmed and do not replay inference. An exact retained draft may reconcile its originating generation; missing drafts do not prove failure. Automatic Architect drafts use the source-bound lifecycle below. Workspace chronology and exact character-position reflow remain separate requirements.


### Native controller command outcomes

Explicit `/dispatch on|off` and live-worker `/cancel-task ID` save immutable controller-bound intent before dispatch. Their local outcomes stay in the originating conversation. A dispatch entry says **Dispatch enabled/disabled for originating controller**: it records that operation on the originating process, while the live header (at 60 columns and above) and Tasks panel show current dispatch state. Restart keeps dispatch off; restoring a historical enabled outcome never starts workers. A changed controller or superseded dispatch epoch requires a new command.

Worker cancellation says **Cancellation requested** only after signaling the exact locally owned worker. Start identity binds the intended run; the Start receipt cannot acknowledge cancellation. A fixed separate result line displays only the exact run-bound Finish, or **Result not acknowledged**. A late successful Finish still says `ReviewReady` even when cancellation was requested. Missing ownership, task status and a local cancellation request cannot be rendered as canonical cancellation. An unknown or refused request remains visible alongside any independently proven worker result.

Conversation schema13 admits controller outcomes and the extra cancellation result slot. The command block keeps its stable reading key across phase updates, session switches and restart; unrelated composer drafts remain independent. Workspace chronology and exact character-position reflow remain unfinished.


### Native automatic worker launch presentation

Each automatic worker launch appears in the conversation containing the exact dispatch-enable command as **Dispatch · launch #N**. Its text names the task and enabling command number. It never labels the automatic actor as **You**, and switching conversations does not move the launch into the newly selected session. Admission preserves the current composer and reading anchor; background work does not force the user to the bottom of history.

The entry first shows **Pending · saving intent**. Only the exact saved entry can authorize the launch, and the live controller, dispatch epoch, task approval and scope are checked again before execution. The launch then uses the same distinct Start claim and Finish result lines as an explicit Run command. Missing canonical evidence remains **Result not acknowledged**, regardless of worker ownership or current task status.

Restart keeps dispatch off and unresolved launches unconfirmed; saved entries are history, never restart instructions. Existing exact receipts may reconcile their original launch without starting a worker. Conversation schema14 retains the source command link and the fixed result slot. Workspace chronology remains unfinished.


### Native automatic Architect draft presentation

An automatic Architect draft appears under the exact review command that requested the revision, labeled **Architect · draft #N**. The entry names the task and source review command number. The selected conversation is not treated as the origin, and appending the entry preserves composer contents and the older passage being read. The review command keeps its own canonical task acknowledgment; the Architect entry keeps only its independent draft outcome.

**Pending · saving intent** precedes generation. **Submitted · planner operation pending** does not imply a saved task. **Draft generated** continues to state that saving and approval are separate; `/plan-save` owns any later Plan receipt. Failed or stopped generation remains visible at the same entry. Architect generation uses one outcome line, without the worker claim/result slots.

Restored unresolved drafts stay unconfirmed without inference replay. An exact saved completed draft may reconcile the matching planner origin. Conversation schema15 preserves the source review request and unique inner planner identity, so explicit and automatic wrappers cannot claim the same generation twice. Workspace event adapters, complete mission formation and launch acceptance remain open.


`/plan-cancel` may withdraw an automatic Architect entry still waiting for its intent-save acknowledgment. The original entry becomes refused and is not released by a late save result; no generation-stop result is invented for inference that never started. Once generation has started, the existing exact-generation cancellation applies. Planner admission also rechecks the expected task revision and exact Architect origin after waiting for a shared model slot and before HTTP dispatch; a stale request reports failure without invoking the model.


### Native saved Wayfinder scope actions

Wayfinder keeps the existing user/assistant conversation turns and adds a compact **Wayfinder · scope #N · turn M** entry for a prepared scope mutation. The entry names entry, draft or exact-revision confirmation without repeating the user's full prompt. Only its equal canonical scope request acknowledges the action; ordinary discussion and model prose do not receive scope authority. Confirmation still grants no task approval or execution.

The entry first shows **Pending · saving intent**, then **Submitted · awaiting scope receipt**. The saved user-message index keeps a delayed preparation with its original turn even if later chat or commands already exist. Rendering orders command entries by that boundary and then their shared sequence. Insertion and receipt updates preserve the reader's passage and newer composer contents.

Switching conversations or cancelling the assistant reply does not move or erase a submitted scope action. Its exact receipt remains visible at the originating turn even when the reply is no longer eligible for updates. Restoration shows unresolved requests as unconfirmed, never automatically writes scope or starts inference, and can reconcile an exact existing receipt. Explicit retry retains the original request and origin. Existing response-source labels remain separate historical reply provenance.

Fresh application notices take precedence over a selected conversation failure in the footer, so cross-session action outcomes remain visible. The session keeps its failed status and partial reply; when the notice clears, the footer returns to the failure detail (or the visible task view notice).

### Native workspace and mission selection history

The picker collects repository and mission choices before creation. Cancelling it
leaves the source work and destination untouched. Once confirmed, **You · selection
#N** stays in the originating conversation; **Workspace · arrival #N** records the
same exact request in the destination. Startup uses an independent journal origin.
The request text keeps the target path and mission separate from bounded failure
details. These entries remain outside model messages and preserve the selected
conversation, unfinished composer and reading passage.

Selection uses one stable outcome slot, with distinct repository, mission, target
loading and handoff milestones. **Repository created** and **Mission created**
remain visible when a later operation fails. **Handoff prepared · selection not
recorded** cannot claim a workspace switch. Only the post-swap observation says
**workspace selected**; the current header remains the live workspace identity.
**Already current** records the no-op choice. Selection observations use neutral
styling, failure uses an error style, and uncertain history remains visibly
unconfirmed. None receives task receipt or approval styling.

Before-swap failure retains source work; created artifacts are retained for
inspection. Restart does not repeat creation or handoff. A missing, malformed or
nonmatching journal cannot validate a saved outcome; an unfinished dispatch keeps
its last observed phase as unconfirmed. Recovery requires an explicit inspected
Open/Resume choice. Full mission formation and production acceptance remain open.

### Native shared inference queue presentation

Before an admission observation, **Preparing request** makes no server-state claim.
**Queued for Alfredo** names waiting for shared client capacity. When available, a
separate metadata line shows foreground/background class, projected queue position,
total waiting requests and active/configured slots. Position includes this request,
excludes active slots and may change as priority requests arrive. The display never
invents a position while waiting for its first coordinator observation.

After admission, **Waiting for model server** replaces queue telemetry. This does
not distinguish network delay, server contention, loading or prompt evaluation
without evidence. **Thinking / waiting for text** requires an actual thinking event;
answer content starts Streaming. Client queue/first-text/stream intervals remain
separate from optional server metrics. Cancellation and completion freeze timing;
stale attempts cannot revive queue state or change a newer turn.

Queue metadata remains visible beside live timing in wide and narrow conversation
views without entering model messages or saved history. Foreground priority never
preempts active work; bounded background progress is scheduling, not task authority.
Shared capacity covers Alfredo requests for the same normalized origin and user,
not external clients, GPU headroom or qualified inference performance.
