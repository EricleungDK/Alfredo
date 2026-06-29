# Roadmap

**Last Updated**: 2026-06-18

## Completed

- Original Albert MVP workflow backlog: `.scratch/local-coding-agent-mvp/`.
- Development backlog for lifecycle cleanup, textual TUI, agent registry, TUI actions, fake runner, command runner, automated evidence, review loop, and PR readiness: `.scratch/local-coding-agent-mvp-development/`.
- Live Qwen3.6-27B prototype verification.
- Gemma4 local worker registry.
- Live Gemma4-12B local subagent verification through Albert's Ollama runner.
- Live Gemma4-26B local subagent verification through a failed-output repair loop.
- Repair relaunch workflow that passes prior Frontier review, evidence, and artifacts into the next Local Agent task packet.
- Qwen-controlled routing that keeps Gemma as the normal worker tier and exposes local Qwen2.5-Coder 14B / DeepSeek-R1 14B only as delegate-only escalation targets.
- Delegation approval gate support before Albert launches configured gated delegates.

## Current Follow-Up

1. Optionally smoke-test Albert launch paths for `qwen2.5-coder-14b` and `deepseek-r1-14b` on a small approved Issue Slice.
2. Design and implement a richer interactive full-screen TUI if command-style `tui` and `tui-action` stop being sufficient.
3. Decide packaging and distribution strategy.

## Verification Command

```bash
python3 -m unittest discover -s tests
```
