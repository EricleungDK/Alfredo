# Run Governed Shell Terminal Commands and Additional Path Grants

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Deliver Shell Terminal as a distinct left-lane mode governed by Albert's command policy and Additional Path Grants. The Mission Commander can execute allowed commands, resolve required approvals, and grant bounded external path access without creating an unrestricted execution route.

## Acceptance criteria

- [x] The left lane switches between Agent Console and Shell Terminal without mixing message history, terminal bytes, or input semantics.
- [x] Commands are classified as auto-allowed, frontier-approvable, or human-required and cannot execute before required approval.
- [x] Denied commands and commands outside granted paths fail with actionable explanations and no false success state.
- [x] An Additional Path Grant records explicit path, access level, and duration, and expired grants stop authorizing access.
- [x] Agents and skills cannot broaden, renew, or change their own Additional Path Grants.
- [x] Terminal bytes stay outside accepted mission state and the Activity Journal, while meaningful command decisions may be journaled.
- [x] Boundary tests cover all command classes, approvals, denial, grant creation, expiry, attempted self-expansion, and constrained desktop interaction.

## Blocked by

- `01-open-and-restore-command-deck-workspace-session.md`
- `02-synchronize-live-state-and-recover-from-disconnection.md`

## Comments

### 2026-07-01 — completion evidence

- Python service and CLI coverage proves all three command classes, approval-before-execution, denial, bounded read/write grants, expiry, self-expansion rejection, path rejection, and terminal-byte exclusion from canonical snapshots and the Activity Journal.
- TypeScript client and Rust bridge coverage proves typed projection, submit, decision, and grant transport through the real Python backend without adding terminal bytes to restored command records.
- React public-interaction coverage proves distinct left-lane panels and drafts, metadata-only restoration, current-session output, human/Frontier approval attribution, reason-required denial, immutable grant history, actionable failures, and constrained keyboard tab interaction.
- Fresh gates: 157 Python tests, 69 frontend tests, TypeScript typecheck, production build, Rust formatting, and 24 Rust tests all pass.
- Full acceptance mapping: `.agent/Reports/2026-06-29-governed-shell-terminal.md`.
