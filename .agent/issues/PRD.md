# Albert Mission Control App Product Requirements Document

Status: ready-for-agent
Date: 2026-06-19

## Problem Statement

The Mission Commander can already operate Albert through CLI/TUI commands, but the current surface does not provide a continuous, coherent desktop workspace for steering conversation, following multiple missions, understanding an Issue Graph, resolving governance decisions, reviewing Evidence Packages, and reconstructing what happened. Operational context is split across commands and records, making it harder to distinguish factual Orchestrator state from model narration and harder to know what needs attention.

The product needs a user-facing Mission Control App over the existing Orchestrator without weakening its authority. The Mission Commander must be able to converse continuously while missions are created, switched, paused, resumed, or run in the background; deliberately target messages through Conversation Scope; inspect work and provenance at increasing levels of detail; and make explicit launch, boundary, review, repair, and escalation decisions.

The app must preserve the product's governance boundaries. Navigation cannot silently retarget conversation, UI state cannot become accepted mission state by itself, questionable actions cannot bypass approval, and Complete cannot be confused with merged. It must recover current state reliably, preserve meaningful attribution in the Activity Journal, expose local-model provenance without coupling the workflow to one provider, and remain usable and accessible across practical desktop window sizes.

## Solution

Build the Mission Control App as a Tauri desktop shell with a React and TypeScript interface connected to Albert's long-running Python Orchestrator backend. The backend supplies an initial canonical state snapshot, then batched state events; the frontend renders state and submits semantic actions while the Orchestrator remains authoritative for validation, accepted state, lifecycle transitions, permissions, execution, evidence, and persistence.

Use the approved Command Deck interaction model. A persistent left lane contains the Agent Console and a distinct Shell Terminal mode. A focused Operations Workspace occupies the right lane and shows one primary operational object at a time: Mission Board, Review Workspace, Workspace Queue, or Activity. A compact mission selector changes the Active Mission without ending the Workspace Session. The Agent Console remains continuous across navigation, and Conversation Scope changes only through a deliberate Mission Commander action.

Mission Board presents the Issue Graph and concise mission progress. Selecting an Issue Slice opens an inspector with its accepted boundary, blockers, lifecycle, attached sessions, role and model provenance, evidence summary, and Working Context sources. Review Workspace is the sole surface for accepting evidence, requesting repair, or escalating to human review. Workspace Queue is the sole decision inbox for Issue Change Proposals, Frontier Confirmations, and Ad Hoc Delegation approvals. Activity provides a searchable chronological Activity Journal.

The first release supports Ollama through provider-neutral model adapters and carries the validated industrial command-deck visual direction into an accessible production interface. It adds the desktop product surface and required state, event, governance, and persistence contracts; it does not replace the existing Orchestrator, automatically merge work, silently alter accepted state, or grant unrestricted terminal execution.

## User Stories

1. As a Mission Commander, I want to open a coding workspace in the Mission Control App, so that I can begin or resume one continuous Workspace Session.
2. As a Mission Commander, I want the app to restore the current Workspace Session state, so that reopening the app does not lose operational continuity.
3. As a Mission Commander, I want the Agent Console to remain available while I navigate operations, so that conversation and inspection form one working relationship.
4. As a Mission Commander, I want Agent Console history to remain continuous across mission switches, so that I do not lose the workspace-level conversation.
5. As a Mission Commander, I want to switch the left lane between Agent Console and Shell Terminal, so that conversation and command execution remain distinct modes.
6. As a Mission Commander, I want Shell Terminal commands governed by Albert's command policy, so that the desktop shell does not create unrestricted execution access.
7. As a Mission Commander, I want command approval requirements shown before execution, so that risky commands cannot run through ambiguous UI actions.
8. As a Mission Commander, I want terminal output kept distinct from domain events, so that transient bytes do not pollute accepted state or the Activity Journal.
9. As a Mission Commander, I want a compact selector for the Active Mission, so that mission switching does not consume the main workspace.
10. As a Mission Commander, I want Background Missions to continue approved bounded work, so that changing the Active Mission does not stop valid sessions.
11. As a Mission Commander, I want a clear indication when Background Missions need attention, so that approvals and clarifications are not missed.
12. As a Mission Commander, I want background attention to route to Workspace Queue, so that governance remains separate from mission progress.
13. As a Mission Commander, I want mission switching to preserve the Workspace Session, so that each mission does not become a disconnected chat.
14. As a Mission Commander, I want the Operations Workspace to show one primary object at a time, so that dense operational data remains understandable.
15. As a Mission Commander, I want navigation choices to remain available without becoming dashboard cards, so that primary work retains visual priority.
16. As a Mission Commander, I want Mission Board to show the Issue Graph, so that I can understand dependencies and execution readiness.
17. As a Mission Commander, I want concise mission progress on Mission Board, so that I can assess status without opening every Issue Slice.
18. As a Mission Commander, I want blockers visible on Issue Slices, so that launch ineligibility is explainable.
19. As a Mission Commander, I want Ready to mean approved, unblocked, and eligible to launch, so that the label has one operational meaning.
20. As a Mission Commander, I want Complete to mean finished, evidence-accepted, and PR-ready, so that completion is not mistaken for merge.
21. As a Mission Commander, I want merged state represented separately from Complete, so that final merge authority remains explicit.
22. As a Mission Commander, I want selecting an Issue Slice to open an inspector, so that the graph remains concise while detail is available on demand.
23. As a Mission Commander, I want the inspector to show the accepted Issue Slice boundary, so that I can compare execution with the approved contract.
24. As a Mission Commander, I want the inspector to show blockers and lifecycle state, so that I can understand the item's current position in the Issue Graph.
25. As a Mission Commander, I want the inspector to show attached Local Agent sessions, so that execution responsibility is traceable.
26. As a Mission Commander, I want the inspector to show Albert role, provider, and model provenance, so that I know which system produced each result.
27. As a Mission Commander, I want the inspector to show an Evidence Package summary, so that I can judge whether deeper review is needed.
28. As a Mission Commander, I want the inspector to show Working Context sources, so that I can understand what informed a model interaction.
29. As a Mission Commander, I want to drill into individual sessions, so that detailed model activity remains available without becoming the primary conversation.
30. As a Mission Commander, I want stale or disconnected session state indicated clearly, so that I do not act on an apparently live but outdated view.
31. As a Mission Commander, I want Review Workspace to list Evidence Packages awaiting a decision, so that review work has a dedicated queue.
32. As a Mission Commander, I want to inspect changed files, diff summaries, commands, test results, risks, and proposed context updates, so that review is evidence-based.
33. As a Mission Commander, I want to accept a reviewed Evidence Package, so that valid work can become Complete and PR-ready.
34. As a Mission Commander, I want to request repair with a reason, so that deficient work returns with actionable feedback.
35. As a Mission Commander, I want to escalate work for human review, so that risky or ambiguous outcomes do not receive model-only acceptance.
36. As a Mission Commander, I want review actions to show their consequence before confirmation, so that consequential state changes are deliberate.
37. As a Mission Commander, I want rejected or repairable work to show its next action, so that recovery paths are clear.
38. As a Mission Commander, I want acceptance to remain distinct from merging, so that the app cannot imply main-branch authorization.
39. As a Frontier Reviewer, I want missing Evidence Package fields to block acceptance, so that unsupported completion claims cannot pass review.
40. As a Frontier Reviewer, I want review limitations displayed when evidence visibility is restricted, so that review confidence is not overstated.
41. As a Mission Commander, I want Workspace Queue to contain every unresolved Issue Change Proposal, so that locked boundary edits are governed consistently.
42. As a Mission Commander, I want Workspace Queue to contain every Frontier Confirmation, so that ambiguous or risky actions wait for explicit authority.
43. As a Mission Commander, I want Workspace Queue to contain every Ad Hoc Delegation approval, so that missionless Local Agent work cannot launch silently.
44. As a Mission Commander, I want Workspace Queue items grouped and filterable by type and mission, so that I can resolve the highest-priority decisions efficiently.
45. As a Mission Commander, I want queue items to explain the requested action, source, consequence, and affected boundary, so that approval is informed.
46. As a Mission Commander, I want to approve, reject, or defer a queue item, so that unresolved governance has explicit outcomes.
47. As a Mission Commander, I want other views to show only compact attention links to Workspace Queue, so that governance is not duplicated.
48. As a Mission Commander, I want a queue decision reflected across all relevant views, so that the interface does not show contradictory states.
49. As a Mission Commander, I want an Issue Change Proposal rather than a direct mutation when a locked field changes, so that accepted mission state cannot drift silently.
50. As a Mission Commander, I want a Frontier Confirmation for ambiguous, risky, irreversible, or launch-boundary-changing actions, so that questionable intent is clarified.
51. As a Mission Commander, I want to request an Ad Hoc Delegation through conversation, so that narrow work can proceed before a mission exists.
52. As a Mission Commander, I want an Ad Hoc Delegation to show scope, acceptance criteria, permissions, and proposed Local Agent, so that its boundaries are reviewable.
53. As a Mission Commander, I want an Ad Hoc Delegation to require approval before launch, so that conversational intent does not automatically execute code changes.
54. As a Mission Commander, I want approved Ad Hoc Delegations to run within the Workspace Session, so that narrow exploratory or corrective work remains visible.
55. As a Mission Commander, I want Ad Hoc Delegation evidence and review to follow bounded-work rules, so that missionless work is not less accountable.
56. As a Mission Commander, I want selected Ad Hoc Delegations to contribute to a Mission Draft, so that useful work can inform a larger plan.
57. As a Mission Commander, I want a Mission Draft to remain proposed until confirmed, so that assembled work does not become accepted mission state silently.
58. As a Mission Commander, I want irrelevant Ad Hoc Delegations excluded from a Mission Draft, so that mission scope remains coherent.
59. As a Mission Commander, I want to set Conversation Scope to the Working directory, so that the next message can address the open repository generally.
60. As a Mission Commander, I want to set Conversation Scope to a Mission, so that Working Context centers on that mission.
61. As a Mission Commander, I want to set Conversation Scope to an Issue Slice, so that the next message can address one bounded work item.
62. As a Mission Commander, I want Conversation Scope shown next to message composition, so that I know where the next message will be interpreted.
63. As a Mission Commander, I want Conversation Scope to remain stable while navigating, so that inspecting another object cannot silently retarget an unfinished message.
64. As a Mission Commander, I want changing the Active Mission to leave Conversation Scope unchanged, so that mission browsing and message intent remain independent.
65. As a Mission Commander, I want selecting an Issue Slice to leave Conversation Scope unchanged, so that inspection does not imply conversational authority.
66. As a Mission Commander, I want deliberate scope changes to be acknowledged visibly, so that retargeting cannot be missed.
67. As a Mission Commander, I want Conversation Scope to influence Working Context without authorizing launch, so that context and execution authority stay separate.
68. As a Mission Commander, I want Conversation Scope to avoid expanding filesystem permissions, so that targeting cannot bypass Additional Path Grants.
69. As a Mission Commander, I want Conversation Scope to avoid mutating locked mission state, so that conversation cannot bypass governance.
70. As a Mission Commander, I want Context Inspector to show the sources assembled into Working Context, so that model inputs are reconstructable.
71. As a Mission Commander, I want to pin eligible context sources, so that important information remains represented in Working Context.
72. As a Mission Commander, I want to exclude eligible context sources, so that irrelevant information does not dominate model input.
73. As a Mission Commander, I want Context Inspector controls to respect Shared Context governance, so that pinning or exclusion cannot rewrite accepted mission truth.
74. As a Mission Commander, I want Working Context to remain bounded, so that long Workspace Sessions do not require replaying the full Agent Console history.
75. As a Mission Commander, I want the full Agent Console history retained for inspection, so that bounded Working Context does not erase conversation history.
76. As a Mission Commander, I want Activity to show meaningful actions chronologically, so that I can reconstruct mission operations.
77. As a Mission Commander, I want Activity Journal entries attributed to Mission Commander, Orchestrator, Frontier Model, or Local Agent, so that responsibility is clear.
78. As a Mission Commander, I want to search the Activity Journal, so that I can locate decisions and state transitions efficiently.
79. As a Mission Commander, I want to filter Activity by mission, actor, action type, and time, so that investigation stays focused.
80. As a Mission Commander, I want Activity entries to link to affected missions, Issue Slices, sessions, and evidence where available, so that chronology leads to detail.
81. As a Mission Commander, I want transient token streams and terminal bytes excluded from the Activity Journal, so that it remains a meaningful audit record.
82. As a Mission Commander, I want Agent Console narration sourced from actual actions and state changes, so that narrative does not masquerade as authority.
83. As a Mission Commander, I want canonical current state reconstructed from snapshots, so that app startup is fast and deterministic.
84. As a Mission Commander, I want meaningful attribution retained separately in the append-only Activity Journal, so that reconstruction does not erase history.
85. As a Mission Commander, I want live updates batched without losing ordering, so that the app stays responsive while state remains coherent.
86. As a Mission Commander, I want reconnecting clients to receive a fresh canonical snapshot, so that missed events do not leave the UI inconsistent.
87. As a Mission Commander, I want semantic actions rejected with actionable errors when state has changed, so that stale UI actions cannot overwrite newer decisions.
88. As a Mission Commander, I want loading, empty, offline, reconnecting, and error states, so that the app remains understandable outside the happy path.
89. As a Mission Commander, I want the current connection and synchronization state visible without dominating the interface, so that data freshness is clear.
90. As a Mission Commander, I want provider and model availability shown separately from mission lifecycle, so that infrastructure outages do not corrupt work state.
91. As a Mission Commander, I want Ollama models available in the first release, so that Albert works with the existing local registry.
92. As a Mission Commander, I want role and model assignments expressed through provider-neutral identities, so that mission records do not depend on Ollama-specific concepts.
93. As a Mission Commander, I want unavailable assigned models identified before launch, so that failures can be resolved deliberately.
94. As a Mission Commander, I want model streaming and availability failures handled without changing accepted mission state, so that provider errors remain operational facts.
95. As a Mission Commander, I want to grant an Additional Path Grant with explicit access and duration, so that required out-of-workspace access is bounded.
96. As a Mission Commander, I want agents and skills unable to expand an Additional Path Grant, so that granted authority cannot escalate itself.
97. As a Mission Commander, I want expired or denied grants to block affected actions clearly, so that boundary failures are explainable.
98. As a Mission Commander, I want the interface to remain usable at constrained desktop widths, so that core controls do not disappear on smaller windows.
99. As a keyboard user, I want all navigation, inspectors, queues, and review decisions operable without a pointer, so that the app is accessible.
100. As a screen-reader user, I want semantic regions, labels, status announcements, and logical focus order, so that live mission state is understandable.
101. As a low-vision user, I want sufficient contrast and visible focus indicators, so that the command-deck aesthetic remains readable.
102. As a motion-sensitive user, I want reduced-motion preferences respected, so that purposeful motion does not create an accessibility barrier.
103. As a Mission Commander, I want dense information to preserve hierarchy through spacing, typography, and restrained accents, so that operational detail does not become visual noise.
104. As a Mission Commander, I want lime and cyan accents used for meaning rather than decoration, so that state and action emphasis remain consistent.
105. As a Mission Commander, I want dangerous and irreversible actions visually distinct from routine actions, so that their risk is apparent.
106. As a Mission Commander, I want state and actions to use domain terms consistently, so that the desktop app and Orchestrator describe the same workflow.
107. As a returning Mission Commander, I want the app to reopen at a sensible operational view without changing Conversation Scope, so that resumption is predictable.
108. As a Mission Commander, I want all accepted actions to survive restart, so that the desktop UI never presents ephemeral approval as durable state.
109. As a Mission Commander, I want failed writes or rejected actions surfaced immediately, so that the UI cannot claim a change the Orchestrator did not accept.
110. As a Mission Commander, I want PR readiness summarized without an automatic merge control, so that the app supports handoff while preserving final human authority.

## Implementation Decisions

- The existing Python Orchestrator remains the sole authority for accepted mission state, lifecycle validation, launch blocking, command policy, permissions, execution, evidence validation, and persistence. The React frontend is a projection and semantic-action client.
- The desktop application uses Tauri with React and TypeScript, as established by the desktop architecture decision. The approved prototype is interaction evidence, not production code or a required component structure.
- The Orchestrator runs as a long-lived local backend. A client connection begins with a versioned canonical state snapshot and continues with ordered, batched state events. Reconnection obtains a new snapshot rather than assuming all prior events were received.
- Frontend actions are semantic domain requests with correlation identity and enough expected-state information for the Orchestrator to reject stale or invalid decisions. UI optimism may improve perceived responsiveness, but no optimistic state is presented as accepted before acknowledgement.
- Current-state snapshots and an append-only Activity Journal are separate persistence concerns. Snapshots reconstruct present state; the journal preserves meaningful attributed actions. Full event sourcing is not introduced.
- High-volume transient data, including model token streams and terminal bytes, is transported and retained only as operationally necessary and is not written as Activity Journal domain events.
- The primary layout is a persistent split surface. The left lane contains Agent Console and distinct Shell Terminal modes; the right lane is the focused Operations Workspace.
- Operations Workspace provides Mission Board, Review Workspace, Workspace Queue, and Activity views. It does not become a comprehensive dashboard, and each view has one primary operational responsibility.
- Active Mission is selected through a compact header control. Switching it does not terminate valid work in Background Missions and does not alter Conversation Scope.
- Conversation Scope has exactly three user-facing target kinds in the initial release: Working directory, Mission, and Issue Slice. It is changed explicitly at the message-composition surface and remains stable across navigation.
- Conversation Scope controls Working Context assembly and interpretation only. It grants no launch authority, permission expansion, Additional Path Grant, locked-state mutation, or approval.
- Context Inspector exposes the sources used to assemble Working Context and allows eligible sources to be pinned or excluded. It cannot directly change governed Shared Context.
- Mission Board centers the Issue Graph, concise progress, blockers, and lifecycle. Selecting an Issue Slice opens an inspector rather than expanding the board into a dashboard.
- The Issue Slice inspector includes accepted boundary, blockers, lifecycle, attached sessions, role/provider/model provenance, Evidence Package summary, and Working Context sources. Individual session detail is available by drill-down.
- Ready means approved, unblocked, and eligible to launch. Complete means execution is finished, the Evidence Package is reviewed and accepted, and the work is PR-ready. Merged remains a separate state and human decision.
- Review Workspace exclusively owns Evidence Package acceptance, repair requests, and human escalation decisions. Review actions are submitted to and validated by the Orchestrator.
- Workspace Queue exclusively owns unresolved Issue Change Proposals, Frontier Confirmations, and Ad Hoc Delegation approvals. Other views link to queue items but do not duplicate their decision controls.
- Changes to a locked Issue Slice boundary, blockers, acceptance criteria, risk, classification, evidence requirements, or other governed fields create an Issue Change Proposal rather than directly mutating accepted state.
- Ambiguous, risky, irreversible, or launch-boundary-changing Frontier Model actions create a Frontier Confirmation and remain pending until resolved by the Mission Commander.
- Ad Hoc Delegation is a bounded, missionless Local Agent work packet in a Workspace Session. It requires explicit approval, acceptance criteria, evidence, and review; it is not an Issue Slice.
- A Mission Draft may organize relevant Ad Hoc Delegations and new work, but becomes accepted mission state only after Mission Commander confirmation.
- The Activity Journal stores meaningful Mission Commander, Orchestrator, Frontier Model, and Local Agent actions with timestamps, attribution, affected entities, and links to available evidence. Search and filters operate on this durable record.
- Agent Console narration is derived from acknowledged actions and authoritative state changes. It must distinguish proposals, pending operations, accepted changes, failures, and model commentary.
- Shell Terminal uses the existing command-policy classifications and Additional Path Grant rules. The desktop surface does not introduce an unrestricted shell or allow agents and skills to broaden granted access.
- The first model provider is Ollama, behind provider-neutral adapter contracts for role assignment, model identity, streaming, availability, and failure reporting. Provider-specific details stay at the adapter and diagnostics boundary.
- Role, provider, and model provenance is part of the state contract and is shown wherever work, evidence, or decisions are attributed to a model session.
- The app must represent disconnected, reconnecting, stale-action, unavailable-model, permission-denied, loading, empty, and backend-error states as first-class interaction states.
- The visual direction is the approved industrial Command Deck: high contrast, restrained lime and cyan semantic accents, dense precise information, purposeful motion, minimal decorative borders, and limited uppercase metadata.
- Accessibility is a release requirement. Core workflows support keyboard operation, semantic regions and controls, managed focus, live status announcements, visible focus, WCAG AA color contrast, zoom/reflow, and reduced motion.
- Responsive behavior prioritizes the Agent Console, current scope, primary Operations Workspace object, and pending governance. Constrained layouts may collapse secondary detail into drawers or sequential views but may not hide required decisions or silently change state.
- Production implementation is delivered later through Issue Slices generated only after this Product Requirements Document is reviewed and approved.

## Testing Decisions

- Good tests verify externally observable behavior at the highest practical seam: what the Mission Commander can see or do, what the backend accepts or rejects, what persists across restart, and what governance prevents. Tests should avoid asserting React component structure, private Python methods, CSS implementation, or event-bus internals.
- The primary React interaction seam covers the application as a user operates it with a real state client or contract-faithful test backend. It verifies mission and view navigation, continuous Agent Console history, explicit and stable Conversation Scope, Issue Graph presentation, Issue Slice inspection, review decisions, Workspace Queue attention routing, keyboard workflows, and constrained layouts.
- Conversation Scope tests navigate and switch Active Missions while a message is being composed, then verify the message target remains unchanged until the Mission Commander explicitly changes it. They also verify scope changes never launch work, expand permission, or mutate locked state.
- Mission Board and inspector tests verify Ready and Complete semantics, blockers, attached sessions, accepted boundaries, Evidence Package summaries, Working Context sources, and role/provider/model provenance using visible behavior rather than component snapshots.
- Review Workspace tests submit accept, repair, and human-escalation decisions and verify only acknowledged Orchestrator outcomes update lifecycle state. Missing evidence, stale actions, and rejected backend decisions must remain visibly unresolved.
- Workspace Queue tests verify Issue Change Proposals, Frontier Confirmations, and Ad Hoc Delegation approvals can be resolved only through the queue, while other views provide attention links without duplicate decision controls.
- Ad Hoc Delegation and Mission Draft tests cover proposal, approval, bounded launch eligibility, evidence and review, selection into a draft, exclusion of irrelevant work, and explicit Mission Commander confirmation before accepted mission creation.
- The backend contract seam starts a client against the long-running Orchestrator and verifies the initial versioned state snapshot, ordered batched events, semantic action acknowledgements, stale-action rejection, disconnect/reconnect recovery, and a fresh snapshot after missed events.
- Contract fixtures must represent states rather than frontend component needs. Compatibility tests protect required domain fields, entity identities, provenance, governance status, and version behavior while allowing additive evolution.
- The governance seam exercises the authoritative backend without the UI. It verifies questionable actions remain Issue Change Proposals or Frontier Confirmations, Ad Hoc Delegations remain unlaunched until approved, locked Issue Slices do not mutate silently, and Conversation Scope has no authorization effect.
- The persistence seam writes accepted actions, restarts the backend, and verifies current-state snapshots reconstruct the same authoritative UI state while Activity Journal entries retain order, attribution, affected entities, and meaningful action details.
- Persistence tests verify transient token and terminal streams are not promoted into Activity Journal entries and that failed or unacknowledged actions do not appear as accepted state.
- The provider seam uses a provider-neutral fake adapter plus an Ollama adapter contract test. It verifies stable role/model provenance, availability, streaming state, and failure behavior without requiring UI tests to know Ollama-specific protocols.
- Provider-failure tests verify unavailable or disconnected models block or fail the relevant operation clearly without changing accepted mission state.
- Shell Terminal boundary tests verify command classifications, approval prompts, denied commands, Additional Path Grant access level and duration, expiration, and the inability of an agent or skill to expand its own grant.
- Accessibility tests combine automated semantic and contrast checks with keyboard journey tests for changing scope, navigating the Issue Graph, opening the inspector, resolving Workspace Queue items, and completing a Review Workspace decision. Reduced-motion and zoom/reflow behavior are explicit acceptance cases.
- Responsive tests exercise representative wide and constrained desktop viewports. They verify that primary content, current Conversation Scope, pending decisions, and actionable errors remain reachable without horizontal loss or state-changing layout side effects.
- Recovery tests cover backend startup failure, temporary disconnect, event lag, stale client action, persistence-write failure, malformed contract data, and partial provider availability. The UI must explain the state and avoid claiming success.
- Existing acceptance coverage in the repository is prior art for Orchestrator lifecycle, approval locking, blockers, delegation gates, model registry provenance, Evidence Package validation, Frontier review, repair, persistence, and PR-ready semantics. New backend tests should extend those public mission behaviors instead of duplicating them behind UI-specific abstractions.
- The approved Command Deck prototype is prior art for interaction responsibilities and expected user journeys only. Production tests may encode its validated behaviors but must not compare against its HTML structure, CSS, or in-memory mock implementation.
- A release candidate is acceptable only when the full Mission Commander journey works across the real desktop-to-backend boundary: restore a Workspace Session, select or create relevant work, deliberately set Conversation Scope, approve governed work, observe bounded execution state, inspect evidence and provenance, record review, and recover the same accepted state after restart.

## Out of Scope

- Automatic merging, merge approval by a model, or bypassing repository branch protection and human final authority.
- Silent mutation of accepted Shared Context, locked Issue Slices, launch boundaries, permissions, or governance decisions.
- Unrestricted terminal execution or a desktop command path that bypasses Albert's command policy.
- OpenAI, Anthropic, or other cloud-provider adapters in the initial release; the provider boundary is included, but Ollama is the first shipped adapter.
- Full event sourcing, replaying every transient output as domain state, or retaining token streams and terminal bytes in the Activity Journal.
- Replacing the Python Orchestrator with frontend-owned workflow logic.
- Treating the approved throwaway prototype as production code, a fixed component architecture, or a pixel-perfect implementation mandate.
- A comprehensive dashboard that duplicates Mission Board, Review Workspace, Workspace Queue, Activity, and inspector responsibilities on one screen.
- Per-model primary chat lanes; model sessions remain available through provenance and drill-down within the unified Agent Console experience.
- Automatic conversion of every Ad Hoc Delegation into an Issue Slice or accepted mission.
- Automatic creation of Issue Slices, production implementation, branch creation, or PR creation as part of this PRD publication.
- Mobile-native applications. Responsive desktop behavior and constrained desktop windows are included.
- Broad visual redesign beyond the validated Command Deck direction.

## Further Notes

- The completed Plan Grill Gate and approved Variant A prototype direction constitute alignment approval for this Product Requirements Document. The documented testing seams were already part of that approved handoff and are recorded above at their highest externally observable boundaries.
- The Orchestrator and existing CLI/TUI remain useful compatibility and diagnostic surfaces while the desktop app is introduced. The Product Requirements Document does not require their removal.
- Domain terminology in the interface and contracts should remain aligned with the root glossary. User-facing “Working directory” names the open repository/directory scope; Workspace Session remains the domain term for the continuous relationship.
- Delivery risk is concentrated in keeping frontend projection state synchronized without transferring authority, making dense live operations accessible, and preventing conversation/navigation affordances from implying authorization. Contract, governance, persistence, and end-to-end recovery tests are therefore release gates rather than secondary polish.
- Provider neutrality should be demonstrated through contracts and tests, not by prematurely implementing multiple providers.
- After review and explicit approval of this document, the next planning step is to use the local `to-issues` workflow to create independently grabbable Issue Slices. No Issue Slices should be generated before that approval.
