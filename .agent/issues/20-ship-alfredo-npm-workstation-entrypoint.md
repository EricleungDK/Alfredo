# Ship the Alfredo Npm Workstation Entrypoint

Status: ready-for-agent
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Make Alfredo installable and launchable through the public npm command path. Running `alfredo` opens the desktop workstation by default, `alfredo --agent <agent-id>` opens it with that controller or model selected, and `alfredo workstation --agent <agent-id>` provides the explicit rich UI command. Startup preflight should keep the product install location, selected coding workspace, backend runtime, desktop shell, Ollama availability, model availability, and writable runtime locations legible as separate checks.

## Acceptance criteria

- [ ] The published npm bin exposes `alfredo`, launches the Tauri/React workstation by default, and sets the desktop app title to Alfredo.
- [ ] `alfredo --agent <agent-id>` and `alfredo workstation --agent <agent-id>` select a valid controller or model before the first prompt turn.
- [ ] Startup preflight reports actionable, copyable failures for npm/runtime setup, desktop shell launch, backend process, workspace access, Ollama, required model availability, and writable runtime locations.
- [ ] The selected coding workspace is never inferred from the Alfredo installation location, and recent workspaces remain available after relaunch.
- [ ] A deprecated `albert` public alias remains available for one compatibility window if practical, without requiring internal package or `.albert` path renames.

## Blocked by

None - can start immediately.

