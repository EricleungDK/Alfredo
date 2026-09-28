# Native workspace and mission selection continuity

Date: 2026-09-26
Status: implemented and verified for the bounded native selection slice; broader production work remains open.
Authority: the active Rust terminal rewrite goal and [command chronology](native-command-chronology.md).

## Required behavior

Startup and in-process workspace selection must save an exact request before repository initialization or mission creation. The original request and each observed result remain inspectable across a failed handoff, cancellation and restart. Selecting work never grants task execution authority.

The original `choose_in_terminal` created a repository before mission selection and called `select_mission(start_new)` before returning. Wrapping only `switch_to` would have left those earlier effects unbound. The picker now returns a read-only choice; startup and switching use the independent journal, with source-session admission for switching.

## Implemented boundary

The picker first collects an immutable choice without creation effects: existing canonical root or resolved parent plus new repository name; Resume or Start New mission; conversation namespace. Existing-path validation and discovery remain read-only. New repositories must collect a mission name before initialization.

A bounded state-directory selection journal captures the request and reserves outcome capacity under an explicit-unlock transaction guard. Synced exact admission gates effects. For an in-process switch, the originating session also saves its command with the same request; both publications must acknowledge before preparation begins. Startup uses a launcher origin in the selection journal.

Capture repository preparation, mission preparation, target loading and actual handoff as separate observed results. They are selection records, never task receipts. Keep the source workstation and owner until target loading and the latest source checkpoint both succeed. Target preparation retains its acquired conversation owner until commit or failure. Preserve the existing fresh event-channel generation when replacing a workstation.

Production boundaries are `selection::prepare_workspace`, `selection_store::Store::{admit,begin,record}`, and `Workstation::{launch,select}`. A consumed journal admission cannot be reconstructed from a saved record. The exact request binds correlation, startup or conversation origin, workspace choice, mission choice and conversation namespace. Source origin includes workspace, mission, conversation and session identity. A stale callback cannot replace a different current source.

## Failure and recovery rules

- Cancelled picker or withdrawn unsent request creates nothing; an older completed save cannot revive it.
- Once creation starts, cancellation cannot claim no changes. A created repository may survive failed mission creation or target loading; keep and report that result while retaining usable source work.
- A crash between creation and outcome publication restores uncertainty. Existing paths or mission names alone cannot prove this request created them. Without exact creation provenance, uncertain retries must refuse silent recreation or conversion to Resume. Explicit new Open/Resume remains available.
- Record handoff prepared before the in-memory swap; do not claim workspace selected early. Publish actual selection afterward and surface any history-publication failure without fabricating rollback.
- A same-target request reports already current before acquiring its already-held owner. Check target history capacity before handoff; arrival events stay outside model messages.

## Verification

Cover failed/delayed intent publication before repository or mission creation; picker cancellation and stale tokens; creation followed by mission/load failure; owner conflict, malformed target history and failed source save; crash cuts and exact identity conflict; same-target selection; original session and destination arrival; retained drafts and reading; stale-channel isolation. Extend installed PTY startup creation, existing-mission resume, cancellation, failed handoff and switching back.

Implemented in `selection_command.rs`, `selection_store.rs`, `selection.rs`, `workstation.rs`, `main.rs`, command intent/state, conversation schema17, rendering and selection tests. `TaskStore::check_mission_selection` provides read-only picker preflight. Repository and mission creation markers remain unchanged; uncertain creation is not replayed.

This selection slice does not close full mission formation, model qualification, execution recovery, retirement/storage, platform qualification or launch acceptance.


## Verified checkpoint

Focused 104 and full 305 pass / 7 ignored; separation guard RED/GREEN, strict Clippy and installed archive pass. [Implementation and validation evidence](../Reports/2026-09-26-selection-continuity.json). The installed binary verifies cancellation before creation, actual startup creation, existing-mission resume, failed target loading and switch-back continuity. Selection is synchronous after confirmation; uncertain creation and bounded journal retirement remain explicit limits.
