# Native command-to-result chronology implementation plan

Date: 2026-09-20
Status: partially implemented; prepared task/scope/run/branch/recovery intents and save barrier integrated; remaining adapters and complete lifecycle acceptance remain open
Authority: current Rust rewrite goal; Agent Console chronology requirements in
UX guidelines; current orchestration in
[context](context.md).

## Required outcome

A submitted command appears immediately in its originating conversation as pending.
Its durable intent is saved before dispatch. Only the exact canonical acknowledgment
may mark an action acknowledged. Pending, refused, acknowledged and uncertain
outcomes remain inspectable across session changes, retries and restart. Background
worker results remain linked to their initiating operation. They are not reassigned
to whichever conversation happens to be selected when a callback arrives.

Observed task receipts do not meet this requirement: they record console observation,
not command causality, actor attribution or original event time. Preserve that honest
label for existing observations. Never invent historical command anchors.

## Existing seams

- `src/main.rs` handles slash submissions, invokes `TaskControl::command`, then clears
  the composer after admission. Its periodic conversation checkpoint currently runs
  independently of task dispatch. It also drives dispatch and Architect generation.
- `src/task_control.rs::command` returns `Result<(), String>`, covering both view-only
  commands and asynchronous mutations. `Ok(())` therefore does not acknowledge a
  canonical action. Ordinary parsed mutations and `/plan-save` create exact `Request`
  values, retain them in `retry`, and call `launch`.
- `TaskControl::launch` handles ordinary task transactions and special asynchronous
  model assignment. Current result channels primarily carry a snapshot, notice and
  optional projections. Notice prose is not an acknowledgment identity.
- `TaskControl::start_worker` allocates a correlation separately and sends final worker
  results through `worker_receiver`. Intermediate Start receipts can first arrive
  through a background snapshot refresh.
- `/branch`, `/recover` and `/scope` each have distinct asynchronous paths. Scope uses
  its own request/receipt namespace. Recovery can acknowledge an existing run result.
- `TaskControl::poll` and `apply_outcome` admit snapshots and preserve newer revisions.
  `newly_observed_receipts` emits canonical receipt references after initial load. It
  intentionally skips historical receipts and cannot recover command origins.
- `src/planner.rs` generates proposals and checkpoints completed drafts. Generation
  completion is not task publication. `/plan-save` is the separate Plan transaction.
- `src/conversations.rs::Autosave` checkpoints conversation state asynchronously. Its
  save acknowledgment must become a prerequisite for dispatching a new durable intent.
- `src/model.rs` keeps task receipt references separate from inference messages;
  preserve that separation. Keyed reading must survive insertion and phase updates.

## Durable state machine

1. **Local admission:** validate command syntax and capture originating namespace,
   stable session identity, submission ID, bounded submitted text and typed intent.
   Render `Pending — saving intent` immediately without claiming task acknowledgment.
   Preserve any newer composer text if admission or persistence later fails.
2. **Intent publication:** checkpoint the pending entry and its immutable intent.
   While publication is pending, do not dispatch the operation. Prevent duplicate
   Enter from admitting the same submission; independent drafts remain editable.
3. **Dispatch eligibility:** only a matching save acknowledgment authorizes dispatch
   of that in-memory submission. Revalidate current task/scope revisions normally;
   persistence does not grant policy approval. A stale intent may be refused.
4. **Execution observation:** carry submission ID and typed intent through every
   asynchronous channel. Render submitted/in-flight phases without inventing actor,
   time or success. Anchor subsequent phases to the original submission key.
5. **Canonical acknowledgment:** reconcile the exact immutable request against its
   authoritative receipt namespace. Store a typed receipt reference, then render
   acknowledgment from the verified source. A successful refresh or command return
   alone must not complete a submission.
6. **Error or interruption:** retain the error text. Distinguish known pre-effect
   refusal from uncertain publication/execution. Existing string errors can include
   post-publication directory-sync failures; do not classify every error as rejection.
7. **Restart:** restore unresolved submissions as `Outcome unconfirmed — reconcile`.
   Never dispatch automatically from a saved pending state. Reconciliation may attach
   an exact already-existing receipt. Receipt absence does not prove that worker,
   branch or recovery effects never occurred.
8. **Explicit retry:** retain the original submission identity and exact canonical
   request, with a separate retry-attempt phase. Retrying from another conversation
   must not transfer ownership of the original chronology entry. A changed request is
   a new submission, not an idempotent retry.

## Typed adapters

| Operation | Persisted intent and acknowledgment |
|---|---|
| Ordinary task mutation, assignment, Plan save | Exact task `Request`; acknowledge only equal receipt request. Assignment provider checks do not themselves acknowledge assignment. |
| View/filter/refresh/evidence inspection | Explicitly view-only; no fabricated task mutation receipt. A refresh can reconcile another submission but is not that submission's acknowledgment. |
| Explicit worker run | Task ID, expected revision and generated Start correlation. Worker preparation supplies baseline/inputs; bind the admitted intent to the matching Start action and revision. Link later Finish by exact run identity. |
| Automatic dispatch | Explicit automatic mission operation, linked to approval/task identity. Never attribute it to an unrelated selected-session command. |
| Branch publication | Stable correlation, task/run and expected revision; reconcile the exact Branch receipt and existing branch-publication protocol. Do not infer absence of Git effects from a missing task receipt. |
| Recovery | Exact task/run recovery target and deterministic result identity. Reuse recovery evidence/ownership guards; never replay execution. |
| Scope draft/confirmation | Exact understanding request plus understanding namespace. Keep scope and task revisions distinct. |
| Planner start/revision | Stable generation identity and draft provenance. Stream status and completed draft are generation events. Persist completed draft separately; approval/publication require subsequent commands. |
| Automatic Architect revision | Link generation to the triggering canonical review/route receipt and original operation. Preserve human-risk precedence and existing no-restart-inference rule. |
| Retry | Original immutable request and origin, plus retry-attempt event. Never generate a replacement correlation for an exact retry. |

## Schema and authority boundaries

Use a versioned, bounded presentation structure separate from `Message` and model
request construction. Stable type-qualified keys distinguish command, generation,
receipt and retry-attempt records. Namespace identity must bind workspace, mission
and conversation; durable session identity must survive selection changes.

Persist only typed intent data necessary to reproduce an explicit retry and reconcile
an acknowledgment. Reuse canonical field limits: correlations at most 160 bytes,
task count 256, task receipts 4096, bounded policies and Plan payloads. Define explicit
aggregate event/intent byte and count limits within the existing 12-MiB conversation
budget. A single Plan may already approach 128 KiB; do not multiply copies per phase.
Reserve capacity for terminal phases before admitting an operation, and refuse before
dispatch if intent or terminal publication cannot fit. Do not silently drop history.

The new conversation schema must reject origin/intent fields under older versions,
retain exact pre-migration bytes, and reject duplicate/conflicting keys, wrong-domain
receipt references, invalid phase transitions and malformed identities. Historical
messages and observation-only references remain readable without synthesized actors,
causal order or commands. Immutable receipt references must match canonical data
before receiving acknowledged styling; unavailable evidence is shown as unavailable.

Do not persist provider secrets or arbitrary terminal output inside command phases.
Do not add presentation entries to Ollama messages. Raw model text and proposed drafts
never grant file, execution, review or dispatch authority.

## Implementation sequence

1. Complete keyed reading so phase updates and inserted events preserve the reader's
   semantic anchor without forcing selection or scrolling.
2. Introduce validated command intent/phase types and conversation migration tests.
3. Add a two-phase admission API: prepare immutable submission, publish pending intent,
   then dispatch that exact admitted intent after save acknowledgment.
4. Extend task result channels with operation identity and typed acknowledgment/error
   status. Reconcile canonical snapshots without depending on notice text.
5. Wire originating-session rendering and cross-session retry. Keep observation-only
   history distinct from causal command entries.
6. Port each special adapter in the table; ordinary task writes alone do not complete
   command chronology.
7. Verify installed-terminal journeys and the full relevant native suite. Update this
   plan and the orchestration checkpoint with exact remaining gaps and evidence.

## Acceptance regressions

- Pending appears immediately; delayed intent save proves no task write, HTTP request,
  worker claim or Git effect before the matching save acknowledgment.
- Failed intent persistence preserves the pending/refused explanation and newer draft;
  no operation runs. Capacity exhaustion similarly refuses before dispatch.
- Switch conversation while a mutation is pending: its acknowledgment and error stay
  with the original session and never enter inference input.
- Exact replay, conflicting correlation, stale revision, delayed/out-of-order snapshot
  and unrelated successful refresh cannot duplicate or falsely acknowledge commands.
- Crash cuts before intent publication, after publication/before dispatch, after task
  acknowledgment/before conversation checkpoint, and during worker/branch effects all
  restore honestly without automatic replay.
- Worker Start and Finish remain distinct, causally linked phases; pre-claim failure,
  cancellation and uncertain recovery are not reported as completed work.
- Scope and task receipts with equal numeric revisions cannot cross-match. Assignment,
  branch and recovery use their own authoritative matching rules.
- Planner draft completion is never labeled task creation. Plan save binds the exact
  draft; Architect revision preserves route/source provenance and risk precedence.
- Retry from another session retains origin and exact request. Changed input is a new
  operation. Reopening Activity after a failed Plan save preserves its draft.
- Long history, unicode, narrow terminal, streaming, phase growth and restart preserve
  keyed reading position; submitting a new prompt remains visible.
- Migration preserves exact old bytes; malformed/oversized/duplicate records refuse
  unchanged. Historical attribution remains explicitly unrecorded.

## Completion boundary

This plan is fulfilled only when the durable-before-dispatch causal path and all
applicable adapters above operate and are verified. Observation-only task entries,
a command log without receipt binding, or a one-session happy path are not substitutes.


## Implemented prepared-operation checkpoint (2026-09-20)

`console_command.rs` stores immutable typed intent fingerprints, shared reading order, origin boundary and attempt; `command_intent.rs` matches canonical receipt domains. Schema10 migrates exact old bytes and refuses duplicate command IDs across sessions. `Autosave::contains_saved_command` requires a successfully synced exact Pending record including attempt. Main uses an in-memory pending token only; restored records never become dispatch tokens. Explicit `/retry-command SESSION:COMMAND` preserves origin; older saves cannot release newer attempts. Normalized reserved capacity keeps terminal phases writable.

Prepared adapters cover task mutations (including assignment and Plan save), scope mutations, explicit worker Start, branch and recovery. Scope evidence is cached independently from panel visibility. Worker Start proves a claim, not eventual success. Explicit Run commands now resolve their exact later Finish into a separate originating result phase; dispatch without a saved Run intent retains observation-only history. Planner generation/revision/cancellation, automatic dispatch decisions, active-worker cancellation and Wayfinder/workspace events still need dedicated causal adapters. This checkpoint does not satisfy the full completion boundary above.


## Worker lifecycle checkpoint (2026-09-20)

Explicit Run command phases now resolve exact Start and same-run Finish receipts. Current TaskRun id/baseline/inputs and evidence digest must match; task status or lost local ownership cannot substitute. Claim/result remain separate lines under the original command, and cross-session observer duplicates are suppressed. Real-worker tests cover success, failure, cancellation, missing-Finish recovery and restart without replay. Schema11 expands only reading bounds and preserves exact v10 bytes. Remaining typed adapters above still prevent calling the whole plan complete.


## Planner command continuation (2026-09-20)

Explicit `/plan REQUEST`, `/plan-revise REQUEST`, `/architect-revise ID` and `/plan-cancel` now prepare immutable requests through the saved-intent barrier. Revision binds the prior draft digest; cancellation binds the exact live generation and/or retained draft. Planner outcomes remain presentation provenance, never Task receipts. Current draft origin supports restart reconciliation without inference replay; terminal planner outcomes require a new operation to generate again. Conversation schema12 preserves v11 bytes and refuses unsupported provenance in older schemas. Focused65 and full254 tests pass (7 opt-in ignored), with strict Clippy and installed archive acceptance. [Evidence](../Reports/2026-09-20-planner-command-lifecycle.json).

Automatic Architect routing, dispatch decisions, active-worker cancellation and Wayfinder/workspace events still require causal adapters. Full mission formation and production acceptance remain separate unfinished work.


## Controller command checkpoint (2026-09-26)

Explicit dispatch toggles and active-worker cancellation now capture process controller identity before save. Cancellation also captures the exact Start correlation/revision before its receipt arrives; it acknowledges only a request, with an independent canonical Finish phase. Enabling dispatch verifies captured scope asynchronously and rejects changed controller epochs; off, scope holds and shutdown invalidate pending enables. Restoration never reactivates a controller. The terminal header shows current dispatch state separately from historical command outcomes. Conversation schema13 gates control metadata and expands cancellation reading bounds.

Automatic per-worker launch provenance, automatic Architect routing and Wayfinder/workspace event adapters remain unfinished, alongside full mission formation and launch acceptance.

Focused84 and full262 tests pass (7 opt-in ignored), strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-controller-command-lifecycle.json).


## Automatic launch checkpoint (2026-09-26)

Effectful dispatch ticks are removed. Controller selection captures exact source, approval and task revision without starting work; main stages a DispatchRun under its acknowledged enabling command and only releases it after its exact Pending save. Failure stops scheduling; off can withdraw a pending launch. Same-session schema14 parent validation, independent Start/Finish binding, automatic actor and preserved reading/drafts are verified. Focused79/full267 pass (7 ignored), strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-automatic-launch-admission.json). Automatic Architect and Wayfinder/workspace adapters remain open.


## Automatic Architect checkpoint (2026-09-26)

The exact acknowledged architecture review now prepares an inert draft intent that must be saved in its source session before inference. Schema15 validates same-session earlier review and unique inner planner identity, preserves v14 bytes and restores outcomes without replay. Cancellation withdraws unsent tokens; a final state check after model capacity refuses stale queued requests. Draft generation, task adoption and approval remain separate. Focused127 and full274 pass / 7 ignored, strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-automatic-architect-admission.json). Wayfinder/workspace adapters, pre-admission failure history and the broader completion boundary remain open.


## Wayfinder scope admission checkpoint (2026-09-26)

Wayfinder mutations now retain exact requests bound to existing user turns and wait for their saved Pending records. Async preparation remains read-only; queued operations, exact retries, entry races, cancellation and in-flight receipts preserve original ownership. Schema16 preserves v15 and rejects malformed/duplicate bindings; delayed chronology and reply headers use canonical evidence. Focused83 and full286 pass / 7 ignored; strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-wayfinder-command-admission.json). Workspace/mission selection events, pre-admission failure history and broader product requirements remain open.

The next selection slice is specified in [workspace and mission continuity](native-selection-chronology.md), including startup and creation effects before handoff.


## Selection continuity checkpoint

Startup and in-process workspace/mission choices now use saved journal admission before creation, plus exact source and destination history outside model messages. Conversation schema17 preserves v16; failure and restart retain uncertainty without replay. Focused 104 and full 305 pass / 7 ignored; separation guard RED/GREEN, strict Clippy and installed archive pass. [Evidence](../Reports/2026-09-26-selection-continuity.json). Full production requirements remain open.
