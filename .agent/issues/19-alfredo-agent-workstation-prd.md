# Alfredo Agent Workstation Product Requirements Document

Status: ready-for-human
Type: PRD

## Problem Statement

The Mission Commander wants Alfredo to feel installable and usable like modern coding-agent tools such as Claude Code, Codex, and OpenCode, while still providing stronger visibility into orchestrated agent and subagent work. The current direction for Mission Control over-emphasizes a standalone desktop app and does not clearly make npm installation and CLI launch the public product entrypoint.

The Mission Commander also does not want subagent activity to disappear into chat history. When prompting a coding agent, the user needs to constantly see what agents and subagents are doing, what they are waiting on, which model is involved, what files or commands are affected, and which decisions need human approval. This status should be human-readable, visual, persistent, and interactive rather than buried in raw logs, transient terminal bytes, or a long transcript.

The existing codebase has many pieces of this system: a Python Orchestrator, CLI commands, a Tauri and React desktop surface, model registry, governed Shell Terminal behavior, Workspace Queue decisions, Activity Journal, Review Workspace, and prior prototype work. The product direction now needs to be restated around Alfredo: an npm-installed command that launches a prompt-dominant desktop workstation by default, with headless CLI/TUI operation as a reduced fallback.

## Solution

Build Alfredo as an npm-installed coding-agent product whose default command opens a desktop workstation. The workstation combines a Claude Code/Codex/OpenCode-like prompt pane with a persistent interactive Agent Workstations side pane.

The prompt pane is the dominant working surface. It contains durable user prompts, assistant/controller responses, consequential workstation actions, and their outcomes. The active prompt composer stays pinned at the bottom, with a compact status line near the composer so the product still feels like a coding-agent terminal.

The Agent Workstations side pane is persistent and does not scroll away with chat history. It shows compact live workstation cards for agents and subagents. Each card presents human-readable operational state: status, current task, model, role, progress, approvals, files touched, latest command or test summary, last activity, and next action. Expanding a card reveals deeper operational detail such as tool calls, diffs, evidence, terminal excerpts, and review controls.

The side pane is interactive. Consequential actions such as approving a launch, retrying failed work, cancelling an active session, granting path access, accepting evidence, requesting repair, escalating review, changing model assignment, or confirming a Mission Draft create visible prompt/orchestrator turns. Routine navigation such as selecting a session, opening a diff, filtering cards, expanding details, or viewing evidence remains side-pane-only and does not pollute durable prompt transcript history.

All consequential prompt and side-pane actions submit typed Orchestrator requests. The Orchestrator remains authoritative for mission state, lifecycle validation, command policy, path grants, permission boundaries, accepted decisions, Evidence Packages, Activity Journal records, and session state. The UI never treats a click or prompt as accepted state until the Orchestrator acknowledges it.

The public product name becomes Alfredo. The public CLI command and npm bin are Alfredo. The desktop app title and user-facing documentation use Alfredo. Internal Python package names, existing runtime paths, existing `.albert` configuration, and compatibility aliases may remain temporarily until a dedicated migration slice safely renames them.

## User Stories

1. As a Mission Commander, I want to install Alfredo through npm, so that the tool is available through a familiar coding-agent installation path.
2. As a Mission Commander, I want running `alfredo` to open the full desktop workstation, so that the best experience is the default.
3. As a Mission Commander, I want `alfredo --agent <agent-id>` to open the workstation with a selected controller or model, so that I can start with the model I intend to use.
4. As a Mission Commander, I want `alfredo workstation --agent <agent-id>` to explicitly open the desktop workstation, so that scripts and docs can name the rich UI mode clearly.
5. As a Mission Commander, I want `alfredo run --agent <agent-id> "<prompt>"` to run terminal-only model work, so that I can use Alfredo over SSH, in automation, or when a desktop UI is unavailable.
6. As a Mission Commander, I want `alfredo review --agent <agent-id>` to run review-oriented model work, so that review can be driven from the CLI when needed.
7. As a Mission Commander, I want `alfredo review <session-id> --agent <agent-id>` to review a specific session, so that targeted review remains scriptable.
8. As a Mission Commander, I want `alfredo agents` to list configured agents and models, so that I can choose a valid model identity.
9. As a Mission Commander, I want terminal-only CLI/TUI operation to remain available as a reduced fallback, so that Alfredo is still useful without the desktop workstation.
10. As a Mission Commander, I want the desktop workstation to feel prompt-first, so that prompting a coding agent remains the primary activity.
11. As a Mission Commander, I want the prompt composer pinned at the bottom, so that I can keep steering the system while reading prior turns.
12. As a Mission Commander, I want a compact terminal status line near the prompt, so that I can see connection, model, scope, and active execution state without leaving the prompt.
13. As a Mission Commander, I want the Agent Workstations side pane to remain visible while I prompt, so that subagent work never disappears into chat scrollback.
14. As a Mission Commander, I want compact workstation cards for active agents and subagents, so that I can understand multiple streams of work at a glance.
15. As a Mission Commander, I want each workstation card to show agent or subagent name, so that responsibility is clear.
16. As a Mission Commander, I want each workstation card to show model and role, so that I know which model is acting and why.
17. As a Mission Commander, I want each workstation card to show current task, so that I can understand the active work in human language.
18. As a Mission Commander, I want each workstation card to show status, so that I can distinguish thinking, running, waiting approval, blocked, reviewing, done, and failed work.
19. As a Mission Commander, I want each workstation card to show last activity, so that stale or stuck work is obvious.
20. As a Mission Commander, I want each workstation card to show progress or phase, so that long-running work is legible without raw logs.
21. As a Mission Commander, I want each workstation card to show approval badges, so that blocked work needing my decision rises to the surface.
22. As a Mission Commander, I want each workstation card to show files touched count, so that I can judge the blast radius quickly.
23. As a Mission Commander, I want each workstation card to show latest command or test summary, so that I can see meaningful execution state without reading terminal bytes.
24. As a Mission Commander, I want each workstation card to show the next action, so that I know how to move the work forward.
25. As a Mission Commander, I want blocked and waiting-approval cards to float toward the top, so that important decisions are not buried.
26. As a Mission Commander, I want active work cards grouped separately from done or historical sessions, so that the side pane stays focused on live supervision.
27. As a Mission Commander, I want optional mission or scope grouping, so that related agent work can be scanned together when many sessions exist.
28. As a Mission Commander, I want to expand a workstation card, so that I can inspect tool calls, commands, files, diffs, evidence, terminal excerpts, and review state.
29. As a Mission Commander, I want opening a diff or selecting a session to remain local UI navigation, so that the prompt transcript remains readable.
30. As a Mission Commander, I want filtering, sorting, expanding, collapsing, or pinning workstation cards to avoid transcript entries, so that routine UI navigation does not become noise.
31. As a Mission Commander, I want consequential side-pane actions to create visible prompt turns, so that I can see my operational intent in the same place where I prompt the controller.
32. As a Mission Commander, I want consequential side-pane actions to show live Orchestrator/controller reaction in the prompt pane, so that I can understand validation, rejection, launch, retry, or approval flow as it happens.
33. As a Mission Commander, I want approve, reject, and defer actions to create visible prompt/orchestrator turns, so that governance decisions are reconstructable.
34. As a Mission Commander, I want subagent launch actions to create visible prompt/orchestrator turns, so that new execution never starts invisibly.
35. As a Mission Commander, I want cancel actions to create visible prompt/orchestrator turns, so that stopping work is explicit and auditable.
36. As a Mission Commander, I want retry actions to create visible prompt/orchestrator turns, so that recovery from failure is visible.
37. As a Mission Commander, I want path grant approvals and denials to create visible prompt/orchestrator turns, so that filesystem authority changes are explicit.
38. As a Mission Commander, I want evidence acceptance to create visible prompt/orchestrator turns, so that PR-ready state cannot appear without a visible decision.
39. As a Mission Commander, I want repair requests to create visible prompt/orchestrator turns, so that deficient work receives an explicit reason.
40. As a Mission Commander, I want human-review escalations to create visible prompt/orchestrator turns, so that risky or ambiguous work is not silently routed.
41. As a Mission Commander, I want model assignment changes to create visible prompt/orchestrator turns, so that changing execution responsibility is visible.
42. As a Mission Commander, I want Mission Draft confirmation to create a visible prompt/orchestrator turn, so that proposed work does not become accepted mission state silently.
43. As a Mission Commander, I want Conversation Scope changes that affect the next model turn to be visible, so that I know what the next prompt will target.
44. As a Mission Commander, I want every consequential workstation action to submit a typed action rather than a brittle natural-language prompt, so that governance can validate the action reliably.
45. As a Mission Commander, I want the visible prompt turn for a workstation action to be human-readable, so that typed governance does not make the product feel opaque.
46. As a Mission Commander, I want prompt-entered requests and side-pane actions to use the same Orchestrator governance path, so that neither surface can bypass the other.
47. As a Mission Commander, I want the UI to show pending, accepted, rejected, failed, stale, and disabled states, so that clicks never feel inert or falsely successful.
48. As a Mission Commander, I want stale side-pane actions to explain the current state and recovery path, so that I can act safely after concurrent updates.
49. As a Mission Commander, I want the durable prompt transcript to contain user prompts, assistant/controller responses, consequential workstation actions, and outcomes, so that it remains readable and reconstructable.
50. As a Mission Commander, I want continuous telemetry to stay in workstation cards and structured activity/evidence records, so that the transcript does not become a raw operations log.
51. As a Mission Commander, I want raw token streams and raw terminal bytes to remain transient unless summarized as evidence or meaningful activity, so that durable records stay useful.
52. As a Mission Commander, I want the Activity Journal to preserve meaningful attributed actions, so that auditability does not depend on replaying UI animations.
53. As a Mission Commander, I want Evidence Packages linked from workstation cards, so that review is grounded in concrete artifacts.
54. As a Mission Commander, I want terminal excerpts shown in expanded cards only when relevant, so that command context is available without overwhelming the side pane.
55. As a Mission Commander, I want the selected controller/model visible near the prompt composer, so that I know which model will answer the next prompt.
56. As a Mission Commander, I want each subagent card to show its actual model, so that delegated work does not hide model provenance.
57. As a Mission Commander, I want model selection to be explicit in CLI commands, so that scripts remain predictable.
58. As a Mission Commander, I want global `--agent` to apply to default workstation launch, so that `alfredo --agent <agent-id>` remains convenient.
59. As a Mission Commander, I want per-command `--agent` to be the canonical scripting form, so that headless commands are unambiguous.
60. As a Mission Commander, I want the public product name to be Alfredo, so that the product is no longer branded as Albert.
61. As a Mission Commander, I want existing internal names to remain compatible temporarily, so that the rename does not destabilize the workstation implementation.
62. As a Mission Commander, I want the old public `albert` command to be available as a deprecated alias for one compatibility window if practical, so that existing usage does not break suddenly.
63. As a Mission Commander, I want startup preflight to identify missing runtime layers, so that installation, backend, workspace, Ollama, model, and desktop failures are actionable.
64. As a Mission Commander, I want selected workspace and backend installation location to remain separate, so that Alfredo does not confuse the product install with the coding repo.
65. As a Mission Commander, I want recent workspaces restored, so that returning to active coding work is fast.
66. As a Mission Commander, I want active sessions, approvals, evidence links, selected workspace, card state, and prompt transcript to restore after restart, so that work continuity survives app relaunch.
67. As a Mission Commander, I want ephemeral streaming animation and raw transient output to avoid exact replay requirements, so that persistence focuses on accepted and meaningful state.
68. As a Mission Commander, I want the desktop UI to remain usable at constrained widths, so that the prompt and critical workstation decisions remain accessible.
69. As a keyboard user, I want prompt, card navigation, expansion, approvals, review actions, and diff opening to be keyboard-operable, so that the workstation does not require a pointer.
70. As a screen-reader user, I want semantic regions, card labels, live status announcements, and clear action names, so that agent activity is understandable.
71. As a low-vision user, I want sufficient contrast, visible focus, compact badges, and clear grouping, so that dense operational state remains readable.
72. As a motion-sensitive user, I want reduced-motion preferences respected, so that live status changes do not become distracting or harmful.
73. As a Frontier Model, I want prompt turns and workstation actions represented through stable typed contracts, so that model responses, tool intents, approvals, errors, and outcomes can be rendered consistently.
74. As a Local Agent, I want delegated work to arrive with explicit scope, permissions, acceptance criteria, and evidence expectations, so that I can work without expanding boundaries.
75. As the Orchestrator, I want all accepted mutations to pass through governed APIs with expected revisions, so that mission state remains authoritative and auditable.
76. As a reviewer, I want card-linked Evidence Packages and visible review actions, so that acceptance decisions are based on visible facts rather than hidden logs.
77. As a future implementer, I want this PRD split into independently grabbable slices, so that packaging, workstation layout, event contracts, governance, rename, and verification can be delivered safely.

## Implementation Decisions

- Alfredo is the public product name for the new direction. The public CLI command, npm bin, desktop title, user-facing docs, and PRD language use Alfredo.
- Internal compatibility aliases may remain during the first workstation implementation. Existing Python package names, existing runtime directories, existing `.albert` agent configuration, and old command names do not need to be renamed in the first slice.
- The public npm installation path is the primary distribution contract. Native desktop packaging may exist, but npm installation and CLI launch are the user-facing entrypoint.
- Running `alfredo` with no subcommand opens the desktop workstation by default.
- `alfredo --agent <agent-id>` opens the workstation with that controller or model preselected.
- `alfredo workstation --agent <agent-id>` is the explicit desktop launch command.
- `alfredo run --agent <agent-id> "<prompt>"`, `alfredo review --agent <agent-id>`, `alfredo review <session-id> --agent <agent-id>`, and `alfredo agents` define the first public headless command grammar.
- Per-command `--agent` is the canonical grammar for headless and scripted commands. Global `--agent` applies only to default workstation launch.
- The default desktop experience is prompt-dominant. The prompt pane is the primary surface; the Agent Workstations pane is persistent, interactive, and secondary in width and hierarchy.
- The accepted prototype direction is compact live workstation cards rather than a dense table as the primary side-pane model.
- The table/list model may remain useful later for historical or high-volume session browsing, but the first production workstation should optimize live supervision through cards.
- Workstation cards are fed by typed Orchestrator/session events and canonical snapshots, not by frontend-invented state.
- Consequential side-pane actions submit typed Orchestrator actions containing action type, actor, target identity, expected revision, and a reason when required.
- Consequential side-pane actions also create visible human-readable prompt/orchestrator turns so the user can watch the controller react in real time.
- The visible turn text is presentation, not the authority mechanism. The typed action is the authority mechanism.
- Routine side-pane navigation remains local UI state and does not create durable transcript entries.
- Durable prompt transcript includes user prompts, controller responses, consequential workstation actions, and meaningful outcomes.
- Continuous operational telemetry remains in workstation cards, live event streams, Activity Journal summaries, and Evidence Packages rather than being dumped into prompt history.
- Raw token streams and raw terminal bytes remain transient interaction data unless summarized as a meaningful action, evidence excerpt, or review artifact.
- Prompt-entered requests and side-pane actions share the same Orchestrator governance model. The prompt cannot bypass side-pane governance, and the side pane cannot bypass prompt governance.
- The Orchestrator remains authoritative for workspace state, mission state, command policy, path grants, lifecycle transitions, review state, evidence validation, Activity Journal records, and accepted decisions.
- The UI may show pending intent immediately, but accepted state is shown only after Orchestrator acknowledgement.
- Stale, rejected, failed, disabled, pending, and accepted states are first-class UI states.
- Workstation cards prioritize blocked, waiting-approval, active, review-ready, failed, and done groups, with blocked and waiting-approval work surfaced first.
- Cards expose at minimum: status, current task, model, role, last activity, approval blockers, files touched count, latest command/test summary, and next action.
- Expanded cards expose tool activity, files and diffs, Evidence Package links, terminal excerpts, review state, and available governed actions.
- The selected controller/model is visible near the prompt composer. Each subagent card shows the actual model used for that work.
- Startup preflight remains a product surface and should distinguish npm install/runtime, desktop shell, backend process, workspace access, Ollama availability, required model availability, and writable runtime locations.
- Desktop app runtime location and selected coding workspace remain separate concepts.
- The Tauri and React desktop shell, long-running backend, canonical snapshots, batched updates, Activity Journal, and Ollama-first provider-neutral model adapter ADRs remain valid. This PRD changes the product framing, public naming, command grammar, and layout direction rather than rejecting those architectural foundations.
- The previous three-zone layout draft is superseded by a prompt-dominant two-zone workstation: main prompt pane plus persistent Agent Workstations side pane, with in-app expanded detail for selected cards.

## Testing Decisions

- Good tests verify externally visible behavior, accepted Orchestrator state, persisted records, and governed boundaries. They should avoid asserting private implementation details, CSS internals, or mock-only component structure.
- The highest-value release seam is the full desktop-to-backend journey: install/launch command intent, startup preflight, workspace selection, prompt display, workstation card projection, consequential side-pane action, visible prompt/orchestrator turn, Orchestrator acknowledgement, card state update, Activity Journal record, and restart restore.
- CLI tests should verify the public Alfredo grammar for default workstation launch, explicit workstation launch, headless run, review, session review, agent listing, and `--agent` handling.
- Compatibility tests should verify that staged internal Albert compatibility still works where intentionally preserved.
- Backend contract tests should verify typed consequential actions with expected revisions, actor, target identity, and reason where required.
- Governance tests should prove that launch, retry, cancel, approval, rejection, path grant, review, repair, escalation, model assignment, and Mission Draft confirmation cannot bypass Orchestrator validation.
- UI interaction tests should verify that consequential workstation actions append visible prompt/orchestrator turns while routine side-pane navigation does not.
- UI projection tests should verify workstation cards render from canonical session/event state and do not invent accepted state before acknowledgement.
- Transcript tests should verify durable prompt history contains user prompts, assistant/controller responses, consequential side-pane actions, and outcomes, while selection, filtering, diff opening, card expansion, and raw telemetry do not become prompt transcript entries.
- Activity Journal tests should verify meaningful attributed actions are recorded separately from raw token streams and raw terminal bytes.
- Persistence tests should restart the app/backend and verify selected workspace, prompt transcript, active sessions, approvals, evidence links, card state, and recent workspaces restore.
- Provider/model tests should use provider-neutral fake streaming for CI and a separately marked Ollama smoke where local machine verification is appropriate.
- Accessibility tests should cover prompt operation, card navigation, card expansion, approval/review actions, visible focus, screen-reader labels/status, reduced motion, and constrained width behavior.
- Responsive tests should verify the prompt remains primary and critical workstation decisions remain reachable when width is constrained.
- Prior art exists in current Python unit tests for Orchestrator behavior, frontend tests for Mission Control interactions, Rust/Tauri bridge tests, launch-focused tests, Activity Journal tests, Workspace Queue tests, Shell Terminal governance tests, and the throwaway Alfredo workstation prototype.

## Out of Scope

- A full internal repo-wide rename from Albert to Alfredo in the first workstation slice.
- Removing existing `albert_mvp`, `.albert`, old runtime paths, old tracker paths, or internal compatibility aliases immediately.
- Making the dense table/list the primary live workstation interaction model.
- Building a full source-code editor inside Alfredo.
- Allowing prompt text or side-pane clicks to bypass Orchestrator governance.
- Allowing unrestricted terminal execution from the desktop UI.
- Auto-launching subagents or delegation without explicit governed approval where policy requires it.
- Promoting raw token streams or raw terminal bytes into durable prompt transcript history.
- Implementing multiple cloud model providers in the first slice.
- Automatic merge to main or model-approved final merge.
- Final public branding, marketing website, iconography, or marketplace copy.
- Pixel-perfect production reuse of the throwaway prototype.

## Further Notes

- This PRD supersedes the product framing in the earlier “Rebuild Mission Control as an Agent Workstation” draft where that draft treated the native desktop app as the main deliverable independent of npm CLI distribution.
- The accepted visual prototype direction is prompt-dominant desktop layout with compact live workstation cards. The prototype is interaction evidence only, not production architecture.
- The first production slice should be narrow: prove the prompt-dominant shell, live card projection, and one consequential side-pane action that creates a visible prompt/orchestrator turn and updates from acknowledged state.
- A later migration slice should plan the full Albert-to-Alfredo internal rename, including package names, runtime paths, identifiers, docs, branch prefixes, and compatibility/deprecation policy.
- After this PRD is accepted, split it into independently grabbable Issue Slices for npm CLI packaging, public command grammar, desktop shell layout, workstation event contract, card projection, consequential action transcript integration, governance coverage, persistence/restart behavior, accessibility/responsive validation, and staged rename work.

## Comments

- 2026-07-13: All AFK implementation slices, including ticket 22, are complete. The current-build Chromium geometry suite passes 4/4 after its tablet-width overflow exposed and drove a real grid-track fix. Ticket 20 remains `ready-for-human` pending the repository-visibility/provenance decision, authorized publication, a fresh public-registry install/PATH/headless-GUI smoke, and one real-display launch; ticket 28 remains `ready-for-human` for independent assistive-technology and visual review. The PRD is therefore `ready-for-human`, not complete.
