# Qwen-Controlled Delegation Report

Date: 2026-06-18

## Summary

Implemented the routing model requested by the user: Qwen3.6-27B remains the frontier model and now decides whether an approved Issue Slice should stay with a Gemma worker or escalate to a delegate-only model. The original Kimi/DeepSeek cloud plan was replaced after subscription-gated `403 Forbidden` responses; the default registry now uses local Qwen2.5-Coder 14B and DeepSeek-R1 14B delegates.

## Model Configuration

Current `.albert/agents.json`:

- Router/frontier: `qwen3.6-27b` -> `ollama:qwen3.6:27b`, `routing: router`.
- Local worker: `gemma4-12b` -> `ollama:gemma4:12b`, `routing: worker`.
- Local worker: `gemma4-26b` -> `ollama:gemma4:26b`, `routing: worker`.
- Delegate-only coding worker: `qwen2.5-coder-14b` -> `ollama:qwen2.5-coder:14b`, `routing: delegate`.
- Delegate-only architect/reviewer: `deepseek-r1-14b` -> `ollama:deepseek-r1:14b`, `routing: delegate`.

## Implemented Behavior

- Agent registry supports `routing`, `delegate_only`, and `requires_approval`.
- Manual assignment filters out Qwen/router entries and delegate-only entries.
- Attempting to manually assign a delegate-only agent is rejected.
- `route ISSUE_ID` asks Qwen for a structured JSON delegation decision with complexity, recommended agent, reason, and approval requirement.
- Configured gated or cloud delegates require `approve-delegation ISSUE_ID` before launch.
- Approved gated delegation records the exact runner command in the command policy as auto-allowed.
- Launch task packets include the recorded delegation decision.
- TUI next actions include `route` for approved unrouted work and `approve-delegation` for unapproved gated decisions.

## Files Changed

- `.albert/agents.json`
- `albert_mvp/agents.py`
- `albert_mvp/core.py`
- `albert_mvp/cli.py`
- `albert_mvp/tui.py`
- `tests/test_albert_mvp.py`
- `README.md`
- `.agent/Tasks/context.md`
- `.agent/Tasks/README.md`
- `.agent/System/project_architecture.md`

## Verification

```bash
python3 -m unittest discover -s tests
```

Result: 66 tests passing.

```bash
python3 -m albert_mvp agents --target-repo . --tracker-dir .scratch/local-coding-agent-mvp-development --runtime-root /tmp/albert-runtime --mission-id local-coding-agent-mvp-development --agent-config .albert/agents.json
```

Result: registry lists Qwen as router, Gemma models as normal workers, and Qwen2.5-Coder 14B / DeepSeek-R1 14B as delegate-only targets.

```bash
python3 -m albert_mvp tui --target-repo . --tracker-dir .scratch/local-coding-agent-mvp-development --runtime-root /tmp/albert-runtime --mission-id local-coding-agent-mvp-development --agent-config .albert/agents.json
```

Result: TUI renders successfully and normal assignment options show only Gemma workers.

Cloud access check after `ollama signin`:

```bash
ollama run kimi-k2.6:cloud "Return exactly: KIMI_CLOUD_OK"
ollama run deepseek-v4-pro:cloud "Return exactly: DEEPSEEK_CLOUD_OK"
```

Result: both commands reached Ollama Cloud but returned `403 Forbidden` because the account needs a subscription/upgrade for those models.

Local replacement pulls:

```bash
ollama pull deepseek-r1:14b
ollama pull qwen2.5-coder:14b
```

Result: both local replacements pulled successfully. `qwen3:14b` also pulled successfully while checking available Qwen 14B tags; `qwen3.6-coder:14b` and `qwen3-coder:14b` did not resolve as Ollama tags.

## Remaining Follow-Up

- Optionally smoke-test Albert launch paths for `qwen2.5-coder-14b` and `deepseek-r1-14b` on a small approved Issue Slice.
- Keep Gemma as the default worker tier unless Qwen routes a task upward.
- Consider whether the interactive TUI should expose a dedicated route/approve-delegation workflow once the command semantics settle.
