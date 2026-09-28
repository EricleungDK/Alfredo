# Native Mission Work tree

Date: 2026-09-27  
Status: bounded implementation verified; local human preview shipped. Broader native supervision parity remains open.

## Authority and current gap

The current user requests a polished multi-agent Rust terminal and regression fixes.
Fresh authenticated GitHub reads on September27 confirm the supervision requirements
in [#63](https://github.com/EricleungDK/Alfredo/issues/63),
[parent #56](https://github.com/EricleungDK/Alfredo/issues/56), and the earlier
[#25](https://github.com/EricleungDK/Alfredo/issues/25) and
[#33](https://github.com/EricleungDK/Alfredo/issues/33).
Their closed desktop/legacy implementation state does not establish native parity.
The current explicit Rust rewrite request governs the terminal migration; historical
desktop-only implementation restrictions remain context for compatibility decisions.
The preceding [inference diagnostic checkpoint](../Reports/2026-09-27-native-inference-qualification.json)
records the verified candidate and the remaining model-quality limits.

Before this slice, native F2 listed tasks as flat status/title rows. Canonical snapshots already
retain Plan membership, dependencies, repair ancestry, review decisions and accepted
repair replacements. The task inspector and readiness projection expose much of the
necessary detail, but users must inspect individual tasks to reconstruct the work.

## Bounded implementation

Build a read-only Mission Work projection from the existing snapshot in
`mission_work.rs`. Group existing tasks by recorded Plan membership, show manual tasks
as such, and nest repair descendants beneath their original work. Use stable canonical
IDs; never present native task IDs as imported GitHub Issue Slice identities. Display
dependencies as edges/blockers rather than ownership, so diamonds never duplicate
work or imply that one dependency owns its consumer.

Reuse `ScopeStatus::readiness`, `Snapshot::dependency_source`, `resolved_by` and
architecture/hold checks for explanations. A rendered recommendation grants no
execution authority. Counts must name their units and distinguish tasks from active
workers; uncertain recorded runs remain uncertain.

Adapt `task_control.rs` navigation and the task-list/inspector regions in `ui.rs`.
Keep selection anchored to exact task identity across filtering, expansion, refresh
and restart. Group navigation must not silently retarget task shorthand. Preserve
the composer and current evidence/activity controls, including the focused narrow
layout and long-output paging behavior. Detailed output stays tied to the exact
opened task/run; collapsed groups do not fabricate agent state or activity receipts.

## Acceptance and verification

1. A multi-task Plan, manual task and repair family appear once each with canonical
   IDs, useful titles, named counts and explicit lifecycle text.
2. A dependency diamond shows all dependency edges without duplicate task rows;
   pending approval, human holds, Architect revision and accepted replacement inputs
   use the same readiness truth as current governed actions.
3. Selecting started work exposes its actual model/run identity, current observation,
   recent canonical activity, evidence and permitted next action. Missing or stale
   observation is explicit and cannot imply a live or dead worker.
4. Keyboard navigation, filtering and group expansion retain the intended action
   target; view changes cause no task transaction or conversation action history.
5. Controlled concurrent-worker and rejected-parent/accepted-repair journeys remain
   usable at normal size and 32x10, with reachable composer and inspector. Projection,
   navigation/render tests and installed PTY acceptance cover these behaviors.

Start with the projection and selection contracts, then integrate rendering and one
installed journey. Root coordinates Cargo; separate subagents can own projection,
navigation and rendering/acceptance with explicit file boundaries. No dependency or
canonical schema change is presumed; any persistent view-state change needs an
explicit compatibility decision and appropriate existing-snapshot regressions.

This slice does not complete GitHub Issue Graph import, Mission Draft formation,
general capability routing, archive/retirement, exact process recovery or all #63
desktop/accessibility criteria. Those remain in the regression inventory. The recorded inference diagnostic checkpoint remains separate model-quality evidence.

## Verified checkpoint

[Implementation report](../Reports/2026-09-27-native-mission-work-tree.json) records
379 passing native tests with7 opt-in skips, followed by32 focused layout/review
checks after the evidence-width correction, final formatting/strict Clippy and
all5 installed checks. These counts overlap. Independent review reproduced and
fixed stale reviewer text; installed acceptance exposed and fixed the focused
evidence width regression. All52 source and6 payload hashes match the final archive.

The local Linux/WSL preview is `alfredo-tui/dist/preview-2026-09-27-work-tree`;
launch its `alfredo-tui` with `--model qwen3:14b --state-dir "$HOME/.local/state/alfredo-preview"`.
F2 opens the tree; Up/Down selects rows, Alt+Left/Right changes branch disclosure,
F3 opens selected-task evidence and Ctrl+Q quits. MIT and third-party notices remain
bundled. The prior preview remains intact.

The [proposed next slice](native-check-result-recovery.md) addresses conservative
recovery after a durably saved terminal check result. No recovery implementation
or GitHub publication is included in this checkpoint.
