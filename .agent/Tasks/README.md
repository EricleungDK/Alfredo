# Roadmap

**Last Updated**: 2026-08-30

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
- Alfredo prompt-first workstation correction: responsive readable UI, unified console chronology, deterministic and typed controller task routing, governed automatic delegation, commands/skills/tasks/discussion, persistent Mission Work, queued/cancellable Local Agent execution, iterative repair, atomic idempotent actions, minimal resource-bounded process sandboxes, real Evidence Package artifacts, and bounded inline artifact viewing. See [the implementation report](../Reports/2026-07-11-alfredo-one-shot-workstation.md).
- Retirement lifecycle through Issue #68: every Retirement Unit has reserved preservation, exact quiescence/identity proof, verified snapshots, outcome-driven retirement, bounded Snapshot Payload retention and reclamation, deterministic storage inspection, and governed blocked retry/export/discard actions. See [the Issue #68 report](../Reports/2026-08-09-issue-68-retirement-storage.md).
- Qualified Local Agent host effects through Issue #73: Python retains authorization, canonical Mission/session state, exact replay, and reconciliation while one integrity-bound Rust provider performs the prepared external effect; live child binding, cancellation, uncertainty, compatibility, and the explicit packaged Python fallback are covered by the [Issue #73 report](../Reports/2026-08-30-issue-73-local-agent-rust-cutover.md).
- Qualified Shell host effects through Issue #74: Python retains command policy, approvals, Additional Path Grants, canonical Shell/audit state, exact replay, and reconciliation while one integrity-bound Rust provider performs the prepared external effect; independent rollback, transport compatibility, live child binding/cancellation, and uncertainty are covered by the [Issue #74 report](../Reports/2026-08-30-issue-74-shell-rust-cutover.md).
- Integrated modernization verification through Issue #75: the production-equivalent Starting Location → Coding Workspace → Mission → Wayfinder → governed work → evidence/review → retirement → restart journey is executable against the real backend; Local Inference Profile/Lease identity remains visible without Mission authority; packaged Shell and Local Agent pass both protocol generations with explicit Python fallback; and the protected release workflow includes performance contracts/fixtures plus the installed selection-required marker boundary. See the [Issue #75 report](../Reports/2026-08-30-issue-75-modernized-workstation-verification.md).

## Current Follow-Up

1. Decide whether to make the currently private source repository public for npm provenance or explicitly accept a non-provenanced private bootstrap, then authorize commit/push of the exact reviewed source revision.
2. Configure the protected GitHub `npm-production` environment and first-release `NPM_TOKEN`, publish `alfredo-agent-linux-x64-gnu@0.1.0` before `alfredo-agent@0.1.0`, pass the workflow's fresh public-registry install/PATH/headless-GUI smoke, and confirm one human-visible `alfredo` launch before completing ticket 20.
3. Complete the separately tracked human visual hierarchy, screen-reader, zoom/reflow, low-vision, and reduced-motion confirmations for Issues 14/28.
4. Optionally smoke-test delegate launch paths for `qwen2.5-coder-14b` and `deepseek-r1-14b` on a small approved Issue Slice.

## Verification Command

```bash
python3 -m unittest discover -s tests
(cd mission-control && npm test -- --run && npm run performance:fixtures && npm run test:performance && npm run typecheck && npm run build && npm run test:layout)
(cd mission-control && npm run release:verify && npm run release:check)
(cd mission-control/src-tauri && cargo fmt -- --check && cargo test)
```
