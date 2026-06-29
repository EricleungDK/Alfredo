# Project Architecture - Albert MVP

**Last Updated**: 2026-06-26

## Quick Overview

Albert is a local coding-agent orchestrator MVP. It starts from local markdown Product Requirements Documents and Issue Slices, renders a textual mission-control TUI, persists runtime state outside the target repo, lets Qwen route approved work to the right agent tier, launches bounded Local Agent sessions in isolated worktrees, collects Evidence Packages, routes Frontier review outcomes, relaunches repair sessions with prior review context, and prepares PR instructions without auto-merging.

The MVP now includes real runner paths: deterministic fake runner, command-backed runner, and an Ollama runner that asks a configured model for a JSON file plan and writes generated files into the isolated worktree. A live Qwen3.6-27B run generated a runnable `prototype_app.py` and moved the slice to PR-ready after review. Gemma workers have also been live-verified, including a repair loop after a failed first pass.

## Tech Stack

- Python 3 stdlib only.
- `unittest` for acceptance coverage.
- Local markdown tracker under `.scratch/`.
- Local model execution through Ollama.
- App-local runtime JSON and bulky artifacts stored outside the target repo.
- Command Deck desktop work uses React 19 and TypeScript behind a Tauri 2 shell, while Python remains the authoritative Orchestrator.

## Command Deck Workspace Snapshot Boundary

Command Deck Issue 01 introduces `albert_mvp.workspace.WorkspaceSnapshotService` as the desktop projection boundary. It returns a versioned canonical snapshot containing Workspace Session identity, Active Mission, explicit Conversation Scope, acknowledged Operations Workspace selection, and Mission Board state. Preferences are stored atomically under the existing app-local mission runtime directory; malformed persistence returns a structured failure instead of allowing the client to fabricate accepted state.

The React application lives under `mission-control/`. Its `WorkspaceClient` interface isolates contract-faithful tests from the production Tauri transport. React owns loading, empty, retry, and visual projection behavior only. It does not own lifecycle or acceptance decisions.

The Tauri command starts `python3 -m albert_mvp workspace-snapshot` with explicit repository, tracker, issue, runtime, mission, and agent-registry paths. Rust validates the versioned response and preserves structured startup, contract, and persistence failures. `TauriWorkspaceClient` maps that result into the React load-state union. The Rust integration test launches the real Python boundary twice and verifies Workspace Session restoration.

## Command Deck Live Synchronization Boundary

Command Deck Issue 02 extends the same boundary with correlated semantic actions and ordered revision batches. `WorkspaceSyncService` rejects mismatched expected revisions before mutation and stores the accepted preference update plus its event in one atomic JSON replacement. Event batches must be contiguous from the client's acknowledged revision; malformed, missing, or future revisions produce persistence or revision-gap failures that require a canonical snapshot reload.

Tauri exposes typed `workspace_action` and `workspace_updates` commands. React applies batches through a pure reducer that preserves unchanged Workspace Session, mission, and board identities. Pending intent never changes accepted projection state. Transport failure preserves the last acknowledged snapshot and shows Offline; Reconnect calls `workspace_snapshot` and replaces the projection rather than assuming missed events were delivered.

## Agent Console and Conversation Scope Boundary

Command Deck Issue 03 adds `AgentConsoleHistoryService`, an append-only, atomically persisted message history. Every record retains a stable sequence, role, content, source, one of five explicit outcomes, and a snapshot of the acknowledged Conversation Scope at submission. Public append commands require the current workspace revision and the exact displayed scope; a mismatch is rejected before history mutation.

`workspace-scope` validates Working directory, Active Mission, and Issue Slice targets before submitting the existing expected-revision preference action. Scope changes update Working Context selection only: the command has no launch, approval, assignment, path-access, review, or locked-contract fields. Tauri exposes typed scope, append, and history commands. React keeps history, the composer draft, and the uncommitted scope selection above the Operations projection so navigation and Active Mission replacement cannot silently retarget a message.

## Working Context Assembly and Curation Boundary

Command Deck Issue 04 adds `WorkingContextService`, an Orchestrator-owned projection derived from the acknowledged Conversation Scope. It deterministically identifies required governed Workspace Session and Shared Context sources, eligible unresolved items, the six most recent messages matching that scope, and pinned older messages as deliberate references. Included content is capped at 4,000 characters while the independent Agent Console history remains complete.

Pin/exclude curation is stored atomically in `working-context-curation.json` with its own expected revision. Governed and unknown sources return structured `context-source-ineligible` failures before mutation. Curation cannot alter workspace revision, issue contracts, history, permissions, or Shared Context inputs. React renders the returned projection, hides curation controls for governed sources, and reloads only after Tauri returns an acknowledgement.

Issue 04 integration also hardened the core runtime boundary: `AlbertMission._persist` now writes a unique sibling temporary file and atomically replaces `runtime.json`. Concurrent CLI readers therefore observe a complete old or new document instead of an in-place truncation window.

## Active Mission Board and Issue Inspection Boundary

Command Deck Issue 05 adds a workspace-level mission registry and expected-revision Active Mission switching. Background Mission sessions keep running under their accepted boundaries, while new approvals and clarification needs surface through Workspace Queue attention. Switching the Active Mission reloads mission board state but preserves the Workspace Session, Agent Console history, draft, and acknowledged Conversation Scope.

Command Deck Issue 06 deepens the canonical Mission Board projection with Issue Slice summaries. `AlbertMission.board_summary()` now emits lifecycle, progress, launch eligibility, blockers, accepted boundary, attached sessions, role/provider/model provenance, Evidence Package summary, and Working Context source summaries. Ready means approved, unblocked, and launch-eligible; Complete means evidence-accepted and PR-ready; Merged remains distinct. React renders this projection as an Issue Graph plus local inspector selection, so inspecting a slice never changes Conversation Scope or accepted mission state.

Command Deck Issue 07 makes model assignment provider-neutral across fake and Ollama runners. Configured agents can report availability and an availability reason, and `AlbertMission.board_summary()` emits `model_assignment` plus session operation status/failure fields. Unavailable or disconnected assignments are visible before launch and block only launch/repair launch; provider failures remain session operation failures and do not mutate accepted Issue Slice lifecycle. React and Tauri consume the same assignment contract without Ollama-specific UI logic.

Command Deck Issue 08 adds Review Workspace as the dedicated evidence-decision surface. `ReviewWorkspaceService` projects sessions awaiting review with complete Evidence Package details, missing required evidence, proposed context updates, artifacts, and file visibility limitations. Review decisions are acknowledged commands with expected-revision checks: accept requires complete evidence and marks the Issue Slice Complete/PR-ready without merging, repair requires a reason and exposes the repair next action, and human escalation records `needs-human-review`.

Command Deck Issue 09 adds Workspace Queue as the governance inbox for Issue Change Proposals and Frontier Confirmations. `WorkspaceQueueService` persists pending governance items in `workspace-queue.json`, groups and filters by item type and Mission, and records source, requested action, affected boundary, consequence, and proposed payload. Locked Issue Slice governed-field edits create pending proposals instead of mutating accepted state. Queue decisions use expected queue revision checks: approval of an Issue Change Proposal applies the proposed governed fields and reopens the slice for re-review, while reject/defer and stale decisions preserve authoritative Mission state. React renders grouped queue items with decision controls only inside Workspace Queue, and Mission summaries expose compact links to pending queue items.

Command Deck Issue 10 adds Ad Hoc Delegations as a distinct Workspace Queue item type. Proposed delegations capture originating Agent Console context, accepted Conversation Scope, acceptance criteria, allowed paths, command policy, and proposed Local Agent. The proposal path is exposed through the `ad-hoc-delegation-proposal` CLI command, Tauri `ad_hoc_delegation_proposal` command, and a compact React Workspace Queue form sourced from the latest Agent Console message. Approval creates bounded `ADHOC-*` `LocalAgentSession` records without adding `mission.issues` entries; non-auto-allowed command policies deny launch and leave the queue item pending. Workspace Mission summaries expose ad hoc session status plus role/provider/model provenance, and Review Workspace accepts valid Evidence Packages for ad hoc sessions as complete bounded work without changing Issue Slice lifecycle.

Command Deck Issue 11 adds `MissionDraftService` as the backend proposal boundary for assembling selected Ad Hoc Delegations and new work before accepted mission state changes. Drafts persist in `mission-drafts.json` with their own revision, included `ADHOC-*` work, explicit exclusions, new work, dependencies, and unresolved decisions. Creating or editing a draft does not change Workspace revision, sessions, Issue Slices, or mission-specific context. Draft edits rebuild included work only from explicitly selected ids so later unselected Ad Hoc Delegations are not attached silently. Confirmation requires the current draft revision and creates a durable accepted Issue Slice through `AlbertMission`, while abandonment and stale confirmation preserve existing missions. The CLI exposes the boundary through `mission-drafts`, `mission-draft-create`, `mission-draft-update`, `mission-draft-confirm`, and `mission-draft-abandon`; Tauri exposes typed `mission_drafts`, `mission_draft_create`, and `mission_draft_decision` commands. React's Workspace Queue lets the Mission Commander include/exclude pending Ad Hoc Delegations, enter proposed goal/new work/dependencies/unresolved decisions, create a Mission Draft through the Orchestrator, review the proposed scope before confirmation, require a reason for Confirm/Abandon, and reload canonical state only after acknowledgement.

Command Deck Issue 12 adds `ActivityJournalService` as a persistence concern separate from canonical snapshot/runtime state. `activity-journal.json` stores contiguous append-only entries with UTC time, Mission Commander/Orchestrator/Frontier Model/Local Agent attribution, stable action type and correlation, affected-entity links, and available evidence links. The journal records acknowledged workspace navigation, Review Workspace decisions, Workspace Queue decisions, Mission Draft lifecycle actions, Frontier Confirmation requests, Orchestrator session launches, and validated Local Agent Evidence Packages; pending token/terminal output, stale actions, and failed evidence remain excluded. Search plus Mission, actor, action-type, and inclusive time filters preserve chronological order through Python CLI, Tauri, TypeScript client, and React Activity. A journal write failure is surfaced as `WorkspacePersistenceError` after any already-completed canonical write—cross-file atomicity is not claimed—and restart tests independently compare canonical reconstruction with retained journal order and attribution.

## Project Structure

- `albert_mvp/core.py` - Orchestrator domain model: PRD loading, Issue Slice parsing, Issue Graph ordering, lifecycle state, review locking, Qwen-controlled delegation, gated delegation approval, launch sessions, repair relaunch sessions, command/file policy, runner execution, Evidence Package validation, Frontier Reviewer outcomes, mission records, and PR prep.
- `albert_mvp/agents.py` - Agent registry loading and validation for configured Frontier/router, worker, and delegate-only agent roles.
- `albert_mvp/tui.py` - Textual mission-control TUI state, renderer, and TUI-backed actions.
- `albert_mvp/cli.py` - CLI command surface over the core and TUI actions.
- `tests/test_albert_mvp.py` - end-to-end acceptance tests for the MVP workflow and development backlog.
- `albert_mvp/workspace.py` - versioned Workspace Session state, synchronization, deliberate scope changes, Active Mission switching, Working Context curation, Review Workspace evidence decisions, Workspace Queue governance/ad hoc delegation decisions, Mission Draft proposal/confirmation state, append-only Activity Journal, and atomic scoped Agent Console history.
- `tests/test_workspace_snapshot.py` - snapshot, restart, empty-state, and persistence-failure integration coverage.
- `mission-control/src/` - React/TypeScript Command Deck projection and interaction tests.
- `mission-control/src-tauri/` - Tauri 2 desktop shell, typed Python bridge, and desktop-to-backend integration tests.
- `.albert/agents.json` - model registry. Qwen3.6-27B is the frontier/router; Gemma4-12B and Gemma4-26B are normal local workers; Qwen2.5-Coder 14B and DeepSeek-R1 14B are local delegate-only escalation targets.
- `.scratch/local-coding-agent-mvp/` - product PRD and Issue Slice workflow records.
- `.scratch/local-coding-agent-mvp-development/` - implementation roadmap and completed development backlog.
- `docs/agents/` - per-repo configuration for local engineering skills.
- `prototypes/issue-review-board/` - throwaway HTML design probe, not production code.

## Key Interfaces / Boundaries

- Product Requirements Document to Issue Slices: local markdown records are parsed into the Issue Graph.
- Mission-control TUI: the textual TUI summarizes ordered slices, blockers, review state, assignment options, launch readiness, review queue, and PR readiness.
- Approval locking: approved Issue Slices lock contract fields while allowing assignment and notes before launch.
- Reopen control: completed or stuck slices require explicit reopen with a recorded reason before re-review.
- Agent registry: configured agents include stable id, role, provider, runner, command/model details, routing role, delegate-only visibility, approval requirements, and provider-neutral availability. Unknown assignments are rejected when a registry is configured.
- Qwen-controlled routing: approved issues can be routed through the configured `routing: router` agent. Qwen returns a structured delegation decision with complexity, recommended agent, reason, and approval requirement.
- Delegate-only policy: delegate agents are hidden from normal assignment and cannot be launched until Qwen selects them. Configured gated or cloud delegates require explicit `approve-delegation` before Albert records their runner command as allowed.
- Local Agent launch: approved unblocked slices create isolated worktrees and session records with task packets, allowed paths, command policy, and evidence requirements.
- Runner execution: fake, command, and Ollama runners record artifacts and produce automated Evidence Packages.
- Model assignment projection: Issue Slice and session surfaces expose role, provider, model identity, availability, operation status, and failure reason from authoritative state.
- Command policy: commands classify as `auto-allowed`, `frontier-approvable`, or `human-required` before execution.
- Visibility policy: files classify as `Normal`, `Local-only`, or `Blocked` for Frontier Model access.
- Evidence Package: changed files, diff summary, commands, test results, risks, proposed context updates, and artifacts are required before approval.
- Review Workspace: active sessions awaiting review expose complete and incomplete evidence, visibility limitations, missing evidence, and acknowledged accept/repair/human-escalation decisions.
- Workspace Queue: locked Issue Change Proposals and Frontier Confirmations remain pending until acknowledged approve/reject/defer decisions; queue items expose source, requested action, affected boundary, consequence, and compact `workspace-queue#...` attention links.
- Ad Hoc Delegation: Agent Console intent can become a pending Workspace Queue proposal and only approved proposals launch bounded `ADHOC-*` Local Agent sessions; these sessions require auto-allowed command policy, Evidence Packages, and Review Workspace acceptance while remaining distinct from Issue Slices.
- Mission Draft: selected Ad Hoc Delegations and new work persist as proposed state until Mission Commander confirmation; confirmation creates an accepted Issue Slice, while edit, abandon, and stale-confirmation paths preserve existing Mission state.
- Activity Journal: meaningful acknowledged actions persist chronologically with actor, affected-entity, queue/session/evidence links, and independent filters; canonical snapshots reconstruct current state separately, and transient or failed output is excluded.
- Frontier Reviewer: review outcomes drive PR readiness, repair routing, or escalation; accepted evidence is Complete/PR-ready and never presented as merged.
- Repair relaunch: repairable review outcomes can launch a new Local Agent session for the same or specified fresh agent, carrying prior review outcome, reason, evidence, and artifact links in the task packet.
- PR preparation: the tool generates branch/PR instructions or a `gh pr create` command when available, but never marks a slice merge-approved.
- Mission records: generated Markdown is written inside the target repo; runtime state and bulky evidence stay outside it.

## Verification

- `python3 -m unittest discover -s tests` passes 148 tests.
- `npm test -- --run` passes under `mission-control/` with 55 frontend, transport, and reducer tests.
- `npm run typecheck` passes under `mission-control/`.
- `cargo fmt --manifest-path mission-control/src-tauri/Cargo.toml -- --check` passes.
- `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` passes 19 Rust bridge tests, including real Python launch/restoration, action/update transport, scoped Agent Console restoration, mission switching, Working Context curation/rejection across restart, Review Workspace contract preservation, Workspace Queue contract preservation, Ad Hoc Delegation proposal transport, Mission Draft create/confirm transport, and Activity Journal filter transport.
- Issue 10 verification passes 126 Python tests, 47 focused frontend/client/sync tests, TypeScript typecheck, Rust formatting, and 15 Rust bridge tests for the Ad Hoc Delegation proposal, approval, bounded launch, permission-denial, evidence, and review workflow.
- Issue 11 backend/CLI verification passes 132 Python tests for Mission Draft selection, exclusion, revision, confirmation, abandonment, stale confirmation, command transport, and restart persistence. Tauri/client/React verification passes 32 App tests, 19 TypeScript client tests, 2 sync reducer tests, TypeScript typecheck, and 18 Rust bridge tests, including real Python-created Mission Draft creation, projection loading, and confirmation through the Tauri bridge.
- Issue 12 verification passes 148 Python tests, 55 frontend/client/sync tests, TypeScript typecheck, Rust formatting, and 19 Rust bridge tests. Focused persistence coverage proves all four actors, chronological search/filtering, affected/evidence/queue links, transient and failed-action exclusion, explicit journal-write failure semantics, and separate restart reconstruction.
- `python3 -m albert_mvp agents ... --agent-config .albert/agents.json` lists Qwen3.6-27B as router, Gemma4-12B and Gemma4-26B as normal workers, and Qwen2.5-Coder 14B plus DeepSeek-R1 14B as delegate-only targets.
- `python3 -m albert_mvp tui ... --agent-config .albert/agents.json` hides delegate-only targets from normal assignment while still exposing routing next actions.
- Live Qwen3.6-27B session `session-ISS-01-4` generated `prototype_app.py`; running it printed `Albert prototype ready`.
- Live Gemma4-12B local-agent session `session-ISS-01-1` generated `/tmp/albert-gemma-live-20260616/.albert-worktrees/target/ISS-01/prototype_app.py`; running it printed `Albert Gemma prototype ready`; the session reached `evidence-ready`, then `reviewed`/PR-ready after an approved review.
- Live Gemma4-26B local-agent session `session-ISS-01-1` generated an incorrect string, then Frontier review recorded `Needs repair`; `tui-action repair` launched `session-ISS-01-2` with prior evidence and review reason in `repair_context`; the repaired prototype printed `Albert Gemma 26B prototype ready` and reached PR-ready.
- Cloud delegate live execution was attempted after `ollama signin`; both `kimi-k2.6:cloud` and `deepseek-v4-pro:cloud` reached Ollama Cloud but returned `403 Forbidden`. The default registry now uses local `qwen2.5-coder:14b` and `deepseek-r1:14b` delegates instead.

## Performance Targets

- Board/TUI load should be effectively instant for small local issue sets.
- Runtime writes should be deterministic JSON updates.
- Runner artifacts should be linked, not embedded into mission Markdown.
- The MVP should remain dependency-free until a richer interactive TUI requires otherwise.
