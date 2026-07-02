# PRD: Rebuild Mission Control as an Agent Workstation

Status: ready-for-agent
Type: PRD

## Problem Statement

The Mission Commander cannot rely on the current Mission Control App as a working coding-agent interface. The deck is not reliably launchable as a native desktop product, the Agent Console accepts messages without producing an LLM response, and the user cannot see or control agent/subagent work in the way Codex CLI or OpenCode make activity visible.

The current UI also does not match the desired mental model. The Mission Commander wants an IDE-like workstation: agent and subagent workstations visible on the left, selected agent work opened in the center only when needed, and the human controller conversation always available on the right. Instead, the current panels feel old-fashioned, collide with each other, and include buttons or links whose effects are unclear, non-reactive, or not useful.

Existing completed issue slices prove narrower capabilities, such as scoped console history and basic launch behavior, but they do not prove the product-level experience the Mission Commander expects: launch the app, talk to an actual controller agent, watch tool/command/delegation activity, approve risky actions, inspect sessions, and keep operating without panel crashes.

## Solution

Rebuild Mission Control into a native Windows and Ubuntu desktop agent workstation.

The rebuilt app will provide a reliable launch path, packaged backend runtime, actionable startup preflight, real controller-agent conversation loop, policy-gated tool execution, and an IDE-like three-zone layout:

- Left: persistent Workstations list showing agents, subagents, sessions, status, progress, current task, approvals, and last activity.
- Center: Session Workspace that appears only after selecting a workstation/session and shows live plan, tool activity, diffs/files, evidence, terminal output, and review state.
- Right: persistent Controller Console where the Mission Commander talks with Albert, sees streaming assistant responses, handles approvals, cancels/retries turns, and controls conversation scope.

The first controller model will be the configured local frontier model through Ollama. The implementation will keep the Orchestrator authoritative for mission state, permission boundaries, workspace activity, and governance decisions.

## User Stories

1. As a Mission Commander, I want to launch Mission Control on native Windows, so that I can use the app without depending on a source checkout.
2. As a Mission Commander, I want to launch Mission Control on native Ubuntu, so that I can use the app as a proper desktop application.
3. As a Mission Commander, I want startup failures to identify the failing layer, so that I know whether the problem is the backend, Ollama, the model, workspace access, or desktop runtime.
4. As a Mission Commander, I want copyable startup diagnostics, so that I can report or debug launch failures without guessing.
5. As a Mission Commander, I want to select the workspace explicitly, so that Mission Control does not confuse backend installation paths with the coding repo.
6. As a Mission Commander, I want recent workspaces remembered, so that reopening active projects is fast.
7. As a Mission Commander, I want the app to verify Ollama availability, so that agent failures are surfaced before I start working.
8. As a Mission Commander, I want the app to verify required model availability, so that the Controller Console does not silently fail.
9. As a Mission Commander, I want to send a message and receive a streamed assistant response, so that the Agent Console behaves like a real coding-agent controller.
10. As a Mission Commander, I want every submitted message to create a visible turn, so that I can understand whether Albert is queued, thinking, waiting, done, failed, or cancelled.
11. As a Mission Commander, I want to cancel an active turn, so that I can stop a bad or obsolete request.
12. As a Mission Commander, I want to retry a failed or cancelled turn, so that I can recover without retyping the request.
13. As a Mission Commander, I want assistant output to remain durable after streaming finishes, so that I can reconstruct the working conversation later.
14. As a Mission Commander, I want tool calls shown as structured cards, so that I can distinguish reasoning, command execution, file inspection, errors, and results.
15. As a Mission Commander, I want safe read-only work to proceed automatically, so that normal inspection does not require constant approval.
16. As a Mission Commander, I want risky commands to require approval, so that Albert cannot exceed the intended boundary.
17. As a Mission Commander, I want file writes to require approval when policy requires it, so that the UI remains aligned with governance rules.
18. As a Mission Commander, I want delegation and subagent launch requests to be gated, so that no background work starts without an explicit boundary.
19. As a Mission Commander, I want the Controller Console to show pending approvals inline, so that I can approve or reject without hunting through another view.
20. As a Mission Commander, I want approval results to be visible in the turn timeline, so that I know what happened after my decision.
21. As a Mission Commander, I want conversation scope to remain explicit and stable, so that navigation does not silently retarget my next instruction.
22. As a Mission Commander, I want the left Workstations pane to always show active agents and subagents, so that I can maintain operational awareness.
23. As a Mission Commander, I want each workstation to show status and current task, so that I know what every agent is working on.
24. As a Mission Commander, I want agent/subagent work grouped by mission or active context, so that related work is easy to scan.
25. As a Mission Commander, I want approval badges in the Workstations pane, so that blocked agents are obvious.
26. As a Mission Commander, I want selecting a workstation to open the center Session Workspace, so that I can inspect details only when needed.
27. As a Mission Commander, I want the center Session Workspace hidden when nothing is selected, so that the Controller Console has more room.
28. As a Mission Commander, I want to close the center Session Workspace, so that I can return to controller-focused operation.
29. As a Mission Commander, I want the right Controller Console always available, so that I can steer the system while browsing work.
30. As a Mission Commander, I want the center workspace to show a live plan, so that I can judge whether an agent is following the intended path.
31. As a Mission Commander, I want the center workspace to show tool activity, so that I can inspect exactly what the selected agent did.
32. As a Mission Commander, I want the center workspace to show diffs and changed files, so that review is available without leaving the app.
33. As a Mission Commander, I want the center workspace to show terminal output when relevant, so that command results are connected to the session that produced them.
34. As a Mission Commander, I want Shell Terminal activity governed through the same policy model, so that commands cannot bypass agent safety.
35. As a Mission Commander, I want terminal output separated from accepted mission state, so that transient output does not pollute the Activity Journal.
36. As a Mission Commander, I want every button to show a visible result or disabled reason, so that the UI never feels inert.
37. As a Mission Commander, I want internal links to route to app actions, so that clicking navigation elements reliably opens the expected mission, issue, session, or artifact.
38. As a Mission Commander, I want rejected actions to explain why, so that I can correct the request or grant the needed permission.
39. As a Mission Commander, I want panel sizes and selected views to persist, so that the workstation returns to my preferred layout.
40. As a Mission Commander, I want constrained-width behavior to remain usable, so that panels do not crash into each other.
41. As a Mission Commander, I want a refined modern workstation visual style, so that the product feels current and clear rather than decorative or old-fashioned.
42. As a Mission Commander, I want semantic colors and compact badges, so that status is readable without visual noise.
43. As a Mission Commander, I want keyboard navigation and focus states, so that the app is usable without relying entirely on pointer interactions.
44. As a Mission Commander, I want the app to restore after restart, so that ongoing sessions, recent messages, and selected workspace state are not lost.
45. As a Mission Commander, I want reconnect behavior to replay or restore active turn state, so that a desktop refresh does not hide an active agent.
46. As a Frontier Model, I want a stable controller-turn contract, so that responses, tool intents, approvals, and errors can be represented consistently.
47. As a Local Agent, I want delegated work to arrive with explicit boundaries and acceptance criteria, so that I can work without expanding scope.
48. As the Orchestrator, I want all mutations to pass through governed APIs, so that mission state remains authoritative and auditable.
49. As a reviewer, I want evidence packages connected to the selected workstation/session, so that review decisions are based on visible facts.
50. As a future implementer, I want the app split into clear UI and runtime seams, so that panel behavior, agent turns, and launch checks can be tested independently.

## Implementation Decisions

- Primary desktop targets are native Windows and native Ubuntu. WSL/WSLg is a development bridge only and is not a release gate.
- The desktop app will be packaged as a native Tauri application with the backend available as a platform sidecar. Development overrides may point to a source backend, but packaged operation must not require a source checkout.
- Backend installation/runtime location and selected coding workspace are separate concepts. Mission Control opens a selected workspace and persists recent workspaces.
- Startup preflight is a first-class product surface. It checks backend availability, workspace access, model/runtime availability, desktop prerequisites, and writable runtime locations before showing the main workstation.
- The current scoped console history is extended into a real controller-agent runtime. A submitted console message creates a durable controller turn with visible lifecycle state.
- Controller turns expose ordered events for assistant messages, stream chunks, tool calls, approval prompts, delegation proposals, failures, cancellation, and completion.
- The controller runtime uses a provider-neutral streaming boundary. Ollama is the first provider, with the configured local frontier model as the default controller model.
- Tool execution is policy-gated. Safe read-only inspection may run automatically; writes, risky commands, delegation, path grants, and accepted-state changes require explicit approval.
- The Orchestrator remains authoritative for mission state, governance, path grants, command policy, workspaces, evidence, and accepted decisions.
- Raw token streams and raw terminal bytes are transient interaction data. Durable history stores finalized assistant messages, turn summaries, tool metadata, decisions, and meaningful activity records.
- The UI shell becomes an IDE-like three-zone workstation. The left zone is persistent workstations, the center zone is a conditional selected-session workspace, and the right zone is the persistent controller console.
- The Shell Terminal is no longer a permanent competing left-lane panel. It appears as governed command activity inside a turn or selected session.
- The visual direction is “refined workstation”: calmer dark UI, clearer hierarchy, modern typography, semantic accents, compact status badges, less decorative chrome, and less all-caps command-deck styling.
- Internal navigation and buttons become typed app actions with explicit pending, success, rejected, disabled, and failed states.
- Existing historical console messages should remain viewable as conversation history. They do not need to be backfilled into full controller-turn records.
- Existing completed Command Deck issues remain historical prior art. This PRD supersedes their product-level acceptance where their original criteria were narrower than the desired workstation behavior.

## Testing Decisions

- The highest-value test seam is the Mission Commander desktop journey across the app/bridge/backend boundary: launch, preflight, open workspace, submit message, stream response, show tool activity, gate a risky action, inspect a workstation, close/reopen, and restore state.
- Backend tests should verify controller-turn lifecycle behavior through public commands or service-level APIs, not private implementation details.
- Provider tests should use a deterministic streaming fake for normal CI and a separately marked local Ollama smoke for machine-level verification.
- UI tests should verify visible behavior: pane opening/closing, console persistence, workstation selection, button feedback, disabled reasons, approval prompts, and responsive layout.
- Launch tests should cover native Windows and native Ubuntu packaging. WSL may be documented and smoke-tested, but it is not a required release target.
- Governance tests should prove that safe reads can proceed, while writes, risky commands, delegation, path grants, and accepted-state changes require approval.
- Activity Journal tests should prove that durable records include meaningful events and exclude raw transient token or terminal streams.
- Prior art in the codebase includes Python unit tests for backend behavior, frontend tests for Mission Control UI interactions, Rust/Tauri bridge tests, and launch-focused tests from the existing Command Deck work.

## Out of Scope

- Replacing the Orchestrator or changing the core mission-governance authority model.
- Making WSL/WSLg a fully supported desktop release platform.
- Adding remote/cloud model providers beyond the provider-neutral seam needed for local Ollama.
- Building a full source-code editor inside Mission Control.
- Allowing unrestricted terminal execution from the UI.
- Auto-launching delegation or subagents without policy approval.
- Migrating every historical issue slice into the new PRD format.
- Designing final branding, marketing copy, or public website assets.

## Further Notes

- The current app can pass existing unit and frontend tests while still failing the desired product experience, because the tests mostly verify contracts and static interaction surfaces rather than a complete interactive controller-agent journey.
- The completed Agent Console work implemented scoped persistent messages, not a streaming LLM loop. This PRD treats the missing loop as a core product gap.
- The completed launch work improved command behavior, but this PRD requires packaged native app behavior with actionable startup preflight and backend/runtime decoupling.
- After this PRD is accepted, it should be split into independently grabbable issue slices. The likely slices are launch packaging/preflight, controller-turn runtime, policy-gated tool lifecycle, workstation shell layout, visual redesign/design-system pass, and end-to-end verification.
