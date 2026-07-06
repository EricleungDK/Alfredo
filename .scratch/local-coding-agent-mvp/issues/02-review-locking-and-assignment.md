Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp/PRD.md

## What to build

Build the Issue Slice review workflow: the user can inspect slices, approve or hold them, lock approved work contracts, and change only allowed pre-launch fields such as assignment and launch notes without unlocking the contract.

## Acceptance criteria

- [x] The user can approve, hold, or request revision for an Issue Slice from the local command surface.
- [x] Approval locks slice goal, acceptance criteria, blockers, HITL/AFK classification, risk level, and evidence requirements.
- [x] Assigned model agent, launch order, and optional Local Agent notes remain editable before launch.
- [x] Attempts to change locked fields after approval are rejected unless the user explicitly unlocks the slice.
- [x] The review state is persisted and visible in the mission board summary.

## Blocked by

- .scratch/local-coding-agent-mvp/issues/01-mission-state-and-record-loading.md
