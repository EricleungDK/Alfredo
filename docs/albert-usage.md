# Albert MVP Usage Guide

**Last updated:** 2026-06-18

These commands assume you are in the repository root.

## Confirm Local Models

```bash
ollama list
```

The expected active models are:

- `qwen3.6:27b`
- `gemma4:12b`
- `gemma4:26b`
- `qwen2.5-coder:14b`
- `deepseek-r1:14b`

## List Albert Agents

```bash
python3 -m albert_mvp agents \
  --target-repo . \
  --tracker-dir .scratch/local-coding-agent-mvp-development \
  --runtime-root /tmp/albert-runtime \
  --mission-id local-coding-agent-mvp-development \
  --agent-config .albert/agents.json
```

## Run Tests

```bash
python3 -m unittest discover -s tests
```

## Open the Textual Mission-Control TUI

```bash
python3 -m albert_mvp tui \
  --target-repo . \
  --tracker-dir .scratch/local-coding-agent-mvp-development \
  --runtime-root /tmp/albert-runtime \
  --mission-id local-coding-agent-mvp-development \
  --agent-config .albert/agents.json
```

## Inspect a Slice

```bash
python3 -m albert_mvp show ISS-01 \
  --target-repo . \
  --tracker-dir .scratch/local-coding-agent-mvp-development \
  --runtime-root /tmp/albert-runtime \
  --mission-id local-coding-agent-mvp-development \
  --agent-config .albert/agents.json
```

Many demo tracker slices are already complete. To exercise launch, review, repair, or delegation end to end, reopen an existing slice intentionally or create a new tracker mission.

## Common Command Shape

Every command uses the same runtime context:

```bash
python3 -m albert_mvp <command> \
  --target-repo . \
  --tracker-dir .scratch/local-coding-agent-mvp-development \
  --runtime-root /tmp/albert-runtime \
  --mission-id local-coding-agent-mvp-development \
  --agent-config .albert/agents.json
```

Common commands:

- `board` shows PRD title, issue order, blockers, and readiness.
- `agents` lists configured model agents.
- `tui` renders the textual mission-control surface.
- `show ISSUE_ID` shows one slice, blockers, assignment, and next actions.
- `approve ISSUE_ID` approves and locks a slice.
- `assign ISSUE_ID --agent AGENT` changes the assigned normal worker.
- `route ISSUE_ID` asks Qwen to choose a worker or delegate.
- `approve-delegation ISSUE_ID` approves a gated delegation when required.
- `launch ISSUE_ID` launches an approved unblocked slice.
- `evidence SESSION_ID ...` validates and records an Evidence Package.
- `review SESSION_ID --outcome OUTCOME --reason REASON` records Frontier review.
- `tui-action repair ISSUE_ID --session SESSION_ID` launches repair from a repairable review.
- `records` generates mission Markdown records.
- `pr ISSUE_ID` prepares PR-ready instructions.

## Typical Flow for New Work

1. Create or select a tracker mission under `.scratch/`.
2. Render `board` or `tui` to inspect order and blockers.
3. Use `show ISSUE_ID` to inspect acceptance criteria and next actions.
4. Approve a ready slice with `approve ISSUE_ID`.
5. Optionally run `route ISSUE_ID` so Qwen can choose Gemma or a delegate.
6. Launch with `launch ISSUE_ID`.
7. Record or inspect evidence.
8. Review with `review SESSION_ID`.
9. Repair if needed.
10. Prepare PR instructions with `pr ISSUE_ID`.
