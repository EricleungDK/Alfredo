# FirstMate pattern evaluation for Alfredo

**Captured:** 2026-07-27  
**Wayfinder ticket:** [Evaluate FirstMate patterns for Alfredo](https://github.com/EricleungDK/Alfredo/issues/52)  
**Map:** [Chart Alfredo's reliable, observable, and faster modernization](https://github.com/EricleungDK/Alfredo/issues/41)  
**FirstMate upstream:** [`kunchenguid/firstmate`](https://github.com/kunchenguid/firstmate)  
**Inspected revision:** [`a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499`](https://github.com/kunchenguid/firstmate/commit/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499), committed 2026-07-26 at 08:08:14 UTC  
**Inspection date:** 2026-07-27

## Executive conclusion

FirstMate is most useful to Alfredo as a source of **reliability invariants**, not as a product base or runtime to import.

The evidence supports four planning conclusions:

1. **Keep Alfredo's existing authority and safety boundary.** Exact-correlation receipts, Mission-qualified canonical state, runner ownership identity, bounded recovery, `outcome-unknown`, allowed paths, Additional Path Grants, Bubblewrap isolation, Evidence Packages, repair lineage, and the Mission Execution Tree are stronger fits for a Local-Model-First workstation than FirstMate's prompt-and-terminal authority model.
2. **Adapt three FirstMate mechanisms:** durable attention delivery before observation cursors advance; explicit reconciliation precedence that never promotes event prose or terminal capture into current truth; and fail-closed worktree retirement that proves cleanup is safe before discarding session state.
3. **Prototype one combined supervision seam:** an Alfredo-owned, attention-driven Local Agent supervision and recovery loop. It should combine canonical session state with independent liveness observations, fault-inject missed notifications and dead owners, and prove that observations can raise attention without becoming Mission authority.
4. **Defer or reject the rest:** defer persistent sub-supervisors, worktree pooling, and complete external coding-harness implementations until a measured need and a stable adapter contract exist; reject autonomous permission bypass, terminal scrollback as state, direct terminal intervention as an ungoverned mutation path, and prose-level project modes as substitutes for typed Orchestrator policy.

These findings do **not** change the closed decision in [Choose Alfredo's relationship to FirstMate](https://github.com/EricleungDK/Alfredo/issues/51#issuecomment-5093947435): Alfredo remains Local-Model-First, FirstMate remains a comparative reference, and complete external coding-agent harnesses remain optional future adapters rather than prerequisites or Mission authority.

## Evidence method and scope

The verdicts use the gate established by [Choose Alfredo's relationship to FirstMate](https://github.com/EricleungDK/Alfredo/issues/51#issuecomment-5093947435):

- **Keep** — retain an existing Alfredo mechanism or boundary; FirstMate provides corroborating evidence but no justified change.
- **Adapt** — carry a named mechanism into Alfredo while preserving Local-Model-First operation, the Orchestrator as the only Mission authority, existing safety boundaries, and one canonical source of truth.
- **Prototype** — the mechanism is promising but needs disposable, comparative evidence before entering the modernization blueprint.
- **Defer** — the mechanism may become useful, but it is not needed to reach this map's destination or lacks a present trigger.
- **Reject** — the mechanism conflicts with Alfredo's approved authority, governance, product, or evidence model.

FirstMate claims were checked against the pinned upstream source: its operating contract, architecture, watcher and wake-queue code, current-state and fleet-snapshot code, spawn and teardown code, recovery playbook, adapter reference, runtime-backend documentation, and verification-oriented lifecycle documents. The earlier [FirstMate product-boundary research](./2026-07-27-firstmate-product-boundary-research.md) identified the exact upstream and established the product comparison.

Alfredo fit was checked against the local ubiquitous language and architecture sources: [domain context](../../CONTEXT.md), [project architecture](../System/project_architecture.md), [persistence schema](../System/database_schema.md), [API boundaries](../System/api_endpoints.md), [UX guidelines](../System/ux_guidelines.md), [Albert architecture](../../docs/albert-architecture.md), the [architecture and performance baseline](./2026-07-23-alfredo-architecture-performance-baseline.md), and the [workspace-selection and false-success diagnosis](./2026-07-24-workspace-selection-false-success-diagnosis.md).

This is a source-mechanism review, not a comparative production benchmark. FirstMate has extensive behavior tests and dated adapter verification, but this investigation did not run the same fault-injection workload against both systems. The upstream had no tagged release at inspection time; the reviewed contracts are pinned `main`-revision evidence, not a stable compatibility promise. ([FirstMate tags](https://github.com/kunchenguid/firstmate/tags), [FirstMate releases](https://github.com/kunchenguid/firstmate/releases))

## Verdict matrix

Verified FirstMate facts and Alfredo planning judgments are separated below.

| Area | Verified FirstMate mechanism | Verdict | Alfredo planning judgment |
|---|---|---:|---|
| Authority | One liaison supervises delegated workers; the primary does not edit project code or discard unlanded work. ([README lines 31–53](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/README.md#L31-L53), [operating contract lines 11–37](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/AGENTS.md#L11-L37)) | **Keep** | Retain the Mission Commander, Agent Console, Local Agent work, Evidence Package review, and Orchestrator as sole Mission authority. |
| Attention delivery | A deterministic watcher absorbs benign events; actionable wakes are durably appended before surfaced markers advance. ([architecture lines 9–29](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#L9-L29), [wake append lines 370–399](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-wake-lib.sh#L370-L399)) | **Adapt** | Add an Orchestrator-owned typed attention/outbox transaction; do not copy the file queue or terminal classifier. |
| Supervision backstop | Identity-matched watcher locks, beacons, bounded stale escalation, heartbeats, and harness turn-end hooks prevent blind supervision loss. ([turn-end lines 12–33](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/turnend-guard.md#L12-L33), [watcher health lines 73–100](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-wake-lib.sh#L73-L100)) | **Prototype** | Prove one Alfredo-owned, token-free attention/liveness/recovery loop against missed events, dead owners, restarts, and duplicate recovery. TUI hooks are not the target. |
| Reconciliation | Endpoint presence, busy evidence, attributable run state, and append-only status events are separate; weak or contradictory evidence becomes `unknown`. ([architecture lines 30–38](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#L30-L38), [spawn recovery lines 861–926](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-spawn.sh#L861-L926)) | **Keep / Adapt** | Keep PID-start identity, bounded requeue, receipts, and `outcome-unknown`; make observation precedence and typed unknown reasons explicit for Local Agent attention. |
| Recovery ownership | Relaunch requires proof that no live agent owns the task and reuses the recorded task/worktree; ambiguity preserves state. ([recovery playbook lines 19–35](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/stuck-crewmate-recovery/SKILL.md#L19-L35)) | **Adapt** | Preserve exact Mission/session/worktree identity before infrastructure recovery; keep Canonical Repair Action as a distinct reviewed child session. |
| Worktree launch | Spawn refuses a path that is not a real worktree root distinct from the primary checkout and serializes same-task creation. ([spawn lines 53–86](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-spawn.sh#L53-L86), [validation lines 835–851](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-spawn.sh#L835-L851)) | **Keep** | Retain isolated session worktrees, exact identities, and no-duplicate session effects as adapter-invariant requirements. |
| Worktree retirement | Teardown refuses dirty or unlanded work and ambiguous proof; forced discard requires explicit authority. ([teardown lines 1–88](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-teardown.sh#L1-L88)) | **Adapt** | Define a typed Alfredo retirement effect. `cleanup_eligible` is an input, not proof that evidence, dirty state, runner ownership, and worktree identity are safe to remove. |
| Worktree pool | Treehouse supplies reusable worktrees for most FirstMate backends. ([architecture lines 104–115](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#L104-L115)) | **Defer** | No measured Alfredo allocation bottleneck justifies leases and sanitization complexity; local-model execution dominates the sampled latency. |
| State projection | Append-only status is event history, while one bounded `fm-fleet-snapshot.v1` contract feeds human views and exposes provenance, omissions, and unknowns. ([fleet snapshot lines 1–58](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-fleet-snapshot.sh#L1-L58)) | **Keep** | Preserve Python-owned canonical snapshots, Activity Journal as non-authoritative history, and React as a renderer. |
| Terminal evidence | Bounded scrollback can diagnose but never override structured state because prompts, copied output, idle shells, and prose are untrusted. ([architecture lines 43–53](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#L43-L53)) | **Keep / Reject** | Keep bounded diagnostic output; reject terminal capture or direct typing as an Alfredo authority path. Typed Orchestrator actions own steering and cancellation. |
| Worker permissions | Common harnesses launch with permission/sandbox bypass or allow-all modes. ([spawn lines 413–458](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-spawn.sh#L413-L458)) | **Reject** | Every runner/adapter remains subordinate to allowed paths, command policy, Additional Path Grants, Bubblewrap, evidence validation, and Mission Commander review. |
| Prose authority | FirstMate uses natural-language dispatch and project modes, including optional `+yolo`. ([architecture lines 165–174](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#L165-L174)) | **Reject** | Do not replace typed proposals, exact-match auto-approval, expected revisions, or Workspace Queue decisions with a second prose policy source. |
| Adapter boundary | Harness mechanics/knowledge are separate from runtime-session backends; unverified adapters are refused and successful send requires a postcondition. ([adapter lines 27–48](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/harness-adapters/SKILL.md#L27-L48), [submission lines 159–163](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/harness-adapters/SKILL.md#L159-L163)) | **Adapt** | Define start/observe/cancel/recover/evidence/safety capabilities and empirical qualification for Alfredo runners; keep complete external harnesses optional. |
| Incomplete/exotic runtimes | Codex Desktop is refused without supported create/send/read/stop/status-return semantics; persistent secondmates add isolated supervisor homes. ([Codex App lines 7–18](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/codex-app-backend.md#L7-L18), [secondmates lines 135–164](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#L135-L164)) | **Defer** | Record adapter completeness as a blueprint constraint, but defer external-harness implementations and nested supervisors until a measured need exists. |

## Detailed findings

### Supervision

FirstMate's important contribution is not that supervision happens in Bash. It is the separation of:

1. cheap deterministic observation;
2. classification into benign or actionable attention;
3. durable delivery before the observer advances;
4. bounded backstops when the normal observer is stale;
5. an LLM or human turn only after actionable attention exists.

The queue-before-suppression rule is the strongest mechanism. The watcher records an actionable wake before advancing the surfaced marker, and a heartbeat can rediscover attention that a narrower event path missed. ([FirstMate architecture lines 9–29](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#L9-L29), [watcher heartbeat lines 983–1018](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-watch.sh#L983-L1018))

Alfredo should adapt this into canonical typed records, not a second file queue. An attention record should identify the Coding Workspace, Mission, Issue Slice or Ad Hoc Delegation, Local Agent session, observation kind, source, source identity/cursor, first-seen time, current disposition, and any superseding canonical state. Creating the record and advancing the observer cursor should be one Orchestrator transaction. The attention record may prompt the Mission Commander or a deterministic recovery action, but it cannot itself mark work complete, approve evidence, change authority, or launch a repair.

The independent liveness loop remains a prototype because the correct thresholds and signals for Alfredo's owned Ollama/command runners are not established by FirstMate's terminal-pane experience. The prototype should compare event-driven notification with a bounded reconciliation sweep and prove no model tokens are spent on a healthy no-change loop.

### Liveness and recovery

FirstMate distinguishes:

- endpoint existence;
- native or rendered busy evidence;
- an attributable validation/run state;
- append-only status events;
- declared external pause;
- supervisor-needed block;
- unknown ownership or state.

It then applies an evidence order rather than accepting the newest string. A matching current-code validation run can remain authoritative after the pane closes; a dead pane without a matching run becomes unknown rather than inheriting a stale status line. ([current-state architecture lines 30–38](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#L30-L38))

Alfredo already has the critical foundation: runner PID/start identity, bounded recovery count, Workstation Recovery Markers, exact-correlation replay, Canonical Repair Action lineage, and `outcome-unknown`. The adaptation is to state an explicit Local Agent reconciliation order:

1. canonical terminal/review state;
2. exact session and runner-operation identity;
3. live owner and operation evidence;
4. persisted runner result/evidence validation;
5. bounded diagnostic output or event history;
6. unknown when the stronger facts disagree or cannot be read.

No observation below canonical state can authorize a domain transition. Recovery must preserve the original session/worktree identity until owner death is proven. A separately approved repair remains a child session with inherited, integrity-bound context; it is not a silent resurrection of the failed session.

### Worktree and task lifecycle

FirstMate provides three useful lifecycle safeguards:

- launch only into a physically resolved worktree root distinct from the primary checkout;
- serialize same-task spawn so endpoint creation and metadata publication cannot race;
- refuse destructive teardown until dirty/unlanded work has been accounted for, or the human explicitly authorizes discard.

The teardown check is the largest concrete Alfredo gap found by this comparison. Alfredo can mark a session `cleanup_eligible` after accepted review, but that flag alone is not a proof that removal is safe. A planning-level retirement contract should require:

- exact Mission/session/worktree identity;
- no live runner owner or in-progress operation;
- accepted Evidence Package and registered review-safe artifacts preserved outside the worktree, or explicit discard authority;
- accepted review/PR-ready state where applicable, without equating Complete with merged;
- dirty/untracked state either represented in validated evidence or explicitly preserved/rejected;
- idempotent retry after partial cleanup;
- a durable retirement outcome that never claims cleanup after an ambiguous failure.

This is an adaptation of the invariant, not FirstMate's GitHub-specific landed-work algorithm. Alfredo's lifecycle ends at reviewed, PR-ready evidence unless a separate merge action is authorized.

Treehouse-style pooling is deferred. It should return only if a later measurement shows worktree allocation materially affects workspace readiness or goal-to-reviewed-evidence latency. Any future pool would need a stronger sanitizer because Alfredo also carries allowed-path, Bubblewrap, artifact, and Mission-isolation guarantees.

### State reconciliation

FirstMate's structured snapshot is a sound pattern: one machine-readable contract owns current classification, source/provenance, endpoint observation, bounded subsidiary data, omissions, truncation, and unknown reasons; human views render it rather than reparsing underlying files. ([fleet snapshot contract lines 1–58](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-fleet-snapshot.sh#L1-L58))

Alfredo already follows the deeper version of this rule: Python owns accepted state, Tauri transports typed data, and React renders acknowledged projections. Activity Journal records support reconstruction but do not own lifecycle. The implication is to add liveness and attention only through the existing snapshot/event boundary. A UI-local poll, terminal capture, or provider callback must never become a second current-state store.

FirstMate also demonstrates why bounded terminal output should be supplemental only: copied output, prompts, stale scrollback, and agent prose can contradict actual task state. This directly reinforces Alfredo's existing distinction between transient raw streams and durable finalized messages, evidence, summaries, and attributed outcomes.

### Safety and governance

FirstMate's high-level human authority rules align with Alfredo: the supervisor does not write project code, unlanded work is not discarded casually, and consequential delivery stays human-governed. The implementation boundary differs sharply.

FirstMate deliberately starts common coding harnesses in bypass/allow-all modes so unattended workers do not stall on per-tool prompts. ([spawn templates lines 413–458](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-spawn.sh#L413-L458)) Worktree isolation limits collisions, but it does not constrain process access to the host filesystem, environment, network, or unrelated commands.

Alfredo must reject that posture. A future runner or external harness adapter is subordinate to the same task packet, allowed paths, command policy, Additional Path Grants, minimal Bubblewrap view, sanitized environment, Evidence Package validation, review, and Mission Commander authority as current Local Agents. If a harness cannot operate inside that boundary, it is not an eligible Alfredo adapter.

Natural-language `+yolo` and project delivery modes are also rejected as replacements for Alfredo policy. Alfredo's typed exact-match auto-approval can keep safe work fast without moving authority into prose. The [workspace-selection and false-success diagnosis](./2026-07-24-workspace-selection-false-success-diagnosis.md) already showed why a successful-looking controller statement or transport result cannot stand in for an Orchestrator effect receipt; FirstMate's adapter rule that send success requires a verified postcondition corroborates, rather than changes, that closed finding.

### Observability

FirstMate offers direct terminal visibility, bounded capture, structured fleet status, and attention recaps. The useful Alfredo principle is **progressive disclosure backed by one structured state source**, not terminal panes as the primary product surface.

Alfredo already chose a prompt-dominant Agent Console beside a work-centered Mission Execution Tree. [Prototype the Coding Workspace-to-Mission journey](https://github.com/EricleungDK/Alfredo/issues/44) established on-demand connected Local Agent output, and [Prototype the Mission Execution Tree](https://github.com/EricleungDK/Alfredo/issues/46) already owns hierarchy, attention, summaries, expansion, review, repair, failure, governed actions, and constrained-width behavior. This research therefore proposes no duplicate live-output or attention-layout ticket.

FirstMate's evidence should instead constrain the data presented there:

- current activity must name its source and observation time;
- stale, dead, paused, blocked, and unknown must not collapse into one inactive state;
- terminal tail or provider output is visibly diagnostic, bounded, and non-authoritative;
- attention remains until canonical state supersedes it or a typed action resolves it;
- steering, interrupt, retry, cancel, repair, and discard remain Orchestrator actions with receipts.

### Coding-harness adapter boundaries

FirstMate separates two axes:

1. a runtime-session backend creates, observes, sends to, and tears down an endpoint;
2. a coding-harness adapter knows launch flags, busy/composer behavior, interrupt, exit, resume, dialogs, model discovery, and turn-end integration.

That separation is valuable, but Alfredo's primary abstraction level remains different. Alfredo owns the Local Agent loop and chooses a model/runner within it; FirstMate delegates the whole loop to a third-party coding harness.

The adaptable rule is a capability contract with empirical qualification. For Alfredo's current and future runners, the contract should cover:

- immutable runner and operation identity;
- start/observe/cancel/recover semantics;
- authoritative versus advisory observations;
- bounded output and evidence collection;
- model/provider availability and resource claims;
- command/filesystem authority conformance;
- completion and failure postconditions;
- restart and duplicate-effect behavior;
- capability/version provenance and conformance tests.

The FirstMate Codex Desktop boundary is an especially useful negative example: a visible thread and a manual ledger are not enough; the adapter is incomplete without supported create, send, bounded read, exact stop/archive, and lifecycle return semantics. ([Codex App backend boundary lines 1–29](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/codex-app-backend.md#L1-L29))

No external-harness implementation belongs in this map. The adapter completeness rule should be recorded in the blueprint as a constraint and explicitly deferred work, consistent with [Choose Alfredo's relationship to FirstMate](https://github.com/EricleungDK/Alfredo/issues/51#issuecomment-5093947435).

## Cross-cutting constraints for the modernization blueprint

1. **One authority:** the Orchestrator remains the only writer of accepted Mission, session, evidence, review, recovery, and retirement state.
2. **Observation is not authority:** process liveness, provider callbacks, terminal output, heartbeats, and status events may create or update attention, but cannot complete work or authorize effects.
3. **Queue before cursor:** actionable attention must be durably recorded in the same transaction, or earlier, than any observation cursor/suppression advance.
4. **Identity before recovery:** reconcile exact Mission, work, session, worktree, runner PID/start identity, and operation identity before relaunch; unknown ownership forbids duplicate execution.
5. **Typed unknowns:** distinguish unavailable, ambiguous, stale, paused, blocked, dead, and outcome-unknown. Do not convert missing evidence into success or generic inactivity.
6. **Bounded no-change supervision:** healthy polling/reconciliation consumes no model turn, uses bounded resource/time budgets, and does not emit user-facing progress.
7. **Recovery is not repair:** infrastructure recovery resumes the same bounded work only when safe; a Canonical Repair Action remains a separately reviewed child session.
8. **Retirement is a domain effect:** cleanup requires its own exact request, safety proof, idempotent outcome, and explicit discard authority. `cleanup_eligible` is an input, not the whole proof.
9. **Single projection contract:** Mission Work and the Mission Execution Tree consume the canonical typed snapshot/event stream; React never reconstructs lifecycle state from output.
10. **Safety is adapter-invariant:** no runner or harness capability may bypass allowed paths, command policy, Additional Path Grants, Bubblewrap, evidence validation, review, or Mission Commander authority.
11. **Local resource awareness stays separate:** liveness and attention must not infer GPU/model health from pane activity. Model residency, scheduling, concurrency, and VRAM decisions belong to [Identify the local-model optimization strategy](https://github.com/EricleungDK/Alfredo/issues/47).
12. **Backend-neutral requirements:** the supervision, reconciliation, attention, and retirement contracts must be implementable behind the current Python boundary and any staged Rust boundary so [Choose Alfredo's backend modernization architecture](https://github.com/EricleungDK/Alfredo/issues/49) can compare implementations without changing semantics.
13. **Fault evidence before adoption:** the prototype should inject missed completion notification, observer crash after record/before cursor, cursor advance failure, dead runner owner, PID reuse, stale activity, contradictory output, unavailable observation, restart, and duplicate recovery request.

## Linked Wayfinder tickets

The resolution published these two additions. They preserve every closed decision and avoid duplicating the in-flight Mission Execution Tree work.

### 1. [Prototype Alfredo's attention-driven Local Agent supervision loop](https://github.com/EricleungDK/Alfredo/issues/53)

**Type:** `wayfinder:prototype`  
**Question:** Can an Alfredo-owned supervision loop combine canonical Local Agent session state with independent runner/liveness observations, durably record actionable attention before observer cursors advance, recover after watcher/restart faults, and remain silent and token-free for healthy no-change work without granting observations Mission authority?

**Required comparison:** current Python Orchestrator behavior versus the disposable loop under the fault cases listed above; measure detection latency, duplicate-effect prevention, recovery correctness, no-change overhead, and projection clarity.

**Relationship:** linked from [Evaluate FirstMate patterns for Alfredo](https://github.com/EricleungDK/Alfredo/issues/52), natively blocked by it and the in-flight [Prototype the Mission Execution Tree](https://github.com/EricleungDK/Alfredo/issues/46), and a native blocker of [Lock the modernization blueprint for specification handoff](https://github.com/EricleungDK/Alfredo/issues/50). It should inform [Prototype a Rust Orchestrator vertical slice](https://github.com/EricleungDK/Alfredo/issues/48) as a contract but need not require Rust or reopen the Mission Execution Tree layout decision.

### 2. [Define the Local Agent session-worktree retirement contract](https://github.com/EricleungDK/Alfredo/issues/54)

**Type:** `wayfinder:grilling`  
**Question:** After accepted evidence, cancellation, failed work, or explicit discard, what exact runner-quiescence, worktree-identity, dirty-state, evidence/artifact-preservation, review/merge, authority, idempotency, and recovery proofs must the Orchestrator require before retiring a Local Agent session worktree?

**Why it is separate:** `cleanup_eligible` establishes eligibility after review but does not, by itself, define or execute safe retirement. FirstMate's teardown evidence makes the missing decision precise without deciding Alfredo's PR/merge policy for it.

**Relationship:** linked from and natively blocked by [Evaluate FirstMate patterns for Alfredo](https://github.com/EricleungDK/Alfredo/issues/52), and a native blocker of [Lock the modernization blueprint for specification handoff](https://github.com/EricleungDK/Alfredo/issues/50). It preserves the closed definition that Complete means reviewed and PR-ready, not merged.

### Explicitly no new ticket now

- **Mission Execution Tree live output/attention UI:** already owned by [Prototype the Mission Execution Tree](https://github.com/EricleungDK/Alfredo/issues/46).
- **False-success/effect acknowledgement:** already diagnosed by [Diagnose workspace selection and false-success action routing](https://github.com/EricleungDK/Alfredo/issues/45); the adapter postcondition evidence is corroboration.
- **Complete external coding-harness adapters:** explicitly out of scope under [Choose Alfredo's relationship to FirstMate](https://github.com/EricleungDK/Alfredo/issues/51#issuecomment-5093947435). Record the qualification contract as a blueprint constraint and defer implementation to a future effort.
- **Persistent second-tier supervisors or worktree pooling:** no measured need currently justifies a decision or prototype ticket. Revisit only if Mission scale or allocation measurements expose a concrete blocker.

## Source inventory

### FirstMate primary sources

- [Pinned FirstMate commit](https://github.com/kunchenguid/firstmate/commit/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499)
- [README](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/README.md)
- [Architecture](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md)
- [Operating contract](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/AGENTS.md)
- [Watcher](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-watch.sh)
- [Wake queue and watcher identity library](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-wake-lib.sh)
- [Current crew-state reconciliation](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-crew-state.sh)
- [Structured fleet snapshot](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-fleet-snapshot.sh)
- [Spawn implementation](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-spawn.sh)
- [Guarded teardown implementation](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-teardown.sh)
- [Stuck-worker recovery playbook](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/stuck-crewmate-recovery/SKILL.md)
- [Harness adapter reference](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/harness-adapters/SKILL.md)
- [Primary turn-end guard](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/turnend-guard.md)
- [tmux runtime backend](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/tmux-backend.md)
- [Codex App backend boundary](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/codex-app-backend.md)
- [Runtime-backend verification](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/verification/runtime-backends.md)
- [Supervision verification](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/verification/supervision.md)

### Alfredo primary sources

- [Domain context](../../CONTEXT.md)
- [Project architecture](../System/project_architecture.md)
- [Persistence schema](../System/database_schema.md)
- [API boundaries](../System/api_endpoints.md)
- [UX guidelines](../System/ux_guidelines.md)
- [Albert architecture](../../docs/albert-architecture.md)
- [Architecture and performance baseline](./2026-07-23-alfredo-architecture-performance-baseline.md)
- [Workspace selection and false-success diagnosis](./2026-07-24-workspace-selection-false-success-diagnosis.md)
- [FirstMate product-boundary research](./2026-07-27-firstmate-product-boundary-research.md)
- [Modernization Wayfinder map](https://github.com/EricleungDK/Alfredo/issues/41)
- [FirstMate relationship resolution](https://github.com/EricleungDK/Alfredo/issues/51#issuecomment-5093947435)

## Residual uncertainties

- The upstream evidence is pinned to a fast-moving untagged revision. Contract names and adapter facts may change before any future implementation.
- FirstMate's terminal/harness liveness signals are not direct evidence for Ollama process health, model residency, tool execution, or GPU/VRAM scheduling.
- The review establishes designed and tested mechanisms, but not comparative fault-injection results against Alfredo.
- The exact Alfredo worktree retirement implementation surface remains unchosen; this report establishes the missing contract decision, not a code location or backend language.
- The correct attention thresholds, escalation timing, and user-facing grouping require the proposed supervision prototype and the Mission Commander's separate Mission Execution Tree evaluation.
