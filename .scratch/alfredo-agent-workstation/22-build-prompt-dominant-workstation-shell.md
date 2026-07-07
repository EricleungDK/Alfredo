# Build the Prompt Dominant Workstation Shell

Status: ready-for-agent
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Replace the superseded three-zone Mission Control framing with Alfredo's prompt-dominant two-zone workstation. The main pane is the durable prompt transcript with a pinned composer and compact status line; the secondary persistent pane is Agent Workstations. The Mission Commander should be able to read prior turns, keep steering from the bottom composer, and always see the selected controller/model and active execution state near the prompt.

## Acceptance criteria

- [ ] The desktop workstation opens to a two-zone layout with the prompt pane as the dominant working surface and Agent Workstations as a persistent side pane.
- [ ] The prompt transcript renders user prompts, assistant/controller responses, consequential workstation actions, and meaningful outcomes without mixing in raw telemetry.
- [ ] The active prompt composer remains pinned at the bottom while prior turns scroll independently.
- [ ] A compact status line near the composer shows connection, selected controller/model, Conversation Scope, workspace, and active execution state.
- [ ] Constrained-width behavior keeps the prompt, composer, critical status, and side-pane decisions reachable without overlapping content.

## Blocked by

- `20-ship-alfredo-npm-workstation-entrypoint.md`

