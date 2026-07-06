Status: complete
Type: AFK

## Parent

.scratch/alfredo-console-first-workstation-redesign/PRD.md

## What to build

Represent governed Shell Terminal execution inside the Agent Console as inline command turns or compact command cards. Command work should feel like part of the continuous agent interaction while still allowing the Mission Commander to inspect full output, policy state, working directory, and audit detail when summaries are not enough.

## Acceptance criteria

- [x] Submitted or observed Shell Terminal commands appear in the Agent Console as inline command turns or compact command cards.
- [x] Command cards show the command, purpose when available, relevant working directory, policy or approval state, execution status, and summarized output.
- [x] Full command output remains reachable through expansion, selected-turn detail, Activity Journal, or equivalent audit/debug drill-down.
- [x] Command history, requested path entry, access-level selectors, and manual grant forms are not standing default GUI panels.
- [x] Durable Agent Console history excludes raw terminal bytes while preserving meaningful command summaries and outcomes.
- [x] Tests cover inline command rendering, summary/default behavior, full-output inspection, and hidden default command-history surfaces.

## Blocked by

- .agent/issues/29-add-alfredo-release-seam-verification.md
- .scratch/alfredo-console-first-workstation-redesign/issues/01-console-first-workstation-layout.md

## Comments

- Implemented inline Agent Console command cards for observed and current-session Shell Terminal commands. Raw stdout/stderr stays out of default console history and is available through explicit full-output expansion or the Command Audit drill-down.
