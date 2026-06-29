# Run Governed Shell Terminal Commands and Additional Path Grants

Status: in-progress
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Deliver Shell Terminal as a distinct left-lane mode governed by Albert's command policy and Additional Path Grants. The Mission Commander can execute allowed commands, resolve required approvals, and grant bounded external path access without creating an unrestricted execution route.

## Acceptance criteria

- [ ] The left lane switches between Agent Console and Shell Terminal without mixing message history, terminal bytes, or input semantics.
- [ ] Commands are classified as auto-allowed, frontier-approvable, or human-required and cannot execute before required approval.
- [ ] Denied commands and commands outside granted paths fail with actionable explanations and no false success state.
- [ ] An Additional Path Grant records explicit path, access level, and duration, and expired grants stop authorizing access.
- [ ] Agents and skills cannot broaden, renew, or change their own Additional Path Grants.
- [ ] Terminal bytes stay outside accepted mission state and the Activity Journal, while meaningful command decisions may be journaled.
- [ ] Boundary tests cover all command classes, approvals, denial, grant creation, expiry, attempted self-expansion, and constrained desktop interaction.

## Blocked by

- `01-open-and-restore-command-deck-workspace-session.md`
- `02-synchronize-live-state-and-recover-from-disconnection.md`
