# Albert MVP Mapping and Status

**Last updated:** 2026-06-21

This document maps the original product idea to the implemented MVP and records the current status without duplicating the full PRDs or issue lists.

## Product Idea to MVP

| Product goal | MVP implementation | Source |
| --- | --- | --- |
| Convert sharpened requests into durable plans | Local markdown PRD and Issue Slice tracker | `.scratch/local-coding-agent-mvp/PRD.md` |
| Break work into independently grabbable slices | Issue Graph parser with blockers and dependency ordering | `albert_mvp/core.py` |
| Let the user review and approve work contracts | `approve`, `reopen`, locking, and TUI detail views | `albert_mvp/core.py`, `albert_mvp/tui.py` |
| Assign local model workers | Agent registry and assignment validation | `.albert/agents.json`, `albert_mvp/agents.py` |
| Let a frontier/router model choose escalation | `route` and Qwen-controlled delegation decisions | `albert_mvp/core.py` |
| Launch bounded implementation sessions | Fake, command, and Ollama runners in isolated worktrees | `albert_mvp/core.py` |
| Require evidence before review | Evidence Package validation | `albert_mvp/core.py` |
| Review and repair based on evidence | Frontier review states and `tui-action repair` relaunch | `albert_mvp/core.py`, `albert_mvp/tui.py` |
| Prepare PR-ready work without merging | `pr` and `prepare-pr` command paths with GitHub fallback | `albert_mvp/core.py`, `albert_mvp/cli.py` |
| Preserve mission memory | Runtime JSON plus generated mission Markdown records | `albert_mvp/core.py` |

## Completed

- Original product MVP tracker is complete: `.scratch/local-coding-agent-mvp/`.
- Development tracker is complete: `.scratch/local-coding-agent-mvp-development/`.
- Textual mission-control TUI is implemented.
- Lifecycle cleanup and explicit reopen controls are implemented.
- Agent registry loading and validation are implemented.
- Fake, command-backed, and Ollama-backed runners are implemented.
- Automated evidence collection and Evidence Package validation are implemented.
- Frontier review outcomes and repair relaunch are implemented.
- PR readiness and GitHub command fallback are implemented.
- Qwen-controlled delegation is implemented and tested.
- Delegate-only model policy is implemented.
- Local Qwen2.5-Coder 14B and DeepSeek-R1 14B replaced subscription-gated cloud delegates.
- Command Deck Issue 01 is complete: canonical snapshot/restoration, Tauri transport, React projection, actionable failure states, and desktop-to-backend restart coverage are implemented and integration-tested.
- Command Deck Issue 02 is complete: ordered live synchronization, correlated expected-revision actions, stale-state protection, explicit action outcomes, and fresh-snapshot reconnect recovery are integration-tested.
- Command Deck Issue 03 is complete: continuous scoped Agent Console history, deliberate Working directory/Mission/Issue Slice targeting, scope-mismatch protection, five sourced outcomes, authorization separation, Tauri transport, and navigation/mission-switch continuity are integration-tested.
- Command Deck Issue 04 is complete: bounded reconstructable Working Context, five source categories, eligible pin/exclude persistence, governed Shared Context rejection, Context Inspector UI, and restart/concurrency recovery are integration-tested.

## Verification Snapshot

Most recent verification on 2026-06-21:

- `python3 -m unittest discover -s tests -v` passed 94 tests.
- `npm test -- --run`, `npm run typecheck`, and `npm run build` passed (30 frontend/transport tests).
- Rust formatting passed and `cargo test --manifest-path mission-control/src-tauri/Cargo.toml` passed 10 tests with desktop features.
- A fresh-runtime CLI smoke restored Working Context revision 3 with a pinned message, excluded eligible Issue Slice, governed required sources, workspace revision 2, and full history intact; eight concurrent reader processes also completed cleanly.
- `python3 -m albert_mvp agents ... --agent-config .albert/agents.json` listed Qwen router, Gemma workers, and local delegate-only agents.
- `python3 -m albert_mvp tui ... --agent-config .albert/agents.json` rendered successfully and hid delegate-only agents from normal manual assignment.
- `ollama list` showed `qwen3.6:27b`, `gemma4:12b`, `gemma4:26b`, `qwen2.5-coder:14b`, and `deepseek-r1:14b` installed.

Live checks from earlier sessions:

- Qwen3.6-27B generated a runnable `prototype_app.py` that printed `Albert prototype ready`.
- Gemma4-12B generated a runnable prototype and reached PR-ready after review.
- Gemma4-26B first produced incorrect output, then succeeded through the `Needs repair` to `tui-action repair` loop.

## Pending Work

- Complete Command Deck Issues 05-14, beginning with multi-mission creation, selection, and background execution visibility.
- Optionally smoke-test Albert launch paths for `qwen2.5-coder-14b` and `deepseek-r1-14b` on a small approved Issue Slice.
- Build a richer interactive full-screen terminal TUI once command semantics are stable.
- Decide packaging and distribution strategy.
- Decide whether provider/model alias mapping is needed for unavailable or renamed Ollama tags.
- Reconcile or annotate older historical reports that reference the superseded cloud delegate registry.

## Authoritative Sources

- Architecture and design: `docs/albert-architecture.md`
- Usage guide: `docs/albert-usage.md`
- Product PRD: `.scratch/local-coding-agent-mvp/PRD.md`
- Product Issue Slices: `.scratch/local-coding-agent-mvp/issues/`
- Development roadmap: `.scratch/local-coding-agent-mvp-development/PRD.md`
- Development issues: `.scratch/local-coding-agent-mvp-development/issues/`
- Qwen delegation report: `.agent/Reports/2026-06-18-qwen-controlled-delegation.md`
