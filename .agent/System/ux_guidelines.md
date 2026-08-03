# UX Guidelines — Alfredo Workstation

**Last Updated:** 2026-07-30
**Implementation reports:** [Alfredo one-shot workstation correction](../Reports/2026-07-11-alfredo-one-shot-workstation.md), [install and Queue acceptance correction](../Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md), [Wayfinder Shared Understanding Gate](../Reports/2026-08-03-wayfinder-shared-understanding-gate.md)

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
- Every accepted Workstation action turn carries the correlation returned by the typed acknowledgement and displays `Receipt <id> · workstation-action-acknowledged`. If it differs from the requested correlation, render a failed non-receipt turn and no success claim.
- Automatic approval is allowed only after the canonical proposal exactly matches the originating Mission, Conversation Scope, user goal, acceptance criteria, allowed paths, command policy, proposed eligible worker, and message identity. Eligible workers must explicitly report `assignable: true`, `delegate_only: false`, and `requires_approval: false`, be local/non-cloud and available, and use worker/local-agent routing; missing authority metadata fails closed for both automatic and manual assignment. The chronology must still show the proposal, approval, and queued session as distinct acknowledged events. Gated, delegate-only, unavailable, unsafe, or mismatched work pauses for manual handling.
- Never describe a controller proposal as acknowledged execution. Only a matching canonical Orchestrator receipt may show a task, launch, permission, file/mutation, review, or accepted-state transition as accepted. See the [false-success diagnosis](../Reports/2026-07-24-workspace-selection-false-success-diagnosis.md) and [receipt-binding implementation](../Reports/2026-08-02-conversational-action-receipts.md).

## Mission Work Interaction

- Project Local Agent cards from canonical Mission/session state and qualify card, action, transcript, and continuity identities with Mission id.
- Show task, model, role, status, progress, attention, touched files, latest command/test, evidence, and next action in human language.
- Render last activity from a validated canonical timestamp and show `Not recorded` when none exists; never synthesize time from revision numbers or evidence counts.
- Keep blocked, waiting, failed, and active work discoverable; completed work may be grouped separately.
- Treat cancelled/canceled Local Agent work as terminal unsuccessful state, never as done or completed work.
- Approve, launch, retry, cancel, assignment, evidence review, repair, escalation, and queue decisions use typed expected-revision requests and create visible human/orchestrator turns.
- A persisted repairable review from Review Workspace, TUI, CLI, or legacy state exposes exactly one `Launch repair` action after reload. The action reuses the review reason and inherited authority; ordinary failed work without a repairable review still requires an explicit retry reason.
- Routine selection, expansion, filtering, sorting, pinning, and diff navigation remain local UI state and do not pollute durable history.
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
