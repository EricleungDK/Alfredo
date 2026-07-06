# Local Coding Agent MVP Product Requirements Document

Status: complete
Date: 2026-06-15

## Problem Statement

The user wants a local coding-agent product that makes coordinated model labor practical without turning the target repo into an opaque pile of model activity. Existing coding-agent workflows tend to rely on one model acting directly in one workspace, which makes planning, delegation, review, evidence, privacy boundaries, and merge readiness hard to track.

The product needs to feel CLI/TUI-first like OpenCode, while adding a stronger Orchestrator that controls execution, delegates bounded coding work to Local Agents, uses Frontier Models for planning and review, and leaves the user as the final guardrail for main-branch merges.

The MVP must turn a completed Plan Grill Gate into a Product Requirements Document, turn that Product Requirements Document into approved Issue Slices, launch Local Agents in isolated workspaces, collect Evidence Packages, run checkpoint-based Frontier Model review, maintain mission records, and prepare GitHub PRs without auto-merging.

## Solution

Build a TUI-first local orchestration tool for coding missions.

The user starts from a sharpened request or completed Plan Grill Gate. The Frontier Architect uses that context to produce a Product Requirements Document and an Issue Graph. The user reviews the Issue Slices in a mission-control TUI, approves or revises the work contracts, and can override model assignments before launch.

The Orchestrator then creates isolated worktrees outside the target repo, gives each Local Agent a narrow task packet, enforces file and command boundaries, records factual execution state, and requires an Evidence Package before a slice can move to Frontier Reviewer evaluation.

Frontier roles do not edit code by default. The Frontier Reviewer evaluates Evidence Packages against acceptance criteria and returns Approved, Approved with limitations, Needs repair, Needs human review, or Rejected. The Frontier Integrator reasons about merge order, conflicts, goal fit, and PR readiness. GitHub PR automation uses existing local authentication when available and falls back to local PR-ready instructions when it is not.

Mission records are generated as navigable Markdown in the target repo for human and AI consumption. Runtime state and bulky evidence are stored locally outside the target repo. The final merge remains human-only.

## User Stories

1. As a user starting a new product, I want a Plan Grill Gate before planning, so that the system sharpens the request before generating work.
2. As a user who has already completed a grill session, I want that session treated as alignment approval, so that I do not have to repeat the same interrogation.
3. As a user, I want a Product Requirements Document created from the approved alignment, so that the mission has a clear what and why.
4. As a user, I want the Product Requirements Document to use the project's domain vocabulary, so that all later Issue Slices stay understandable.
5. As a user, I want the Frontier Architect to create Issue Slices from the Product Requirements Document, so that the work becomes independently grabbable.
6. As a user, I want Issue Slices to include blockers, so that I can see the Issue Graph before launch.
7. As a user, I want each Issue Slice marked HITL or AFK, so that I know which slices need human interaction.
8. As a user, I want each Issue Slice to include acceptance criteria, so that completion can be judged by behavior rather than model confidence.
9. As a user, I want each Issue Slice to include evidence requirements, so that review can be grounded in artifacts.
10. As a user, I want to approve the Issue Slice breakdown before assignment, so that bad work packets do not reach Local Agents.
11. As a user, I want to request revise, split, merge, or hold decisions on Issue Slices, so that I can shape the Issue Graph without editing raw files.
12. As a user, I want approved work contracts locked, so that goal, acceptance criteria, blockers, classification, risk level, and evidence requirements do not drift after approval.
13. As a user, I want to unlock and re-review locked fields, so that intentional contract changes remain possible and explicit.
14. As a user, I want model assignment recommendations, so that the tool can suggest the right Local Agent or Frontier role.
15. As a user, I want to override model assignments before launch, so that I remain in control of execution.
16. As a user, I want one Issue Slice to map to one primary Local Agent by default, so that responsibility is clear.
17. As a user, I want ready slices launched in batches by default, so that independent work can run efficiently.
18. As a user, I want individual launch controls, so that risky slices can be started one at a time.
19. As a user, I want blocked slices to show their blockers, so that I know what must finish before launch.
20. As a user, I want the TUI to show review queue, assignment state, execution state, and frontier review outcomes, so that I can run the mission from one operational surface.
21. As a user, I want the Issue Review Board to be app-native, so that review decisions are not trapped in a generated report.
22. As a user, I want exportable human-readable summaries, so that I can share or archive mission state.
23. As a user, I want AI-readable Markdown records, so that later agents can recover mission context quickly.
24. As a user, I want the Orchestrator to be the authority, so that models cannot bypass execution boundaries.
25. As a user, I want Local Agents to receive narrow task packets, so that they do not need the full product vision to make bounded edits.
26. As a user, I want Local Agents to work in isolated worktrees, so that parallel edits do not collide in the target repo.
27. As a user, I want worktrees to live outside the target repo, so that the target repo stays clean.
28. As a user, I want accepted worktrees marked cleanup-eligible instead of immediately deleted, so that I can inspect them after completion.
29. As a user, I want cleanup to be user-triggered in MVP, so that the system does not destroy useful evidence prematurely.
30. As a user, I want runtime state persisted locally outside the target repo, so that mission control can recover without polluting project history.
31. As a user, I want mission records inside the target repo, so that the repo carries a git-trackable summary of what happened.
32. As a user, I want bulky evidence outside the target repo, so that logs and raw artifacts do not bloat project history.
33. As a user, I want mission records to separate PRD, Issue Slices, Mission, and Evidence, so that each record type has a clear purpose.
34. As a user, I want the mission README to show current mission summary and next action, so that a returning agent can resume quickly.
35. As a user, I want the mission timeline to record meaningful state transitions and decisions, so that it stays useful and not transcript-like.
36. As a user, I want command approvals excluded from the timeline, so that operational approval noise does not obscure mission history.
37. As a user, I want a Local Agent tracker, so that I can see task-level worker status.
38. As a user, I want per-Issue Slice mission records, so that execution status, review status, repairs, evidence links, and integration results are easy to inspect.
39. As a user, I want the Evidence Package to include changed files, diff, commands run, test results, known risks, and proposed context updates, so that review has concrete material.
40. As a Frontier Reviewer, I want missing evidence to block approval, so that slices cannot pass on unsupported claims.
41. As a Frontier Reviewer, I want to approve with limitations when Local-only files prevent full remote review, so that the user sees review limits explicitly.
42. As a Frontier Reviewer, I want to request repair when evidence contradicts acceptance criteria, so that the workflow can recover without immediate human escalation.
43. As a Frontier Reviewer, I want to escalate repeated or architectural failures to the Frontier Architect, so that broken slice structure can be revised.
44. As a Frontier Integrator, I want to reason about merge order and conflicts after slices complete, so that completed work still fits the whole mission.
45. As a Frontier Integrator, I want to recommend grouping PRs only when slices are tightly coupled, so that PRs stay narrow by default.
46. As a user, I want Frontier approval to mean PR-ready rather than merge-approved, so that human merge authority remains intact.
47. As a user, I want automatic PR creation when `gh` is available, so that accepted slices can move into GitHub efficiently.
48. As a user, I want fallback PR instructions when `gh` is missing or unauthenticated, so that work remains usable offline.
49. As a user, I want one Issue Slice to map to one PR by default, so that review scope stays clear.
50. As a user, I want PR bodies to link to issue, evidence, frontier review, and Local Agent activity, so that reviewers get a readable summary rather than raw logs.
51. As a user, I want branch names to include mission and Issue Slice identity, so that branches are traceable.
52. As a user, I want simple PR labels in MVP, so that GitHub organization works without formal status-check integration.
53. As a user, I want CI status surfaced in the TUI, so that I can see merge readiness without leaving the tool.
54. As a user, I want GitHub branch protection to enforce final mergeability, so that the local tool does not become the final gate.
55. As a user, I want visibility levels for files, so that Frontier Models do not read sensitive local-only or blocked paths accidentally.
56. As a user, I want Normal files available to Frontier Models, so that planning and review can be effective.
57. As a user, I want Local-only files withheld from remote Frontier Models unless approved, so that sensitive project material stays local.
58. As a user, I want Blocked files unavailable to all model roles that should not see them, so that explicit privacy boundaries are enforceable.
59. As a user, I want agents to request path expansion with justification, so that boundary changes are deliberate.
60. As a user, I want command policy levels, so that safe commands can run automatically while risky commands require approval.
61. As a user, I want project-specific command policy learned from approvals, so that repeated safe commands become less noisy.
62. As a user, I want file-edit permissions to remain per-Issue Slice, so that broad file access is never learned globally by accident.
63. As a user, I want no persistent caching in MVP beyond request or session derivations, so that the initial system remains predictable.
64. As a user, I want the system to record factual state separately from model opinions, so that later decisions can rely on what actually happened.
65. As a returning agent, I want a root navigation map and folder-level summaries, so that I can find the right mission information without reading everything.

## Implementation Decisions

- The product is CLI/TUI-first. Generated reports are secondary artifacts, not the primary workflow surface.
- The Issue Review Board is a permanent mission-control surface that can show review queue, Issue Graph, assignment, launch readiness, execution status, and review outcomes.
- The Product Requirements Document remains the what/why record. It is not an implementation plan and does not duplicate the glossary.
- Issue Slices are the durable planned work units. The older generic task graph concept is replaced by an Issue Graph made of Issue Slices and blockers.
- The user approves the Issue Slice breakdown before assignment and launch.
- Approval locks the work contract: slice goal, acceptance criteria, dependency blockers, HITL/AFK classification, risk level, and evidence requirements.
- Assigned model agent, launch order among ready slices, and optional notes remain editable after approval until launch.
- Changing locked fields requires an explicit unlock and re-review.
- One Issue Slice maps to one primary Local Agent by default.
- Frontier Model roles are logically split into Frontier Architect, Frontier Reviewer, and Frontier Integrator. The same underlying Frontier Model may power all three roles in MVP.
- Frontier roles do not edit code by default. Local Agents perform code edits.
- The Orchestrator is the authority over contracts, state transitions, worktrees, command policy, file-edit boundaries, evidence validation, and launch blocking.
- Local Agents receive narrow task packets instead of the full product vision.
- Frontier supervision is checkpoint-based and trigger-based, not continuous.
- Frontier approval requires evidence. Model confidence alone is not sufficient.
- Review states are Approved, Approved with limitations, Needs repair, Needs human review, and Rejected.
- Rejected work follows a tiered repair policy: same Local Agent repairs first, then a fresh Local Agent, then Frontier Architect plan revision for repeated or architectural failure, then user escalation for critical, security, or merge-risk failure.
- Command policy is enforced by the Orchestrator and expressed as auto-allowed, frontier-approvable, and human-required.
- Global command defaults apply first. Project-specific command policy can be learned from approvals and stored in app-local runtime state.
- File-edit permissions are per-Issue Slice and are not learned globally.
- Agents may request explicit path expansion with justification.
- Agent worktrees live outside the target repo and are marked cleanup-eligible after accepted or merged work. Cleanup is user-triggered in MVP.
- Runtime state lives in app-local storage keyed by project identity. Mission records live inside the target repo and are git-trackable. Bulky evidence lives outside the target repo.
- Mission records are navigable Markdown and separate mission summary, timeline, Local Agent tracker, evidence index, frontier review summary, and per-Issue Slice execution records.
- The mission timeline records meaningful state transitions and decisions, not full transcripts or command approvals.
- GitHub PR integration is in MVP scope through existing local `gh` authentication.
- If GitHub automation is unavailable, the system produces local PR-ready branches and generated PR instructions.
- Final merge is human-only. Frontier approval means PR-ready, not merge-approved.
- One Issue Slice maps to one PR by default. Grouping requires Frontier Integrator recommendation and human approval.
- PR bodies are readable summaries with links to Issue Slice, changed behavior, acceptance criteria, evidence, frontier review, and Local Agent activity.
- Formal GitHub status checks are out of scope for MVP. The tool surfaces CI status while GitHub branch protection enforces mergeability.
- Visibility levels are Normal, Local-only, and Blocked. Frontier review must explicitly mark limitations when Local-only files affect work.
- Persistent caching is out of scope for MVP except request-level or session-level in-memory derivations.

## Testing Decisions

- Tests should verify external behavior and state transitions rather than implementation details. A good test proves that a user-visible workflow, persisted record, boundary enforcement rule, or generated artifact behaves correctly.
- The Plan Grill Gate to Product Requirements Document seam should be tested by feeding approved alignment input and verifying the resulting document uses the required template sections and glossary vocabulary.
- The Product Requirements Document to Issue Slice seam should be tested by feeding a Product Requirements Document and verifying vertical Issue Slices, blockers, HITL/AFK classification, risk, acceptance criteria, and evidence requirements.
- The Issue Slice approval and locking seam should be tested by approving a slice, attempting locked-field changes, and verifying unlock plus re-review is required.
- The assignment seam should be tested by accepting recommendations and overriding individual assignments before launch.
- The Local Agent execution lifecycle seam should be tested by launching a ready Issue Slice, creating an isolated worktree, recording task state, enforcing allowed paths and commands, and producing a completion state.
- The Evidence Package validation seam should be tested by submitting complete and incomplete evidence packages and verifying incomplete packages block Frontier Reviewer approval.
- The Frontier Reviewer state seam should be tested across Approved, Approved with limitations, Needs repair, Needs human review, and Rejected outcomes.
- The repair policy seam should be tested by simulating first rejection, second rejection, repeated architecture failure, and critical escalation.
- The mission record generation seam should be tested by running a mission state transition and verifying the Markdown records update without embedding bulky evidence.
- The GitHub PR seam should be tested with an available authenticated `gh` path and an unavailable or unauthenticated fallback path.
- The privacy seam should be tested by classifying files as Normal, Local-only, or Blocked and verifying Frontier Model prompts and review summaries honor those classifications.
- The command policy seam should be tested by classifying commands as auto-allowed, frontier-approvable, or human-required and verifying the Orchestrator blocks or requests approval correctly.
- Prior art in the repo is the Issue Review Board prototype, which provides interaction behavior for dependency ordering, assignment override, approval actions, execution board grouping, and export summary behavior. Production tests should use those behaviors as acceptance examples, not reuse the throwaway prototype as the implementation.

## Out of Scope

- Continuous Frontier Model supervision during Local Agent work.
- Frontier roles editing code by default.
- Automatic merge to main or bypassing human merge authority.
- Formal GitHub status-check integration.
- Timeline compaction.
- Persistent cross-session caching beyond durable runtime state and mission records.
- Auto-deleting accepted or merged worktrees.
- Learning global file-edit permissions from prior Issue Slices.
- Embedding bulky raw evidence directly into mission Markdown.
- Replacing the TUI with a web-app-first workflow.
- Treating the existing HTML Issue Review Board prototype as production code.
- Deep commit-message policy beyond traceable branch and PR conventions.

## Further Notes

- The completed grill session is treated as alignment approval for this Product Requirements Document.
- The Issue Review Board prototype favored a mission-control board with exportable summaries. That informs the MVP interaction model, but the production surface remains TUI-first.
- The product name appears as Albert in existing project documentation. The glossary uses product-neutral role names and should remain the source of truth for domain terms.
- The MVP should keep records navigable. A returning agent should read the current mission summary first and search the timeline only when needed.
