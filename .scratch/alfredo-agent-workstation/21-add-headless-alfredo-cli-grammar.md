# Add Headless Alfredo Cli Grammar

Status: complete
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Add the first scriptable terminal-only Alfredo command grammar while keeping the desktop workstation as the default experience. `alfredo run --agent <agent-id> "<prompt>"`, `alfredo review --agent <agent-id>`, `alfredo review <session-id> --agent <agent-id>`, and `alfredo agents` should run through the same Orchestrator and model registry boundaries as the workstation, so SSH and automation users get a reduced but governed fallback.

## Acceptance criteria

- [x] `alfredo run --agent <agent-id> "<prompt>"` executes terminal-only model work and returns clear lifecycle output suitable for automation.
- [x] `alfredo review --agent <agent-id>` and `alfredo review <session-id> --agent <agent-id>` run review-oriented work without launching the desktop UI.
- [x] `alfredo agents` lists configured agents and models using the provider-neutral model registry vocabulary.
- [x] Per-command `--agent` is the canonical scripting grammar for headless commands, and invalid agent ids fail with actionable guidance.
- [x] Terminal-only work still respects Orchestrator governance, command policy, path grants, model assignment, and Evidence Package boundaries.

## Blocked by

- `20-ship-alfredo-npm-workstation-entrypoint.md`

## Comments

### 2026-07-12 — completed

- Public grammar and invalid-agent/preflight cases are covered by the 32 focused launcher/package tests.
- The full 416-test Python gate (one optional live-Ollama skip) covers governed headless execution, review, command policy, path authority, model assignment, and Evidence Package behavior.
