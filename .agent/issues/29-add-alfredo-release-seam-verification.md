# Add Alfredo Release Seam Verification

Status: Completed
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Add high-value release-seam verification for the complete Alfredo desktop-to-backend journey: install or launch command intent, startup preflight, workspace selection, prompt display, workstation card projection, consequential side-pane action, visible prompt/orchestrator turn, Orchestrator acknowledgement, card update, Activity Journal record, and restart restore.

## Acceptance criteria

- [x] An end-to-end journey covers Alfredo launch intent, startup preflight, workspace selection, prompt composer visibility, card projection, one consequential action, prompt/orchestrator reaction, acknowledged card update, Activity Journal record, and restart restore.
- [x] CLI tests cover default workstation launch, explicit workstation launch, headless run, review, session review, agent listing, `--agent` handling, invalid agent ids, and intentional Albert compatibility aliases.
- [x] Backend contract tests cover typed consequential actions with expected revisions, actor, target identity, and reason where required.
- [x] Provider/model tests use a deterministic provider-neutral fake for CI and keep local Ollama smoke verification separately marked.
- [x] Transcript tests prove durable history includes prompts, controller responses, consequential actions, and outcomes while excluding routine navigation and raw telemetry.

## Blocked by

- `21-add-headless-alfredo-cli-grammar.md`
- `26-cover-governed-workstation-action-family.md`
- `27-persist-alfredo-workstation-continuity.md`
- `28-validate-alfredo-accessibility-and-responsive-use.md`

## Comments

- 2026-07-06: Mission Commander approved this release-seam verification issue after all acceptance criteria were checked; registered locally as Completed.
