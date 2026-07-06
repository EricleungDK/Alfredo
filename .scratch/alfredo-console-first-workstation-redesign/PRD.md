Status: ready-for-agent

# Alfredo Console-First Workstation Redesign Product Requirements Document

## Problem Statement

The current Alfredo workstation UI does not match the intended coding-agent workflow. Workstations are presented as a tabbed mode in the right-side pane, which makes active Local Agent work feel hidden or secondary. The left prompt pane is useful mainly as Agent Console history rather than as the primary working terminal where the Mission Commander steers the coding agent.

The current Shell Terminal UI also feels like a scattered command-line runner. It exposes command entry, command history, requested paths, access level controls, and Additional Path Grant creation as standing GUI surfaces. That is not the desired experience. Alfredo should feel closer to Codex, Claude Code, and other coding-agent terminals: the Mission Commander talks to the agent in one continuous console, sees meaningful execution and approval turns inline, and grants extra authority only when Alfredo asks for it at the moment it is needed.

At the same time, Alfredo must preserve its stronger orchestration model. Active agent and subagent work must not disappear into chat scrollback. The Mission Commander needs a persistent side pane that shows which workstations are actively working and what Issue Slices they are working on. The side pane also needs an Issue Assignment Board so the Mission Commander can see which Issue Slices are already assigned, which are unassigned, and what state each issue is in.

This redesign should be picked up after `.agent/issues/29-add-alfredo-release-seam-verification.md` is implemented and accepted. It should not destabilize task 29's release seam. Instead, it should refine the product surface once the full launch-to-restart verification path exists.

## Solution

Redesign Alfredo around a console-first workstation.

The main pane becomes a Codex/Claude-style Agent Console terminal. It is the dominant working surface for natural-language steering, command-like requests, assistant/controller responses, governed command execution summaries, approval prompts, path-grant requests, consequential workstation actions, and outcomes. The prompt composer remains pinned at the bottom with a compact status line for connection, selected controller/model, Conversation Scope, workspace, and active execution state.

The existing Shell Terminal capability remains governed by the Orchestrator and command policy, but its visible default surface changes. Command execution appears as inline console turns or compact command cards raised by Alfredo. Command history, raw terminal records, requested path entry, access-level selectors, and Additional Path Grant forms are no longer primary GUI panels. They remain available through Activity Journal, selected-turn detail, audit/debug drill-down, or explicit agent requests.

The secondary persistent pane becomes a Mission Work pane with two stacked regions:

1. Active Workstations at the top. This remains the primary live-supervision surface. It uses compact workstation cards for active, blocked, waiting-approval, reviewing, review-ready, failed, and done work. These cards show who is working, which Issue Slice they are working on, current state, model/role, last meaningful activity, blocker or next action, and meaningful command/test summary.
2. Issue Assignment Board below. This is an assignment and coverage matrix, not the primary live workstation model. It shows every relevant Issue Slice with owner, assignment state, lifecycle state, readiness/blocker state, and current workstation/session when present. It makes unassigned ready work and blocked work visible without replacing live workstation cards.

Consequential actions from either the Agent Console or the Mission Work pane continue to submit typed Orchestrator requests. The UI may show pending intent immediately, but accepted state appears only after Orchestrator acknowledgement. Routine local navigation, such as selecting an issue row, selecting a workstation card, expanding details, opening evidence, filtering, or sorting, must not pollute the durable prompt transcript.

This redesign preserves the PRD decision from the Alfredo Agent Workstation direction: compact live workstation cards remain the primary side-pane model. The new matrix is a lower Issue Assignment Board for ownership and issue coverage, not a dense table replacement for live supervision.

## User Stories

1. As a Mission Commander, I want Alfredo to open into a console-first workstation, so that prompting the coding agent is the primary activity.
2. As a Mission Commander, I want the main pane to behave like a coding-agent terminal, so that natural-language steering, command requests, approvals, and outcomes stay in one continuous flow.
3. As a Mission Commander, I want the prompt composer pinned at the bottom, so that I can keep steering Alfredo while reading prior turns.
4. As a Mission Commander, I want the compact status line near the composer, so that I can see connection, selected controller/model, Conversation Scope, workspace, and active execution state without leaving the prompt.
5. As a Mission Commander, I want Shell Terminal execution to appear as inline console turns, so that command work feels like part of the agent interaction rather than a separate scattered command runner.
6. As a Mission Commander, I want command output summarized inline by default, so that raw terminal bytes do not overwhelm the Agent Console.
7. As a Mission Commander, I want full command output available through expansion or detail views, so that I can debug when summaries are not enough.
8. As a Mission Commander, I want Alfredo to ask inline before running a command that needs approval, so that risky execution cannot happen through ambiguous UI.
9. As a Mission Commander, I want Alfredo to ask inline before creating an Additional Path Grant, so that extra filesystem authority is granted only when needed.
10. As a Mission Commander, I want path-grant prompts to include path, access level, duration, reason, and affected action, so that I understand exactly what authority is being requested.
11. As a Mission Commander, I want command history hidden from the default GUI, so that routine operation is not dominated by administrative metadata.
12. As a Mission Commander, I want command history still available through audit/search/detail surfaces, so that reconstruction and debugging remain possible.
13. As a Mission Commander, I want Additional Path Grants hidden as a standing form, so that grants feel like contextual authority decisions rather than manual configuration.
14. As a Mission Commander, I want expired, denied, or active grants visible when inspecting a relevant command, Activity Journal entry, or approval, so that governance remains auditable.
15. As a Mission Commander, I want Active Workstations to stay visible while I prompt, so that Local Agent and subagent activity never disappears into chat scrollback.
16. As a Mission Commander, I want Active Workstations at the top of the side pane, so that currently running or blocked work is the first operational signal I see.
17. As a Mission Commander, I want each active workstation card to show the assigned agent or subagent, so that responsibility is clear.
18. As a Mission Commander, I want each active workstation card to show the Issue Slice being worked on, so that agent activity is tied to mission work.
19. As a Mission Commander, I want each active workstation card to show actual model and role, so that provenance is visible.
20. As a Mission Commander, I want each active workstation card to show current state, so that I can distinguish running, waiting approval, blocked, reviewing, review-ready, failed, done, and idle work.
21. As a Mission Commander, I want each active workstation card to show latest meaningful command or test summary, so that I can understand execution state without reading raw terminal output.
22. As a Mission Commander, I want each active workstation card to show the next action, so that I know how to move work forward.
23. As a Mission Commander, I want blocked and waiting-approval workstation cards to surface above routine running work, so that decisions are not buried.
24. As a Mission Commander, I want workstation cards to remain the primary live-supervision model, so that the redesign does not turn Alfredo into a dense dashboard table.
25. As a Mission Commander, I want an Issue Assignment Board below active workstations, so that I can see issue ownership and gaps without switching modes.
26. As a Mission Commander, I want the Issue Assignment Board to show all relevant Issue Slices, so that unassigned work is visible.
27. As a Mission Commander, I want the Issue Assignment Board to show each issue's owner, so that I can see what has already been assigned.
28. As a Mission Commander, I want the Issue Assignment Board to clearly mark unassigned Issue Slices, so that I can decide what to launch or assign next.
29. As a Mission Commander, I want the Issue Assignment Board to show issue lifecycle and readiness state, so that I can distinguish ready, blocked, active, review-ready, complete, and failed work.
30. As a Mission Commander, I want the Issue Assignment Board to link active issues to their current workstation/session, so that I can move from issue coverage to live agent detail.
31. As a Mission Commander, I want selecting an issue row to focus local side-pane detail without changing Conversation Scope automatically, so that browsing cannot silently retarget my next prompt.
32. As a Mission Commander, I want selecting an issue row to offer an explicit scope-change action when useful, so that targeting the next prompt remains deliberate.
33. As a Mission Commander, I want unassigned ready issues to offer a governed launch or assignment action only when the Orchestrator says that action is valid, so that the UI cannot bypass launch boundaries.
34. As a Mission Commander, I want assigned issues to show their current agent or queued assignment, so that I do not accidentally assign duplicate work.
35. As a Mission Commander, I want blocked issues to show blocker summaries, so that I can understand why they are not launchable.
36. As a Mission Commander, I want issue assignment actions to create visible prompt/orchestrator turns, so that consequential operational changes are reconstructable.
37. As a Mission Commander, I want routine issue-board navigation to stay out of the prompt transcript, so that the Agent Console remains readable.
38. As a Mission Commander, I want the Agent Console and Mission Work pane to share the same Orchestrator governance path, so that neither surface can bypass the other.
39. As a Mission Commander, I want stale issue assignment actions to explain the current state and recovery path, so that concurrent updates do not produce confusing failures.
40. As a Mission Commander, I want disabled issue actions to explain why they are disabled, so that blocked or invalid states are understandable.
41. As a Mission Commander, I want the Activity Journal to record meaningful command, grant, assignment, approval, and outcome events, so that auditability does not depend on replaying the UI.
42. As a Mission Commander, I want raw token streams and terminal bytes excluded from durable prompt history and Activity Journal entries unless summarized as evidence, so that records remain meaningful.
43. As a Mission Commander, I want selected workstation, selected issue, expanded cards, and issue-board focus restored when meaningful, so that continuity survives restart without replaying transient animation.
44. As a Mission Commander, I want the redesigned layout to remain usable after restart, so that task 29's release seam continues to prove launch-to-restore behavior.
45. As a Mission Commander, I want the side pane to stay secondary in width and hierarchy, so that the console remains the dominant surface.
46. As a Mission Commander, I want the issue assignment matrix to be compact but readable, so that many Issue Slices can be scanned without visual clutter.
47. As a Mission Commander, I want the issue assignment matrix to avoid replacing active workstation cards, so that live agent supervision remains card-first.
48. As a Mission Commander, I want responsive behavior to keep the prompt, composer, active workstation decisions, and issue assignment state reachable, so that constrained widths remain usable.
49. As a keyboard user, I want logical focus order through the composer, active workstation cards, and issue assignment rows, so that I can operate the workstation without a pointer.
50. As a screen-reader user, I want semantic regions for Agent Console, Active Workstations, and Issue Assignment Board, so that the new hierarchy is understandable.
51. As a low-vision user, I want readable text, sufficient contrast, visible focus, and stable compact rows, so that the assignment matrix does not become inaccessible.
52. As a motion-sensitive user, I want reduced-motion preferences to disable nonessential live-status animation, so that status changes remain understandable without distracting motion.
53. As a Frontier Reviewer, I want evidence and review state reachable from workstation cards and issue rows, so that review decisions stay grounded in concrete artifacts.
54. As a Local Agent, I want issue assignment state to reflect accepted Orchestrator state, so that I am not shown as assigned before launch or assignment is acknowledged.
55. As the Orchestrator, I want all accepted command, grant, assignment, launch, retry, repair, review, and scope changes to pass through typed requests with expected revisions, so that accepted state remains authoritative.
56. As a future implementer, I want this redesign sequenced after task 29, so that it can build on release-seam verification instead of moving the seam while it is being created.

## Implementation Decisions

- This PRD is a follow-on redesign to be picked up after `.agent/issues/29-add-alfredo-release-seam-verification.md` is implemented and accepted.
- The product surface remains a Tauri and React Mission Control App backed by the long-running local Orchestrator.
- The Orchestrator remains authoritative for mission state, Workspace Session state, command policy, Additional Path Grants, lifecycle transitions, review state, evidence validation, Activity Journal records, and accepted decisions.
- The visible default surface becomes a console-first workstation: main Agent Console terminal plus a persistent Mission Work side pane.
- The main Agent Console is the dominant working surface. It contains user prompts, assistant/controller responses, consequential workstation actions, governed command summaries, approval prompts, path-grant prompts, and outcomes.
- The Shell Terminal is not removed as a governed capability, but the visible default Shell Terminal panel is demoted. Command entry, command history, requested path entry, access-level selectors, and manual Additional Path Grant creation should not be standing primary GUI surfaces.
- Commands may be represented as inline console command cards or turns. These cards should show the command, purpose, working directory when relevant, policy/approval state when relevant, status, summarized output, and a way to inspect full output.
- Additional Path Grants are requested contextually by Alfredo when a blocked command or agent action needs out-of-workspace access. The prompt must include path, access level, duration, reason, and affected action.
- Additional Path Grant creation remains explicit Mission Commander authority. Agents and skills cannot expand grants themselves.
- Command history, grant records, raw metadata, and audit detail remain accessible through Activity Journal, selected command detail, selected approval detail, or debug/audit drill-down.
- The right-side pane becomes a Mission Work pane, not a tab switcher between Workstations and Shell Terminal.
- The top region of the Mission Work pane is Active Workstations. It uses compact live cards and remains the primary live supervision surface.
- Active Workstation cards keep the accepted Alfredo PRD card model: agent/subagent identity, actual model, role, current task, status, phase/progress, last activity, approval blockers, files touched count, latest command/test summary, and next action.
- Blocked and waiting-approval cards sort above routine active work. Active, reviewing, review-ready, failed, done, and historical work remain visually distinct.
- The lower region of the Mission Work pane is the Issue Assignment Board. It is an ownership and coverage matrix for Issue Slices.
- The Issue Assignment Board shows at minimum Issue Slice identity, title or concise label, owner/assigned agent, lifecycle/readiness state, blocker state, and linked workstation/session when present.
- The Issue Assignment Board may use a dense matrix/table treatment because it is not the primary live workstation interaction model.
- The Issue Assignment Board must not replace active workstation cards as the primary live supervision model. This preserves the existing Alfredo PRD decision that a dense table/list is out of scope as the primary live workstation surface.
- Selecting an issue, selecting a card, expanding detail, filtering, sorting, opening a diff, or viewing evidence is local UI navigation and should not append durable prompt transcript turns.
- Consequential actions from the console, workstation cards, or issue board submit typed Orchestrator requests with action type, actor, target identity, expected revision, and reason where required.
- Consequential actions also create visible human-readable prompt/orchestrator turns so the Mission Commander can see what changed and how the Orchestrator reacted.
- The visible turn text is presentation. The typed Orchestrator request remains the authority mechanism.
- The issue board must not silently change Conversation Scope. Scope changes remain explicit and visible near the composer.
- The issue board may offer an explicit "set scope" or equivalent action when the Mission Commander wants the next prompt to target a selected Issue Slice.
- The redesigned side-pane selection state should be persisted only when it is meaningful continuity state. Transient hover, animation, and raw terminal bytes should not be persisted.
- The visual direction should be industrial/utilitarian and dense but organized. The UI should avoid decorative marketing layout, oversized hero composition, or card nesting that makes operational state harder to scan.
- The design system should preserve WCAG AA contrast, visible focus, semantic regions, keyboard reachability, reduced motion behavior, and constrained-width usability.

## Testing Decisions

- The highest-value test seam is the existing release seam created by task 29: launch/startup preflight, workspace selection, prompt display, workstation projection, consequential action, prompt/orchestrator reaction, acknowledgement, card update, Activity Journal record, and restart restore.
- This redesign should extend that seam rather than create a parallel lower-level seam. A good end-to-end test verifies the Mission Commander can see the console-first layout, active workstation cards, issue assignment matrix, a governed action, visible prompt/orchestrator turns, Activity Journal evidence, and restored meaningful state after restart.
- UI tests should verify external behavior through the rendered application with a contract-faithful client or backend fixture, not private React component structure.
- Projection tests should verify the Issue Assignment Board derives owner/state/workstation linkage from canonical mission/session data and does not invent accepted assignment state before Orchestrator acknowledgement.
- Interaction tests should verify routine issue-board navigation does not append prompt transcript turns.
- Governance tests should verify assignment, launch, retry, cancel, path grant, evidence acceptance, repair, review escalation, model assignment, and scope-affecting actions cannot bypass typed Orchestrator validation.
- Transcript tests should verify durable Agent Console history includes prompts, assistant/controller responses, consequential actions, command/grant approval decisions, and outcomes while excluding routine navigation and raw telemetry.
- Activity Journal tests should verify meaningful attributed command, grant, assignment, approval, and outcome records remain separate from transient terminal bytes.
- Persistence tests should verify selected workspace, recent workspaces, selected controller/model, prompt transcript, active sessions, approvals, evidence links, active workstation state, issue-board selection when meaningful, and side-pane state restore after restart.
- Accessibility tests should cover keyboard operation through the composer, active workstation cards, issue assignment rows, approvals, review actions, and detail expansion.
- Screen-reader tests or manual checks should verify the semantic hierarchy of Agent Console, Active Workstations, and Issue Assignment Board.
- Constrained-width tests should verify the prompt remains primary and critical workstation decisions plus issue assignment information remain reachable without overlapping content.
- Prior art includes current frontend interaction tests for the prompt-dominant workstation, workstation projection tests, Shell Terminal governance tests, Activity Journal tests, Workspace Queue tests, backend contract tests, persistence/restart tests planned by task 29, and accessibility/responsive checks from the Alfredo workstation hardening work.

## Out of Scope

- Implementing this redesign before task 29's release-seam verification is accepted.
- Replacing active workstation cards with a dense table as the primary live supervision model.
- Removing Orchestrator command policy, Additional Path Grant governance, expected-revision checks, or Activity Journal records.
- Allowing unrestricted terminal execution from the desktop UI.
- Allowing prompt text, command cards, workstation card actions, or issue-board actions to bypass typed Orchestrator governance.
- Promoting raw token streams or raw terminal bytes into durable prompt transcript history.
- Making issue-row selection silently change Conversation Scope.
- Building a source-code editor inside Alfredo.
- Performing a full internal Albert-to-Alfredo rename.
- Reworking the public CLI grammar, npm entrypoint, model registry, or headless command behavior.
- Replacing the Activity Journal with full event sourcing.
- Creating a separate command-history-first or grant-management-first administrative dashboard as the default GUI.
- Pixel-perfect implementation of the visual companion mockups.

## Further Notes

- The selected visual direction from the design discussion was Option A: a chat-first Agent Console terminal. The accepted refinement was Option B for the side pane: active workstation cards at the top plus a compact Issue Assignment Board below.
- The issue assignment matrix is intentionally framed as coverage and ownership, not as the primary live work surface. This keeps the redesign compatible with the existing Alfredo Agent Workstation PRD.
- The redesign should be picked up after `.agent/issues/29-add-alfredo-release-seam-verification.md`, because task 29 establishes the release seam this PRD should reuse.
- If the future implementation needs Issue Slices, split this PRD into vertical slices that preserve the same order: console-first shell reshape, Mission Work pane structure, Issue Assignment Board projection, contextual command/grant prompts, governance integration, persistence, and accessibility/responsive verification.
