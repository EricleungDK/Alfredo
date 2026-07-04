# Add Headless Alfredo Cli Grammar

Status: ready-for-agent
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Add the first scriptable terminal-only Alfredo command grammar while keeping the desktop workstation as the default experience. `alfredo run --agent <agent-id> "<prompt>"`, `alfredo review --agent <agent-id>`, `alfredo review <session-id> --agent <agent-id>`, and `alfredo agents` should run through the same Orchestrator and model registry boundaries as the workstation, so SSH and automation users get a reduced but governed fallback.

## Acceptance criteria

- [ ] `alfredo run --agent <agent-id> "<prompt>"` executes terminal-only model work and returns clear lifecycle output suitable for automation.
- [ ] `alfredo review --agent <agent-id>` and `alfredo review <session-id> --agent <agent-id>` run review-oriented work without launching the desktop UI.
- [ ] `alfredo agents` lists configured agents and models using the provider-neutral model registry vocabulary.
- [ ] Per-command `--agent` is the canonical scripting grammar for headless commands, and invalid agent ids fail with actionable guidance.
- [ ] Terminal-only work still respects Orchestrator governance, command policy, path grants, model assignment, and Evidence Package boundaries.

## Blocked by

- `20-ship-alfredo-npm-workstation-entrypoint.md`

