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

### Local Agent

A coding worker powered primarily by a local model, such as Qwen Coder through Ollama. Local agents receive narrow task packets and perform the actual code edits inside orchestrator-enforced boundaries.

### Ad Hoc Delegation

A narrow, missionless work packet scoped by a Frontier Model and executed by a Local Agent within a Workspace Session after Mission Commander approval. It retains explicit boundaries, acceptance criteria, evidence, and review, and may later contribute to a mission without becoming an Issue Slice itself.

### Mission Draft

A user-requested proposal that organizes selected, relevant Ad Hoc Delegations and new work into a candidate mission. It does not become accepted mission state until the Mission Commander reviews and confirms its scope.

### Orchestrator

The authority process that validates task graphs, creates isolated workspaces, enforces allowed paths and command rules, records factual task status, collects evidence, and blocks invalid work.

### Mission Control App

The user-facing operational shell for a coding workspace. It combines continuous conversational steering with mission switching, Issue Slice review, delegation visibility, session progress, evidence review, and PR readiness.

### Active Mission

The mission currently displayed and eligible for conversational steering in a Workspace Session. Changing the Active Mission does not stop bounded work already running in other missions.

### Background Mission

A non-active mission whose approved Local Agent sessions may continue bounded work. It cannot receive ambiguous conversational steering, and any new approval or clarification need is surfaced to the Mission Commander.

### Mission Commander

The single human operator supervising a Workspace Session and one Active Mission at a time through the Mission Control App. The Mission Commander steers intent, approves boundaries, and remains the final authority for launch, review, and merge decisions.

### Workspace Session

The continuous working relationship between the Mission Commander and Albert within an open coding workspace. It preserves conversation continuity while missions are created, paused, resumed, or switched, without merging their mission-specific Shared Context.

### Additional Path Grant

Mission Commander authorization for Albert or a skill to access a filesystem location outside the Workspace Session's primary workspace root and app-managed runtime locations. A grant has an explicit access level and duration and cannot be expanded by an agent or skill.

### Agent Console

The unified conversational lane in the Mission Control App where the Mission Commander talks with Albert and sees sourced, narrated consequences of board actions. Individual model sessions remain available through drill-down rather than becoming separate primary conversations.

### Operations Workspace

The multi-view operational lane in the Mission Control App for workspace queues, mission boards, interactive review, session inspection, and Activity Journal exploration. It changes view with the selected work while the Agent Console remains continuous.

### Workspace Queue

The decision inbox for unresolved Issue Change Proposals, Frontier Confirmations, and Ad Hoc Delegation approvals across a Workspace Session. Governance decisions are resolved here rather than embedded in mission progress or activity views.

### Conversation Scope

The explicit working directory, Mission, or Issue Slice target used to assemble Working Context and interpret the next Agent Console message. Its target remains stable across navigation until the Mission Commander deliberately changes it; selecting a target does not authorize launch, permission expansion, or locked-state mutation.

### Activity Journal

The durable chronological record of meaningful Mission Commander, Orchestrator, Frontier Model, and Local Agent actions within a Workspace Session. It supports attribution and reconstruction without treating transient output as accepted mission state.

### Shell Terminal

The command-execution lane in the Mission Control App for running repository and system commands under Albert's command policy. It is distinct from the Agent Console even when both appear in the same left-side workspace.

### Shared Context

The canonical runtime understanding for a coding mission, including the user goal, accepted decisions, task graph, interface ownership, global constraints, and integration status. Local agents may propose updates but cannot write to it directly.

### Working Context

The curated model input assembled for a specific interaction from Workspace Session summaries, relevant Shared Context, unresolved items, recent conversation, and deliberately referenced history. It is bounded and reconstructable even though the full Agent Console history remains available to the Mission Commander.

### Context Inspector

The Mission Commander surface for examining and influencing the sources assembled into Working Context. It can pin or exclude eligible material but cannot bypass governance for accepted Shared Context.

### Evidence Package

The structured completion report required before frontier review. It includes changed files, diff, commands run, test results, known risks, and proposed context updates.

### Plan Grill Gate

A pre-draft alignment step that sharpens a request before the Frontier Architect creates a plan or task graph. It runs when the user asks for it, or when the request starts a new product, project, or feature. The completed gate is treated as the user's alignment approval for the resulting Product Requirements Document.

### Product Requirements Document

The structured source of truth produced after a Plan Grill Gate for new products, projects, and features. It follows the local `to-prd` skill template. Issue slices are created from this document using the local `to-issues` skill.

### Issue Slice

A vertical implementation slice produced by the `to-issues` skill from a Product Requirements Document. Each slice is independently grabbable, demoable or verifiable on its own, and can be assigned to a model agent.

### Ready Issue Slice

An Issue Slice that is approved, unblocked, and eligible to launch. Ready describes launch eligibility, not completed work.

### Complete Issue Slice

An Issue Slice whose work is finished and whose Evidence Package has been reviewed and accepted, making it PR-ready. Complete does not mean merged.

### Issue Change Proposal

A Mission Commander edit to an Issue Slice that has not yet been accepted into the mission state. Proposals are required when a board edit affects a locked Issue Slice, launch boundary, blocker relationship, acceptance criteria, risk, classification, evidence requirement, or other questionable action.

### Issue Graph

The dependency graph formed by Issue Slices and their blockers. It replaces a generic task graph as the execution planning structure for model-agent delegation.
