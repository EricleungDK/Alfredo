# Create and Confirm a Mission Draft

Status: complete
Type: AFK

## Parent

`.scratch/albert-mission-control-app/PRD.md`

## What to build

Let the Mission Commander assemble selected relevant Ad Hoc Delegations and new work into a Mission Draft, review the candidate scope, and explicitly confirm it before the Orchestrator creates accepted mission state.

## Acceptance criteria

- [x] The Mission Commander can select relevant Ad Hoc Delegations and describe new work for a Mission Draft.
- [x] Unselected or irrelevant delegations remain outside the draft and are not silently attached later.
- [x] The draft clearly presents proposed goal, included work, exclusions, dependencies, and unresolved decisions before confirmation.
- [x] Creating or editing a draft does not create an accepted Mission or merge mission-specific Shared Context.
- [x] Explicit confirmation creates accepted mission state through the Orchestrator; rejection or abandonment leaves existing missions unchanged.
- [x] Tests cover selection, exclusion, draft revision, confirmation, rejection, stale confirmation, and restart persistence.

## Blocked by

- `10-approve-and-supervise-ad-hoc-delegation.md`
