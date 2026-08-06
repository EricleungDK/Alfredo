# Context

## Terms

### Frontier Model

A high-capability model assigned to architecture, review, integration judgment, routing, and decision support rather than default code editing. A Frontier Model may run locally or through a remote provider; its responsibility is independent of its model or hosting assignment.

### Frontier Architect

The frontier-model role that turns a user request into a plan, skeleton design, shared context proposal, and validated task graph before local coding begins.

### Frontier Reviewer

The frontier-model role that reviews evidence packages from local agents and decides whether work should be accepted, repaired, rejected, or escalated.

### Frontier Integrator

The frontier-model role that reasons about merge order, conflicts, cross-task consistency, and final goal fit after local agents complete bounded work.

### Frontier Confirmation

A clarification or approval request raised by a Frontier Model when a Mission Commander action is ambiguous, risky, irreversible, or likely to change launch boundaries. It prevents questionable actions from silently becoming accepted mission state.

### Local-Model-First

The Alfredo product posture in which the complete primary path from Mission formation through reviewed Local Agent evidence works with models under the Mission Commander's local control. Remote providers and complete external coding-agent harnesses may be optional adapters, but they are not prerequisites and do not replace the Orchestrator as Mission authority.

### Local Inference Profile

A versioned, empirically qualified combination of local model identity and inference boundaries for one Alfredo role, including context and output budgets, thinking and sampling mode, weight and cache precision, residency, and concurrency. Qualification is based on governed reviewed-outcome evidence rather than raw token speed, and a profile does not confer Mission authority.

### Local Inference Lease

A temporary scheduling claim on bounded local inference capacity for one controller or Local Agent model turn. It may order or delay inference and expose non-authoritative loading, prefill, and generation progress, but it does not authorize work or change accepted Mission state; the Orchestrator continues to own cancellation, session, evidence, and review authority.

### Local Agent

A coding worker powered primarily by a local model through Ollama or a governed local command. Local Agents receive task packets with acceptance criteria and allowed paths, work in isolated session worktrees, execute commands inside a minimal Bubblewrap filesystem view, and must return an Evidence Package. Controller/router identities are not manually assignable workers. Cancelled Local Agent work is terminal unsuccessful work, not completed work.

### Retirement Unit

An independently authorized Local Agent session that owns an isolated worktree and therefore has its own preservation and retirement outcome, including child delegations and repairs. Model turns and helper calls that do not own a worktree remain part of their owning session and are not Retirement Units.

### Worktree Identity

The exact agreement between a Retirement Unit's stored session path, its deterministic managed path, its canonical filesystem location, and, when Git-backed, the expected repository's worktree registration. Any disagreement means ownership is ambiguous.

### Runner Quiescence

The independently corroborated absence of a Retirement Unit's supervising runner and spawned process group after its terminal outcome is durably recorded. A terminal label alone is not proof, and ambiguous liveness prevents retirement.

### Preservation Budget

App-local capacity reserved before a Retirement Unit starts, sized for its maximum permitted preserved changes and recovery record. It remains bound to that unit until preservation is verified or discard is explicitly authorized, so execution cannot consume the only space needed to preserve its work.

### Retirement Snapshot

The verified app-local preservation of a quiesced Retirement Unit's baseline identity, all Git-visible agent-authored dirty state, and registered evidence and artifacts. It consists of a durable Retirement Record and a retained Snapshot Payload; Git-ignored state outside registered evidence is not preservation input.

### Retirement Record

The compact durable manifest of a Retirement Unit's identity, content hashes, authority, lifecycle receipts, and payload disposition. It remains after physical worktree retirement or Snapshot Payload reclamation.

### Snapshot Payload

The private, space-consuming content referenced by a Retirement Record that makes a Retirement Snapshot reconstructable. It follows bounded retention and storage-budget policy and may be explicitly pinned without changing the record.

### Snapshot Storage Budget

The configurable aggregate app-local capacity available for Snapshot Payloads. It reclaims eligible unpinned payloads oldest first and blocks new Retirement Units rather than silently deleting policy-protected payloads when exhausted.

### Retention Grace Period

A bounded passive interval after failed work receives a verified Retirement Snapshot, during which its original quiesced worktree remains available for Mission Commander inspection, export, or explicitly authorized repair. It creates no automatic investigation or repair and ends with physical retirement.

### Local Agent Cancellation

The terminal unsuccessful outcome of a Retirement Unit after a stop request has been accepted and its execution has actually ceased. A unit remains Cancelling while termination is in progress or unverified; cancellation does not itself discard evidence or retire the worktree.

### Retirement Blocked

The state of a quiesced Retirement Unit after bounded automatic retirement attempts cannot complete. Automatic retries stop, its worktree remains retained, and the Mission Commander may retry, inspect, export, or explicitly discard it.

### Retained Worktree Discard

The Mission Commander's irreversible authorization to delete one exact Retirement Blocked worktree despite preservation or metadata failure. It cannot override execution quiescence, containment within Alfredo's managed worktree boundary, or exclusion of the Coding Workspace.

### Ad Hoc Delegation

A narrow, Mission-qualified work packet that remains distinct from an Issue Slice. Alfredo's deterministic task path or a Frontier Model may propose it through the Orchestrator, and an eligible Local Agent executes it within a Workspace Session only after the Mission Commander or the exact safe automatic approval path acknowledges its boundary. It retains explicit scope, goal, acceptance criteria, allowed paths, command policy, worker identity, evidence, and review, and may later contribute to a Mission Draft.

### Controller Route

The typed result of an Agent Console controller turn. `discussion` returns commentary only; `coding-task` also returns a bounded task request and acceptance criteria that may enter governed delegation. A malformed, oversized, blank, or otherwise invalid route falls back to discussion, and slash commands are handled deterministically without model redispatch.

### Governed Prompt Delegation

The automatic path from an explicit or controller-classified coding request to executable work. Alfredo persists the user turn, creates an Ad Hoc Delegation proposal, reloads the canonical queue item, verifies exact Mission, scope, original goal, acceptance criteria, allowed paths, command policy, proposed agent, and origin identity, then submits a correlated approval and dispatches the resulting queued session. Ordinary workers must explicitly be assignable, ungated, non-delegate, local/non-cloud, available, and worker-routed; missing authority metadata fails closed. Any unsafe or mismatched boundary pauses for manual handling.

### Canonical Repair Action

The single repair launch made available by any persisted repairable review, regardless of whether the decision originated in Review Workspace, the TUI, the CLI, or older runtime state. It survives reload, inherits the prior session's allowed paths and command policy, queues exactly one child session, and remains distinct from an ordinary failed-session retry that requires a new reason. Parent and prior-session overlays are bounded, staged, integrity-bound, and durably marked before any worktree effect so crash recovery cannot reread changed source state or apply an overlay twice.

### Review Decision

The Mission Commander's acknowledged judgment on a Local Agent Evidence Package. **Accept evidence** makes an Issue Slice Complete/PR-ready without merging it, or completes an Ad Hoc Delegation. **Request repair** records why the evidence is insufficient and exposes one Canonical Repair Action; it does not create a ticket or launch a session. **Ask for human review** pauses the work in `needs-human-review` for a person to inspect; it does not create a ticket or launch an agent.

### Workstation Recovery Marker

A Mission-runtime record that binds a Workstation correlation id to its full normalized request boundary in the same atomic write as an approval, assignment, or cancellation. Session-producing actions carry the equivalent marker in the created session. The Orchestrator uses it to recover a lost acknowledgement, reject cross-action correlation reuse, and reconcile missing audit phases without repeating the domain mutation.

### Session Artifact Viewer

The inline Alfredo surface for reading one registered review-safe artifact from an exact Mission and Local Agent session. The Orchestrator returns bounded text through an opaque app-local reference; raw local paths are never used as browser navigation, and unavailable or forbidden content remains an actionable inline error.

### Mission Draft

A user-requested proposal that organizes selected, relevant Ad Hoc Delegations and new work into a candidate mission. It does not become accepted mission state until the Mission Commander reviews and confirms its scope. Its ordered receipts bind prior draft, request, effect, acknowledgement, exact decision reason, and accepted Issue identity into one canonical lifecycle chain; confirmation recovery is idempotent while later separately governed Issue changes remain valid.

### Mission Formation Route

The explicit, explainable classification of a coding goal into a bounded Ad Hoc Delegation, bounded Mission discovery, or multi-session Wayfinding effort. It is worked out through the continuous Agent Console conversation—not a separate wizard—and the Mission Commander may override it before publication.

### Orchestrator

The authority process that validates task graphs, creates isolated workspaces, enforces allowed paths and command rules, records factual task status, collects evidence, and blocks invalid work.

### Alfredo Workstation

The prompt-first React/Tauri operational shell for a coding workspace, launched through the `alfredo` npm command. The public install spec is `alfredo-agent`, which resolves an exact-version host adapter such as `alfredo-agent-linux-x64-gnu`; registry publication is not accepted until a clean registry-only smoke passes. Its dominant lane is a continuous controller conversation with commands, skills, task requests, and project discussion. Its persistent Mission Work lane shows real Local Agent sessions, attention, active AFK Issue Slices, evidence, and governed actions. `Mission Control` and `Albert` remain compatibility names in internal paths and older records.

### Active Mission

The mission currently displayed and eligible for conversational steering in a Workspace Session. Changing the Active Mission does not stop bounded work already running in other missions.

### Background Mission

A non-active mission whose approved Local Agent sessions may continue bounded work. It cannot receive ambiguous conversational steering, and any new approval or clarification need is surfaced to the Mission Commander.

### Mission Commander

The single human operator supervising a Workspace Session and one Active Mission at a time through the Alfredo Workstation. The Mission Commander steers intent, approves boundaries, and remains the final authority for launch, review, and merge decisions.

### Workspace Session

The continuous working relationship between the Mission Commander and Albert within an open coding workspace. It preserves conversation continuity while missions are created, paused, resumed, or switched, without merging their mission-specific Shared Context.

### Starting Location

The directory Alfredo initially uses for choosing an existing repository or creating a new one. It is not a Coding Workspace unless the Mission Commander deliberately selects it as the repository root.

### Coding Workspace

The repository root Alfredo is currently operating within. It is chosen by the Mission Commander, remains distinct from Alfredo's installation and backend source locations, and scopes the Missions that Alfredo may start or resume for that repository.

### Additional Path Grant

Mission Commander authorization for Albert or a skill to access a filesystem location outside the Workspace Session's primary workspace root and app-managed runtime locations. A grant has an explicit access level and duration and cannot be expanded by an agent or skill.

### Additional Path Grant Request

An append-only Orchestrator record created when a governed Shell action needs authority outside its current boundary. It binds a unique request id and correlation to the Mission, canonical path, read/write access, fixed duration, reason, affected action, and request time. Grant and denial decisions must match this exact pending record; the UI projects it directly rather than parsing an error message.

### Failed Workstation Action Continuity

A bounded, workspace-scoped React record used only when a consequential action reaches a terminal rejected, stale, or failed outcome that may not have reached the backend, such as a transport-unavailable `/run`. It may restore the failed action group after desktop refresh, but it is never canonical accepted or pending state and cannot authorize work.

### Agent Console

The unified arrival-ordered prompt lane where the Mission Commander talks with Alfredo and sees controller replies, governed command cards, consequential workstation actions, and their outcomes. Optimistic user turns appear immediately, canonical history survives restart, and individual Local Agent sessions remain available through Mission Work rather than becoming separate primary conversations.

### Mission Work

The persistent secondary lane beside Agent Console. It projects the Mission Execution Tree and makes each Local Agent session inspectable for its assigned work, current activity, status, evidence, and next action. Governed cancellation, review, repair, blocker explanation, and completed Issue Slice archive/restore actions state their consequence before submission and display accepted outcomes only after their exact canonical acknowledgement.

### Mission Execution Tree

The work-centered supervision model for a Mission. It presents Issue Slices and Ad Hoc Delegations with directly inspectable Local Agent sessions, nested delegations, current activity, review or repair state, blockers, and next actions as one hierarchy. Completed archived Issue Slices appear under retained history without losing their canonical identity, evidence, Activity Journal links, or nested sessions.

### Operations Workspace

The multi-view operational lane in the Alfredo Workstation for workspace queues, mission boards, interactive review, session inspection, and Activity Journal exploration. It changes view with the selected work while the Agent Console remains continuous.

### Workspace Queue

The decision-only inbox for unresolved Issue Change Proposals, Frontier Confirmations, Ad Hoc Delegation approvals, and pending Mission Draft decisions across a Workspace Session. Governance decisions are resolved here rather than embedded in mission progress or activity views. Opening Queue replaces the lower assignment region; resolved history and standing work-creation forms do not appear in the inbox.

### Conversation Scope

The working directory, Mission, or Issue Slice target used internally to assemble Working Context for the next controller turn. It remains stable across navigation and never grants authority. In Alfredo it is deliberately de-emphasized behind the compact Context Inspector instead of being a primary workflow or ticket control.

### Activity Journal

The durable chronological record of meaningful Mission Commander, Orchestrator, Frontier Model, and Local Agent actions within a Workspace Session. It supports attribution and reconstruction without treating transient output as accepted mission state.

### Shell Terminal

The governed command path used by `/run` and Command Audit. Commands use argv execution without a shell, a minimal Bubblewrap filesystem view, sanitized environment, bounded timeout, read/write-aware workspace and Additional Path Grant mounts, and submitting-Mission attribution. Execution is durably marked before process start; live work projects as `executing`, a dead owner becomes `outcome-unknown`, and the exact correlation is never automatically re-executed. Missing request, approval/denial, and final audit phases recover before later Console/Activity entries. Raw output remains transient.

### Shared Context

The canonical runtime understanding for a coding mission, including the user goal, accepted decisions, task graph, interface ownership, global constraints, and integration status. Local agents may propose updates but cannot write to it directly.

### Working Context

The curated model input assembled for a specific interaction from Workspace Session summaries, relevant Shared Context, unresolved items, recent conversation, and deliberately referenced history. It is bounded and reconstructable even though the full Agent Console history remains available to the Mission Commander.

### Context Inspector

The Mission Commander surface for examining and influencing the sources assembled into Working Context. It can pin or exclude eligible material but cannot bypass governance for accepted Shared Context.

### Evidence Package

The structured completion report required before frontier review. It includes changed files, a bounded real `review.diff` artifact, commands run, test results, known risks, and proposed context updates. Blocked or local-only file contents are redacted from diff artifacts, and evidence is not recorded as validated unless validation succeeds. UI evidence controls exist only for registered reader-compatible opaque references; arbitrary relative or host paths do not become navigation.

### Shared Understanding Gate

The project-level authority boundary in Wayfinder Chart mode before its canonical map and child tickets may be created. It opens through an explicit Mission Commander confirmation or a visible agent acknowledgment of the destination, scope, constraints, and known uncertainty; an agent acknowledgment ends its turn and never initiates a later skill or artifact action.

### Product Requirements Document

The structured source of truth for an agreed product, project, or feature direction. Its existence does not prescribe which skill the Mission Commander invokes next.

### Issue Slice

A vertical implementation slice for agreed work. Each slice is independently grabbable, demoable or verifiable on its own, and can be assigned to a model agent.

### Ready Issue Slice

An Issue Slice that is approved, unblocked, and eligible to launch. Ready describes launch eligibility, not completed work.

### Complete Issue Slice

An Issue Slice whose work is finished and whose Evidence Package has been reviewed and accepted, making it PR-ready. Complete does not mean merged.

### Issue Change Proposal

A Mission Commander edit to an Issue Slice that has not yet been accepted into the mission state. Proposals are required when a board edit affects a locked Issue Slice, launch boundary, blocker relationship, acceptance criteria, risk, classification, evidence requirement, or other questionable action.

### Issue Graph

The dependency graph formed by Issue Slices and their blockers. It replaces a generic task graph as the execution planning structure for model-agent delegation.
