Status: ready-for-agent
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Redesign the default Alfredo workstation view so the Agent Console is the dominant working surface. The Mission Commander should open into a console-first workstation with a pinned prompt composer, compact operational status near the composer, and a persistent secondary Mission Work pane instead of a tabbed Workstations/Shell Terminal mode switcher.

## Acceptance criteria

- [ ] Alfredo opens to an Agent Console-first workstation where prompting is the primary visual and interaction surface.
- [ ] The prompt composer remains pinned at the bottom while earlier console turns can be reviewed.
- [ ] A compact status line near the composer shows connection, selected controller/model, Conversation Scope, workspace, and active execution state.
- [ ] The side pane remains secondary in width and hierarchy, and no Shell Terminal or workstation tab switcher is presented as the default primary structure.
- [ ] Tests verify the rendered default layout, composer reachability, status-line content, and console-dominant hierarchy.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
