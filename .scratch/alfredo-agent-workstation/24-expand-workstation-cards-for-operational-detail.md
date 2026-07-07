# Expand Workstation Cards for Operational Detail

Status: complete
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Let the Mission Commander expand a workstation card to inspect the operational detail needed for supervision and review. Expanded cards expose tool calls, command summaries, files and diffs, Evidence Package links, relevant terminal excerpts, review state, and available governed actions while keeping routine navigation out of the durable prompt transcript.

## Acceptance criteria

- [x] Expanded cards show tool activity, files touched, diffs, Evidence Package links, relevant terminal excerpts, review state, and available governed actions for the selected agent or subagent.
- [x] Opening a diff, selecting a session, filtering, sorting, expanding, collapsing, and pinning cards remain local side-pane navigation and do not append prompt transcript turns.
- [x] Evidence Package links and terminal excerpts are connected to their originating session and remain available for Frontier Reviewer or human review decisions.
- [x] Raw token streams and raw terminal bytes remain transient unless summarized as meaningful activity, evidence, or review artifacts.
- [x] Interaction tests distinguish routine side-pane navigation from consequential workstation actions.

## Blocked by

- `23-project-live-agent-workstation-cards.md`

## Evidence

- `npm run typecheck`
- `npm test -- --run`
- `python3 -m unittest discover tests`
- `cargo test`
