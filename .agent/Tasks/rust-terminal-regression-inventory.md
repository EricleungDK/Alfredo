# Rust terminal: all-issue regression inventory

**Captured:** 2026-09-13 from live GitHub, 81 issues and six native parent/child lists.

GitHub remains authoritative. This inventory maps existing contract locations and identifies terminal gaps; it does **not** certify acceptance from titles, issue closure, old checkboxes or aggregate test counts. The [machine-readable inventory](rust-terminal-regression-inventory.json) retains source update timestamps, body hashes, all extracted acceptance checkboxes, native child order, related legacy tests and Rust test entrypoints. Parent user stories and native dependency edges still need a detailed final audit.

The current terminal uses a new task authority and does not replace Python stores. Desktop-specific layout, installation and Python-authority decisions require an explicit migration disposition against the current Rust rewrite request; they cannot silently become passed terminal requirements. GitHub issues #70, #72 and #73 are closed but retain unchecked source criteria. Their recorded implementation reports require reconciliation during final acceptance; neither status alone is proof.

## Verification evidence

- Current Rust terminal: 109 tests passed, three ignored live/subprocess fixtures; strict Clippy, locked release build and PTY passed. Evidence: [implementation report](../Reports/2026-09-13-rust-terminal-foundation.md).
- Earlier full legacy Python: 797 ran, three skips, no failures. Shared-provider compatibility: 65 Rust passed/one ignored and 83 focused Python ran/one skip. These remain dated evidence for unchanged legacy paths, not Rust parity.
- Earlier frontend: 324 passed/one failed; retirement cannot inspect `/proc/372/cwd` (EACCES). TypeScript passed. This is still an unresolved release gate.
- This audit reran the exact #83 and #86 maintenance regressions: five passed, 143 outside the selected filter, 66.90 s. Immediate/delayed acknowledgment and inherited Git true/false/input cases pass. Log: `/tmp/alfredo-issue-regression-maintenance.log`. The full release journey was not rerun. Package installation, browser/human acceptance, cross-platform release, and comparative production performance are not established.

## Issue map

| Issue | Kind / GitHub state | Rust evidence and remaining requirement |
|---|---|---|
| [#1 [PRD] Local Coding Agent MVP](https://github.com/EricleungDK/Alfredo/issues/1) | parent / closed | No terminal acceptance evidence. Roll up native children; parent stories/decisions also require detailed acceptance audit. Closure is not terminal completion. |
| [#2 01 — Mission State and Record Loading](https://github.com/EricleungDK/Alfredo/issues/2) | product / closed | `tasks.rs`. Tracker/PRD loading and authoritative dependency order are absent; task namespaces cover only isolated local task state. Accepted local task candidates now compose into verified isolated child baselines with conflict refusal and durable input provenance; native tracker graph parity and workspace branch publication remain incomplete. |
| [#3 02 — Review Locking and Assignment](https://github.com/EricleungDK/Alfredo/issues/3) | product / closed | `tasks.rs`. Approval/reset guards exist; slice contract locking, explicit reopen and assignment-note continuity are absent. |
| [#4 03 — Launch Local Agent Session](https://github.com/EricleungDK/Alfredo/issues/4) | product / closed | Explicit dispatch starts ready Approved tasks only after Accepted dependencies, with common worker/receipt guards and per-approval retry suppression. Native Issue Slice/session/task-packet parity, configured roles and governed cleanup eligibility remain incomplete. |
| [#5 04 — Command and Visibility Policy](https://github.com/EricleungDK/Alfredo/issues/5) | product / closed | `worker.rs`. Exact file/check sandbox policy exists; classified command approvals, visibility tiers and contextual expiring path grants are absent. |
| [#6 05 — Evidence Package Validation](https://github.com/EricleungDK/Alfredo/issues/6) | product / closed | `worker.rs`. Digest-bound diff/check evidence exists; full required-evidence schema, risks and proposed context updates are absent. |
| [#7 06 — Frontier Review and Repair Policy](https://github.com/EricleungDK/Alfredo/issues/7) | product / closed | `worker.rs`. Readable check/diff/output review and exact inspected-task shorthand validate evidence; explicit receipt-linked repair inherits policy with fresh approval and verified prior context. Automatic repair routing, limited approval, architect escalation and integrated timeline remain absent. Accepted results now support verified local review-branch handoff; remote publication and active-branch merge remain separate. |
| [#8 07 — Mission Record Generation](https://github.com/EricleungDK/Alfredo/issues/8) | product / closed | `tasks.rs`. Searchable saved task receipt activity and conversation snapshots exist; generated mission records, actor/time attribution and integrated action chronology remain absent. |
| [#9 08 — PR Readiness and GitHub Fallback](https://github.com/EricleungDK/Alfredo/issues/9) | product / closed | No terminal acceptance evidence. PR-ready lifecycle, branch/PR summaries and authenticated/manual publishing fallback are absent. |
| [#10 [PRD] Alfredo Agent Workstation](https://github.com/EricleungDK/Alfredo/issues/10) | parent / open | No terminal acceptance evidence. Roll up native children; parent stories/decisions also require detailed acceptance audit. Closure is not terminal completion. |
| [#11 20 — Ship the Alfredo Npm Workstation Entrypoint](https://github.com/EricleungDK/Alfredo/issues/11) | product / open | `cli.rs`, `release_smoke.py`. Native Linux archive, installed-binary PTY acceptance and noninteractive storage/model/Git/tool diagnostics exist; OS/glibc matrix, public entrypoint, license review and desktop npm compatibility decision remain unverified. |
| [#12 21 — Add Headless Alfredo Cli Grammar](https://github.com/EricleungDK/Alfredo/issues/12) | product / closed | `cli.rs`. Rust has interactive CLI flags only; headless run/review/agents grammar is absent. |
| [#13 22 — Build the Prompt Dominant Workstation Shell](https://github.com/EricleungDK/Alfredo/issues/13) | product / closed | `layout.rs`, `terminal_smoke.py`. Terminal layout/composer exists; complete scope/status boundary and simultaneous task/conversation layout equivalence remain incomplete. |
| [#14 23 — Project Live Agent Workstation Cards](https://github.com/EricleungDK/Alfredo/issues/14) | product / closed | `worker.rs`, `tasks.rs`, `layout.rs`, `terminal_smoke.py`. Selected task status/progress/evidence and bounded live check stdout/stderr exist; nested delegation tree, attention priority, role, activity trail and selected-only tool-output subscription are absent. |
| [#15 24 — Expand Workstation Cards for Operational Detail](https://github.com/EricleungDK/Alfredo/issues/15) | product / closed | `worker.rs`, `layout.rs`, `terminal_smoke.py`. Selected-task evidence is inspectable; live tool excerpts, pin/filter/sort and durable full activity are incomplete. |
| [#16 25 — Route the First Consequential Workstation Action](https://github.com/EricleungDK/Alfredo/issues/16) | product / closed | `tasks.rs`, `worker.rs`. Typed revision/correlation task actions exist; natural-language receipt-bound action routing, all action classes, actor/reason attribution and journal are incomplete. |
| [#17 26 — Cover the Governed Workstation Action Family](https://github.com/EricleungDK/Alfredo/issues/17) | product / closed | `tasks.rs`, `worker.rs`. Typed revision/correlation task actions exist; natural-language receipt-bound action routing, all action classes, actor/reason attribution and journal are incomplete. |
| [#18 27 — Persist Alfredo Workstation Continuity](https://github.com/EricleungDK/Alfredo/issues/18) | product / closed | `workstation.rs`, `missions.rs`, `conversations.rs`, `terminal_smoke.py`. Native explicit Resume/Start New, atomic identity collision checks, legacy resume, saved workspace/mission choices, in-app quiescent handoff and task/conversation/view restart continuity exist. Handoff preserves source state on cancellation/open/save failure and isolates old events. Complete Workspace Session and attributed mission-formation receipt chains remain incomplete. |
| [#19 28 — Validate Alfredo Accessibility and Responsive Use](https://github.com/EricleungDK/Alfredo/issues/19) | product / open | `layout.rs`, `terminal_smoke.py`, `editor.rs`. Keyboard/reflow tests exist; terminal assistive-technology, palette contrast and human acceptance remain unverified. |
| [#20 29 — Add Alfredo Release Seam Verification](https://github.com/EricleungDK/Alfredo/issues/20) | product / closed | `terminal_smoke.py`. PTY coding journey covers a subset; complete installed product, planning/retirement/restart/fallback and human release gates are unverified. |
| [#21 [PRD] Alfredo Console-First Workstation Redesign](https://github.com/EricleungDK/Alfredo/issues/21) | parent / closed | No terminal acceptance evidence. Roll up native children; parent stories/decisions also require detailed acceptance audit. Closure is not terminal completion. |
| [#22 01 — Console First Workstation Layout](https://github.com/EricleungDK/Alfredo/issues/22) | product / closed | `layout.rs`, `terminal_smoke.py`. Terminal layout/composer exists; complete scope/status boundary and simultaneous task/conversation layout equivalence remain incomplete. |
| [#23 02 — Inline Command Execution Cards](https://github.com/EricleungDK/Alfredo/issues/23) | product / closed | `worker.rs`. Approved worker check receipt exists; general Shell command cards and durable summarized command history are absent. |
| [#24 03 — Inline Approvals and Contextual Path Grants](https://github.com/EricleungDK/Alfredo/issues/24) | product / closed | `worker.rs`. Exact file/check sandbox policy exists; classified command approvals, visibility tiers and contextual expiring path grants are absent. |
| [#25 04 — Mission Work Pane Active Workstation Cards](https://github.com/EricleungDK/Alfredo/issues/25) | product / closed | `mission_work.rs`, `work_navigation.rs`, `work_tree_ui.rs`, `terminal_smoke.py`. Canonical Plan/manual groups, nested repair ancestry, dependency edges, exact selection, current model/run observations, evidence and recent task receipts now appear in the native Mission Work tree. Full role/delegation parity, attention ordering, selected-only output subscription and broader accessibility remain incomplete. |
| [#26 05 — Issue Assignment Board Projection and Local Navigation](https://github.com/EricleungDK/Alfredo/issues/26) | product / closed | Per-conversation-set restart view/selection/search continuity, terminal task search, exact-ID navigation, visible-row selection, blocker/readiness explanations and receipt-bound model/run projection exist. Native Issue Slice board identity, configured owner/workstation linkage and explicit Conversation Scope-change affordances remain incomplete. |
| [#27 06 — Governed Issue Assignment and Launch Actions](https://github.com/EricleungDK/Alfredo/issues/27) | product / closed | `tasks.rs`, `worker.rs`. Typed revision/correlation task actions exist; natural-language receipt-bound action routing, all action classes, actor/reason attribution and journal are incomplete. |
| [#28 07 — Activity Journal and Durable Transcript Boundaries](https://github.com/EricleungDK/Alfredo/issues/28) | product / closed | `tasks.rs`. Searchable saved task receipt activity and conversation snapshots exist; generated mission records, actor/time attribution and integrated action chronology remain absent. |
| [#29 08 — Restart Continuity through the Release Seam](https://github.com/EricleungDK/Alfredo/issues/29) | product / closed | `workstation.rs`, `missions.rs`, `conversations.rs`, `terminal_smoke.py`. Native explicit Resume/Start New, atomic identity collision checks, legacy resume, saved workspace/mission choices, in-app quiescent handoff and task/conversation/view restart continuity exist. Handoff preserves source state on cancellation/open/save failure and isolates old events. Complete Workspace Session and attributed mission-formation receipt chains remain incomplete. |
| [#30 09 — Responsive and Accessible Workstation Hardening](https://github.com/EricleungDK/Alfredo/issues/30) | product / closed | `layout.rs`, `terminal_smoke.py`, `editor.rs`. Keyboard/reflow tests exist; terminal assistive-technology, palette contrast and human acceptance remain unverified. |
| [#31 [PRD] Local Coding Agent MVP Development Roadmap](https://github.com/EricleungDK/Alfredo/issues/31) | parent / closed | No terminal acceptance evidence. Roll up native children; parent stories/decisions also require detailed acceptance audit. Closure is not terminal completion. |
| [#32 01 — Lifecycle State Cleanup and Reopen Controls](https://github.com/EricleungDK/Alfredo/issues/32) | product / closed | `tasks.rs`. Approval/reset guards exist; slice contract locking, explicit reopen and assignment-note continuity are absent. |
| [#33 02 — Textual TUI Mission Board](https://github.com/EricleungDK/Alfredo/issues/33) | product / closed | `mission_work.rs`, `work_navigation.rs`, `work_tree_ui.rs`, `terminal_smoke.py`. Native Mission Work tree now groups canonical Plans and repairs, preserves exact task anchors, and reuses readiness/replacement provenance. GitHub Issue Graph import, complete issue scope changes and full desktop readiness parity remain absent. |
| [#34 03 — Agent Model Configuration Registry](https://github.com/EricleungDK/Alfredo/issues/34) | product / closed | `models.rs`. Installed model catalog/selection exists; provider-neutral role registry, config validation and assignment eligibility are absent. |
| [#35 04 — TUI Assignment and Launch Controls](https://github.com/EricleungDK/Alfredo/issues/35) | product / closed | Installed model catalog and /assign ID MODEL now provide durable unstarted-worker overrides, fresh approval, common transaction guards and actual reassigned worker dispatch. Native Issue Slice identity, configured agent/profile registry and qualified role assignment remain incomplete. |
| [#36 05 — Fake Local Agent Runner](https://github.com/EricleungDK/Alfredo/issues/36) | product / closed | `worker.rs`. Real model-generated plans/checks are tested via HTTP fixtures; configurable fake/command runner adapters and full task packets are absent. |
| [#37 06 — Command Backed Local Agent Runner](https://github.com/EricleungDK/Alfredo/issues/37) | product / closed | `worker.rs`. Real model-generated plans/checks are tested via HTTP fixtures; configurable fake/command runner adapters and full task packets are absent. |
| [#38 07 — Automated Evidence Collection](https://github.com/EricleungDK/Alfredo/issues/38) | product / closed | `worker.rs`. Digest-bound diff/check evidence exists; full required-evidence schema, risks and proposed context updates are absent. |
| [#39 08 — TUI Review and Repair Loop](https://github.com/EricleungDK/Alfredo/issues/39) | product / closed | `worker.rs`. Readable check/diff/output review and exact inspected-task shorthand validate evidence; explicit receipt-linked repair inherits policy with fresh approval and verified prior context. Automatic repair routing, limited approval, architect escalation and integrated timeline remain absent. Accepted results now support verified local review-branch handoff; remote publication and active-branch merge remain separate. |
| [#40 09 — PR Readiness from TUI](https://github.com/EricleungDK/Alfredo/issues/40) | product / closed | No terminal acceptance evidence. PR-ready lifecycle, branch/PR summaries and authenticated/manual publishing fallback are absent. |
| [#41 [Wayfinder] Chart Alfredo's reliable, observable, and faster modernization](https://github.com/EricleungDK/Alfredo/issues/41) | parent / closed | No terminal acceptance evidence. Roll up native children; parent stories/decisions also require detailed acceptance audit. Closure is not terminal completion. |
| [#42 Capture real Alfredo workflow failures](https://github.com/EricleungDK/Alfredo/issues/42) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#43 Measure Alfredo's current architecture and performance baseline](https://github.com/EricleungDK/Alfredo/issues/43) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#44 Prototype the Coding Workspace-to-Mission journey](https://github.com/EricleungDK/Alfredo/issues/44) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#45 Diagnose workspace selection and false-success action routing](https://github.com/EricleungDK/Alfredo/issues/45) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#46 Prototype the Mission Execution Tree](https://github.com/EricleungDK/Alfredo/issues/46) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#47 Identify the local-model optimization strategy](https://github.com/EricleungDK/Alfredo/issues/47) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#48 Prototype a Rust Orchestrator vertical slice](https://github.com/EricleungDK/Alfredo/issues/48) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#49 Choose Alfredo's backend modernization architecture](https://github.com/EricleungDK/Alfredo/issues/49) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#50 Define Alfredo's project-start Wayfinder routing](https://github.com/EricleungDK/Alfredo/issues/50) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#51 Choose Alfredo's relationship to FirstMate](https://github.com/EricleungDK/Alfredo/issues/51) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#52 Evaluate FirstMate patterns for Alfredo](https://github.com/EricleungDK/Alfredo/issues/52) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#53 Prototype Alfredo's attention-driven Local Agent supervision loop](https://github.com/EricleungDK/Alfredo/issues/53) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#54 Define the Local Agent session-worktree retirement contract](https://github.com/EricleungDK/Alfredo/issues/54) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#55 Measure whether Rust improves Alfredo desktop startup and rendered workflows](https://github.com/EricleungDK/Alfredo/issues/55) | research / closed | No terminal acceptance evidence. Historical research/prototype question; retain as evidence context. Requalify any performance or architecture claim for the Rust terminal. |
| [#56 [PRD] Alfredo reliable, observable, and faster modernization](https://github.com/EricleungDK/Alfredo/issues/56) | parent / closed | No terminal acceptance evidence. Roll up native children; parent stories/decisions also require detailed acceptance audit. Closure is not terminal completion. |
| [#57 01 — Establish an acknowledged Coding Workspace](https://github.com/EricleungDK/Alfredo/issues/57) | product / closed | `workstation.rs`, `missions.rs`, `conversations.rs`, `terminal_smoke.py`. Native explicit Resume/Start New, atomic identity collision checks, legacy resume, saved workspace/mission choices, in-app quiescent handoff and task/conversation/view restart continuity exist. Handoff preserves source state on cancellation/open/save failure and isolates old events. Complete Workspace Session and attributed mission-formation receipt chains remain incomplete. |
| [#58 02 — Resume or start a Mission with restart continuity](https://github.com/EricleungDK/Alfredo/issues/58) | product / closed | `workstation.rs`, `missions.rs`, `conversations.rs`, `terminal_smoke.py`. Native explicit Resume/Start New, atomic identity collision checks, legacy resume, saved workspace/mission choices, in-app quiescent handoff and task/conversation/view restart continuity exist. Handoff preserves source state on cancellation/open/save failure and isolates old events. Complete Workspace Session and attributed mission-formation receipt chains remain incomplete. |
| [#59 03 — Bind conversational action claims to Orchestrator receipts](https://github.com/EricleungDK/Alfredo/issues/59) | product / closed | `understanding.rs`, `workstation.rs`, `terminal_smoke.py`. Native Wayfinder entry/draft/confirmation responses identify exact scope receipts and distinguish historical replay from current scope. Explicit /plan captures pinned repository and scope bindings in task v9 receipts; task execution still requires canonical approval/evidence. General natural-language action routing, all action classes and structured per-message capability attribution remain incomplete. |
| [#60 04 — Enter Wayfinder through the Shared Understanding Gate](https://github.com/EricleungDK/Alfredo/issues/60) | product / closed | `understanding.rs`, `workstation.rs`, `terminal_smoke.py`. Native Wayfinder routing now durably enters Chart/Work-through before conversational inference, reuses the repository flow across missions/restart, preserves read-only entry exclusions, and handles four-field scope drafts plus exact-revision Commander confirmation without follow-on inference/task actions. v2 scope receipts migrate v1 with exact backups; cancellation cannot abandon a pending routing write. Full Mission Draft/graph/skill execution, desktop/native migration and structured conversation-source attribution remain incomplete. |
| [#61 05 — Complete keyboard-first conversational Mission formation](https://github.com/EricleungDK/Alfredo/issues/61) | product / closed | `session.rs`, `conversations.rs`, `terminal_smoke.py`. Native history now retains structured model/Wayfinder source labels and optional scope receipt references, with v1/v2 exact-backup migration and no inferred legacy source. Cursor editing, prompt history and slash completion work. Unified Workspace/Mission/task action chronology, @-capability completion, exact persisted reading anchors across restart/reflow and complete formation/accessibility acceptance remain incomplete. Live reading anchors now survive stream growth and view/session switches; controlled HTTP/PTY and 66,000-line rendering regressions pass. |
| [#62 06 — Measure production startup and rendered-action cohorts](https://github.com/EricleungDK/Alfredo/issues/62) | product / closed | `live.rs`. Bounded server load/prompt/generation metrics and two live READY observations are available; repeated paired production cohorts, complete UI stage decomposition, distributions and rollback-qualified speed evidence remain absent. |
| [#63 07 — Build the inspectable Mission Execution Tree](https://github.com/EricleungDK/Alfredo/issues/63) | product / closed | `mission_work.rs`, `work_navigation.rs`, `work_tree_ui.rs`, `terminal_smoke.py`. Canonical Plan/manual groups, nested repair ancestry, dependency edges, exact selection, current model/run observations, evidence and recent task receipts now appear in the native Mission Work tree. Full role/delegation parity, attention ordering, selected-only output subscription and broader accessibility remain incomplete. |
| [#64 08 — Operate and archive work through the Mission Execution Tree](https://github.com/EricleungDK/Alfredo/issues/64) | product / closed | `mission_work.rs`, `work_navigation.rs`, `work_tree_ui.rs`, `terminal_smoke.py`. Cancel/review, linked repair proposals and canonical readiness/replacement explanations are accessible through task-tree inspection. Identity-preserving archive/restore and complete governed tree actions remain absent. |
| [#65 09 — Supervise and recover Local Agent runners deterministically](https://github.com/EricleungDK/Alfredo/issues/65) | product / closed | `run_boundary.rs`, `check_recovery.rs`, `recovery_terminal_smoke.py`. Owner locks, intact-final-evidence recovery and verified terminal check checkpoints permit explicit Failed interruption without replay. Exact child/process-group quiescence, attention ledger and one-shot safe automatic runner recovery remain absent. |
| [#66 10 — Reserve and prove preservation for every Retirement Unit](https://github.com/EricleungDK/Alfredo/issues/66) | product / closed | No terminal acceptance evidence. No preservation reservations, reconstructable snapshots, retirement phases/grace, aggregate storage budget, pin/export/discard or quiescence proof. |
| [#67 11 — Retire completed, cancelled, failed, and reviewed work safely](https://github.com/EricleungDK/Alfredo/issues/67) | product / closed | No terminal acceptance evidence. No preservation reservations, reconstructable snapshots, retirement phases/grace, aggregate storage budget, pin/export/discard or quiescence proof. |
| [#68 12 — Manage retirement storage and blocked outcomes](https://github.com/EricleungDK/Alfredo/issues/68) | product / closed | No terminal acceptance evidence. No preservation reservations, reconstructable snapshots, retirement phases/grace, aggregate storage budget, pin/export/discard or quiescence proof. |
| [#69 13 — Run local models through instrumented Profiles and Leases](https://github.com/EricleungDK/Alfredo/issues/69) | product / closed | `inference_runtime.rs`, `inference_profiles.rs`. Optional native diagnostics now bind actual payload/profile/prefix hashes and per-request selected digest, quantization, residency/context observations while holding client admission. Ordinary production turns remain unchanged. Explicit token headroom, Mission-attributed inference audit, qualified resident affinity and verified upstream runtime pinning remain incomplete; default capacity2 is not full legacy single-lease parity. |
| [#70 14 — Qualify and promote Local Inference Profiles](https://github.com/EricleungDK/Alfredo/issues/70) | product / closed | `qualification_runner.rs`, `qualification_oracle.rs`, `qualification_cli_smoke.py`, `qualification.rs`. Bounded paired baseline/context-candidate diagnostics now cover four native governed scenarios with canonical review, independent parent checks, exact source/request identities, decomposed timings and no-replay reports. Deterministic installed CLI covers8 cases/20 requests and tampering refusal. The complete11-kind family, broad repeated model quality, exact token/template headroom, runtime withdrawal/pinning, promotion and rollback remain incomplete. |
| [#71 15 — Expand host execution behind a Python request/receipt seam](https://github.com/EricleungDK/Alfredo/issues/71) | product / closed | `run_boundary.rs`, `check_recovery.rs`, `recovery_terminal_smoke.py`. Shared Rust host-effect provider and native request-bound terminal check checkpoints are exercised, with no-replay explicit interruption recovery and real process-death/refusal tests. Full protocol, uncertain-effect/process reconciliation, Shell and packaged rollback parity remain incomplete. |
| [#72 16 — Shadow Rust execution receipts against Python](https://github.com/EricleungDK/Alfredo/issues/72) | product / closed | `worker.rs`, `recovery.rs`. Shared Rust host-effect provider is exercised; full new-authority current/previous protocol, crash reconciliation, Shell and packaged rollback parity remain incomplete. |
| [#73 17 — Cut Local Agent host effects over to Rust safely](https://github.com/EricleungDK/Alfredo/issues/73) | product / closed | `worker.rs`, `recovery.rs`. Shared Rust host-effect provider is exercised; full new-authority current/previous protocol, crash reconciliation, Shell and packaged rollback parity remain incomplete. |
| [#74 18 — Cut Shell host effects over to Rust safely](https://github.com/EricleungDK/Alfredo/issues/74) | product / closed | `worker.rs`, `recovery.rs`. Shared Rust host-effect provider is exercised; full new-authority current/previous protocol, crash reconciliation, Shell and packaged rollback parity remain incomplete. |
| [#75 19 — Verify the packaged modernized workstation and fallback boundary](https://github.com/EricleungDK/Alfredo/issues/75) | product / closed | `terminal_smoke.py`. PTY coding journey covers a subset; complete installed product, planning/retirement/restart/fallback and human release gates are unverified. |
| [#79 [Automation] Daily conservative dead-code cleanup](https://github.com/EricleungDK/Alfredo/issues/79) | automation / open | No terminal acceptance evidence. Operational cleanup policy/run, not a coding-agent product feature. This rewrite does not trigger, publish, or change automation state. |
| [#83 [Maintenance repair] Wait for Workspace Queue acknowledgment in App test](https://github.com/EricleungDK/Alfredo/issues/83) | maintenance / closed | No terminal acceptance evidence. Legacy test-only acknowledgment repair: exact delayed/immediate regression rerun in this audit; no corresponding React port implied. |
| [#86 [Maintenance repair] Isolate release-seam Git fixture from core.autocrlf](https://github.com/EricleungDK/Alfredo/issues/86) | maintenance / closed | No terminal acceptance evidence. Legacy fixture line-ending repair: exact inherited true/false/input regression rerun; full release journey still has the recorded retirement inspection failure. |
| [#89 [Dead-code run] 2026-09-11](https://github.com/EricleungDK/Alfredo/issues/89) | automation / closed | No terminal acceptance evidence. Operational cleanup policy/run, not a coding-agent product feature. This rewrite does not trigger, publish, or change automation state. |
| [#90 [Dead-code run] 2026-09-12](https://github.com/EricleungDK/Alfredo/issues/90) | automation / closed | No terminal acceptance evidence. Operational cleanup policy/run, not a coding-agent product feature. This rewrite does not trigger, publish, or change automation state. |
| [#91 [Dead-code run] 2026-09-13](https://github.com/EricleungDK/Alfredo/issues/91) | automation / open | No terminal acceptance evidence. Operational cleanup policy/run, not a coding-agent product feature. This rewrite does not trigger, publish, or change automation state. |

## Implementation order from the gaps

1. Complete workspace/mission/task-view and attributed journal continuity (#18, #28, #29, #58). Named conversation snapshots now restore model/draft/transcript/selected conversation and interrupt saved active turns without replay.
2. Complete exact process/effect reconciliation and repair lineage (#7, #39, #64, #65), then integrate reviewed dependencies and native issue identities (#2, #4, #26, #35).
3. Implement preservation reservations and retirement/storage lifecycle (#66–68) before sustained worker workloads.
4. Add governed role/profile scheduling and repeated reviewed-outcome measurements (#34, #69, #70); single successful Ollama calls cannot support a speed claim.
5. Finish mission formation, action chronology and command discovery (#57, #59–61), then rerun full issue-scoped and installed release gates (#11, #19, #20, #75).

No issue, pull request, automation state or publication was changed by this inventory.

Plan continuity update (2026-09-15): native complete task-plan previews now persist
through restart and quiescent workspace/mission handoff in conversation v5, with
original revision guards and exact legacy backups. Installed PTY verifies restored
review before explicit save and no inference replay. This advances #18/#29/#58/#61
continuity but does not replace the full Mission Draft/Issue Graph contract.
[Evidence](../Reports/2026-09-15-plan-continuity.json).

Capability completion update (2026-09-15): #61 now has native @wayfinder completion,
dismissal, input preservation and receipt-backed entry/draft/confirmation evidence,
including the corrected minimum-terminal layout. General capability/skill routing
and the full conversational mission journey remain incomplete.
[Evidence](../Reports/2026-09-15-wayfinder-capability.json).

New-project update (2026-09-15): #57 native Create now establishes a disclosed empty
root commit, enabling bounded committed planning context and the first isolated
worker without an external Git setup step. Existing targets/unborn repositories
are preserved. This does not complete the versioned Workspace Session receipt
contract or full Mission formation parity.
[Evidence](../Reports/2026-09-15-new-workspace-baseline.json).


Recovery update (2026-09-20): synced task/run/baseline markers allow explicit
recovery of a stopped worker as Failed only when no check-launch intent or final
evidence exists. Real subprocess crash and refusal/idempotency/repair tests pass;
native suite 176 passed / 5 ignored, strict Clippy and installed PTY pass. Partial
work remains retained. Possible check launch remains unknown without ordinary
terminal evidence; this does not prove helper quiescence or authorize retirement.
[Evidence](../Reports/2026-09-20-precheck-recovery.json).


Terminal panels update (2026-09-20): Models and Activity content remain reachable
at 32×10; shared task-panel logical-line slicing reaches evidence past 65,535
rows. Page navigation clamps after end/resize. Native 178 passed / 5 ignored,
strict Clippy and installed PTY pass. This advances terminal usability coverage;
full chronology and release acceptance remain open.
[Evidence](../Reports/2026-09-20-terminal-panels.json).


Evidence-render update (2026-09-20): cached wrapping of immutable evidence and
visible-page cloning reduce the optimized synthetic 100-redraw workload from
539.962 ms to 17.425 ms. Unicode/style/resize buffer equivalence and work-count
regressions pass; native 180 passed / 6 ignored, strict Clippy and installed
PTY pass. This does not qualify model or end-to-end latency.
[Evidence](../Reports/2026-09-20-evidence-rendering.json).


Acceptance-contract update (2026-09-20): #4 native planned task packets now retain
bounded explicit acceptance criteria through review, save, restart, worker requests
and separately approved repair descendants. Task v10/conversation v6 migrations
retain exact originals; absent legacy criteria stay unrecorded. Fixed criteria
displacing live progress and discovery omitting the new schema. Native 183 passed
/ 6 ignored, strict Clippy and installed PTY pass. Full mission formation,
criterion-level review and chronology remain open.
[Evidence](../Reports/2026-09-20-plan-acceptance.json).


Criterion review update (2026-09-20): #7/#39 native acceptance/rejection now supports
explicit reasons and ordered per-criterion evidence notes via /review. Contracted
acceptance requires all met; historical boolean decisions gain no inferred notes.
Task v11 migration, exact replay, evidence tamper refusal, visible acknowledgment
and separately approved repair context pass. Native 185 passed / 6 ignored,
strict Clippy and installed PTY pass. Limited approval, human escalation and tiered
automatic routing remain unfinished.
[Evidence](../Reports/2026-09-20-criterion-review.json).

## 2026-09-20 explicit review outcomes

Native review now records all five outcomes, including limited approval and a
human-review hold that requires explicit resolution. Both approvals require the
original successful check and all recorded criteria met. Schema v12 preserves
exact older-store backups and historical reviews. Regression coverage exercises
all outcomes with isolated workers, failed-run approval bypass refusal, dependent
dispatch blocking, note replacement and the installed terminal journey.

Validation: 189 native tests passed, six intentionally ignored; final affected
suites, strict Clippy and installed release smoke pass. MIT and third-party notices
are bundled. Exact candidate/hash evidence is in
`.agent/Reports/2026-09-20-review-outcomes.json`. Automated tiered repair/routing,
formation/chronology, recovery/retirement and production qualification remain open.

## 2026-09-20 repair-hold regression

Reproduced and fixed a sibling-repair bypass while an existing child awaited human
review. New proposals refuse under the task lock. Historical v12 receipts remain
readable and exactly replayable; no schema migration. Real isolated-worker tests
cover restart, unchanged storage on refusal, historical replay and explicit hold
resolution followed by a separately approved repair. Native 190 passed / 6 ignored,
strict Clippy and installed PTY pass. Exact candidate and hashes:
[repair-hold report](../Reports/2026-09-20-repair-hold.json).
Tiered routing and full production qualification remain open.

## 2026-09-20 typed risk escalation

GitHub #7's critical/security/merge-risk route now operates on explicit native
reviews. Any declared risk on a nonapproving decision holds the task for human
review and blocks repair/dependent dispatch. Approval carrying risk refuses;
explicit resolution retains original successful-check and criterion requirements.
Task v13 preserves exact old-store backups and unclassified reviews; older schemas
cannot claim classified risk. Activity and evidence show the persisted escalation.

Validation: 192 native tests passed / six ignored, strict Clippy and installed PTY
pass. The real worker matrix covers all three risk classes on successful and failed
runs, restart/retry/conflicts, blocked bypasses and resolved dependent dispatch.
[Evidence](../Reports/2026-09-20-risk-escalation.json). Same/fresh-agent and Architect
routing, automatic model review, full formation/chronology, recovery/retirement and
production launch qualification remain open.

## 2026-09-20 Local Agent conversation continuity

New worker evidence binds a bounded retained user/assistant conversation and
mission-local agent identity. First repair sends the prior exchange; second/later
terminal rejection in ancestry starts a fresh agent with current repair evidence.
Needs repair does not count as rejection. Current policy and fresh approval remain
required. Missing/corrupt referenced history refuses before run claim; legacy or
incomplete history, changed models and budget rollover are explicitly fresh.

Native 197 passed / six ignored, strict Clippy and installed PTY pass. Real HTTP
regressions verify exact history, identity, restart and refusal paths. An independent
subagent reviewed correctness and added edge coverage. Artifact transcript schema1;
task13/conversation6/scope2 unchanged. [Evidence](../Reports/2026-09-20-agent-continuity.json).
Automatic review-triggered repair proposal/launch and Architect routing, model
qualification, full formation/chronology, recovery/retirement and launch remain open.

## 2026-09-20 atomic review-to-repair proposals

No-risk NeedsRepair/Rejected reviews now atomically record the parent outcome and
an inherited Proposed repair child. Fresh approval is still required. Risk holds
create no child. Parent and child Activity searches expose the receipt, and parent
readiness links the child. Task v14 preserves exact backups and old review meaning.

Native 200 passed / six ignored, strict Clippy and installed PTY pass. Subagent
coverage verifies complete atomicity/refusal/replay and actual continued/fresh model
requests, including a failed parent with an already human-held child. The installed
journey verifies the single review command and absence of immediate inference.
[Evidence](../Reports/2026-09-20-atomic-review-repair.json).

The design review identified an outstanding graph gap: accepting a repair does not
resolve dependents referencing the rejected original task. Governed replacement,
Architect routing and the broader product/launch gates remain unfinished.


Observed chronology update (2026-09-20), #8/#28/#61: canonical task receipt references now interleave at saved local observation boundaries, with exact identity verification and no model-input contamination. Initial history stays in Activity. This does not yet provide originating command causality, actor/time attribution or unified Workspace/Mission/planner chronology.


Stable reading update (2026-09-20), #61: schema9 block keys retain viewed message/receipt identity across earlier insertion, retry growth, resize and restart. Direct UI comparison reproduces numeric-only displacement. Exact character-position reflow remains open. The full command lifecycle is specified in `native-command-chronology.md` and remains a required implementation.


Durable command update (2026-09-20), #8/#28/#61: prepared task/scope/run/branch/recovery intents now save before dispatch and retain origin across explicit restored retry; exact canonical receipts supply phases. Installed failure injection proves no task creation when intent saving fails. Full causal adapters remain incomplete as recorded in `native-command-chronology.md`.


Worker lifecycle update (2026-09-20), #8/#28/#61: explicit Run commands now retain separate Start/Finish phases at origin, verified against canonical run identity and digest. Real worker and installed regressions pass; automatic dispatch and remaining command adapters remain open.


## 2026-09-26 shared inference checkpoint

[Shared admission evidence](../Reports/2026-09-26-shared-inference-admission.json) records cross-process capacity/priority, real HTTP, actual worker cancellation and installed two-terminal coverage. Final327 tests pass / 7 ignored, strict Clippy and installed archive pass. This advances #69 scheduling only; profile/headroom/mission audit/affinity and #70 qualification remain incomplete. [Next qualification plan](native-inference-qualification.md).


## 2026-09-27 native diagnostic checkpoint

[Qualification evidence](../Reports/2026-09-27-native-inference-qualification.json)
advances #69/#70 with actual request/profile/source identity, bounded per-request
runtime observations and four governed native fixtures. Full native 357 / final
focused 48 / installed 5 pass; these counts overlap and are not additive. Corrected
live diagnostics complete 8 cases and 16 generation requests, with 1 canonical accepted
outcome. Repair seed refusals are not live repair coverage. No profile is promoted;
token headroom, runtime pinning and broad role quality remain unqualified.
[Mission Work tree](native-mission-work-tree.md) is implemented in the checkpoint below.

## 2026-09-27 Mission Work tree checkpoint

[Tree evidence](../Reports/2026-09-27-native-mission-work-tree.json) advances
#25/#33/#63/#64 with canonical Plan/manual/repair hierarchy, separate dependency
edges, group-safe actions, exact task selection, recorded activity and narrow
inspection. Native379 tests passed /7 opt-in skipped; final layout/review32 and
installed5 checks passed after the focused evidence correction. Counts overlap.
The local preview is shipped with verified source/payload hashes and MIT notices.
Full issue-graph/role/attention/archive parity remains open.
[Check-result recovery](native-check-result-recovery.md) is verified in the checkpoint below.

## 2026-09-27 saved check-result recovery checkpoint

[Recovery evidence](../Reports/2026-09-27-check-result-recovery.json) advances
#65/#71 with exact request-bearing intent, durable actual check results and explicit
Failed reconstruction after worker interruption without check/model/Git replay.
Actual process-death cuts preserve uncertainty before result publication; valid
final evidence has precedence, damaged/legacy/partial artifacts remain unchanged.
Full native396/8, final artifact8/1 and installed7 pass; counts overlap. Narrow
PageUp/PageDown now visits every inspector/evidence row after resize. The verified
Linux preview is shipped locally. Automatic recovery, complete helper quiescence,
unknown effects, retirement and production acceptance remain open.
