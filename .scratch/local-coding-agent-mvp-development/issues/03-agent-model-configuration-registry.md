Status: complete
Type: AFK

## Parent

.scratch/local-coding-agent-mvp-development/PRD.md

## What to build

Add a local agent model configuration registry for Local Agents and Frontier roles. The tool should load configured agents from a local config source, list available agents, validate required fields, reject unknown assignments, and keep model assignment explicit before launch.

## Acceptance criteria

- [x] The user can list configured Local Agent and Frontier role entries from the command surface.
- [x] Configured agents include a stable id, role, provider or runner type, and command or model details needed by that runner type.
- [x] Invalid config produces actionable validation errors that identify the broken agent entry.
- [x] Assigning an Issue Slice to an unknown agent is rejected.
- [x] Existing issue metadata can still provide suggested and assigned agents when no config is present.
- [x] Tests cover config loading, missing config fallback, invalid config, listing agents, and assignment validation.

## Blocked by

- .scratch/local-coding-agent-mvp-development/issues/01-lifecycle-state-cleanup-and-reopen-controls.md
