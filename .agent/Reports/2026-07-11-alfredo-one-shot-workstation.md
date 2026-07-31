# Alfredo One-Shot Workstation Correction

> **Superseded release conclusion (2026-07-12):** The implementation history below is retained, but its npm completion evidence did not perform a real npm install/PATH/native launch and the Queue acceptance claim missed mixed manual/history content. Its documented launcher prewarm was subsequently removed. Read the [install and Queue acceptance correction](2026-07-12-alfredo-install-queue-acceptance-correction.md) for current status. Public npm publication remains open.

**Date:** 2026-07-11
**Last verified:** 2026-07-13
**Status:** Historical implementation report; release conclusion superseded. The four-viewport Chromium gate now passes, while authenticated npm publication, public-registry smoke, real-display launch, and separately tracked human accessibility review remain open in the current correction report.
**Implemented by:** Codex root with parallel frontend, persistence, sandbox, and audit agents

## Overview

Alfredo is now a prompt-first local coding-agent workstation rather than a scope-first operations dashboard. The primary lane supports fast controller discussion, `/help`, `/skills`, `/use`, `/run`, `/task`, and imperative coding requests. The persistent Mission Work lane projects canonical Local Agent/subagent sessions, attention, workable Issue Slices, evidence, and typed governance actions.

This correction also closes the backend conditions that made a polished UI unreliable: deferred execution, concurrent JSON overwrites, colliding Mission-local identities, stuck queued/running sessions, one-pass model failures, unsafe process views, misleading evidence links, and controller/worker role confusion.

## Requirements Delivered

- Readable two-lane layout with Ubuntu Sans/Ubuntu Mono fallbacks, WCAG contrast/focus rules, reduced motion, independent scroll owners, bounded overlays, and 1040/680/520 px reflow.
- A self-contained npm artifact with `alfredo` and deprecated `albert` bins, the real Python backend and default agent registry staged only during packing, explicit `ALBERT_BACKEND_ROOT` override support, and packed-consumer launch coverage that does not depend on the source repository layout.
- Immediate optimistic prompt echo, qwen3:14b controller default, the then-present controller prewarm, `--think=false`, persistent backend transport, and a measured warm-controller median of about 1.07 seconds. The prewarm was removed after this historical measurement. Rejected persistence cannot overwrite a newer draft, Enter cannot start a second mutation while the first save is active, and prompt input is capped at the shared 16,000-character durable boundary.
- One arrival-ordered Agent Console chronology for prompts, controller replies, commands, approvals, workstation actions, and outcomes; restart reconstruction causally anchors durable delegation milestones beside their originating controller turn, while smart auto-follow preserves the reader's position in older history.
- Discoverable installed skills and slash commands, keyboard-managed capability palette, governed `/run`, skill-aware `/use`, deterministic `/task` plus narrow prefix/suffix delegation routing (`Please ask a subagent to fix …`, `fix … with a subagent`), and ordinary questions, explanations, or ambiguous project discussion through the controller.
- Skill discovery streams at most 64 KiB of UTF-8 front matter per `SKILL.md`; large bodies remain discoverable without whole-file allocation, while oversized, unclosed, or invalid metadata is skipped safely.
- A bounded `AgentConsoleResponseRoute` contract with only `discussion | coding-task`; natural coding language can enter the same governed work path even when it misses the lexical fast path. Malformed, blank, oversized, or structurally invalid controller output safely remains discussion, and slash commands cannot be model-redispatched.
- Conversation Scope moved behind the compact Context Inspector and removed from ticket rows and the primary status workflow.
- Real Mission-qualified subagent/session cards and Issue Assignment Board rows with approve, launch, retry, cancel, assignment, review, repair, escalation, and queue actions.
- Timestamp-backed last activity with malformed-time rejection, plus recent-workspace relaunch commands that preserve the selected controller, shell-quote paths, copy safely, and never retarget the currently connected backend.
- Explicit and controller-classified coding prompts follow persisted prompt → proposal → canonical queue reload → exact Mission/scope/goal/criteria/path/policy/worker/origin equality → correlated approval → queued-session dispatch. Automatic and manual workers must explicitly be assignable, ungated, non-delegate, local/non-cloud, available, and worker-routed; missing authority metadata and any mismatched boundary fail closed.
- Queued Local Agent execution with isolated Git worktrees or bounded directory copies, cancellation, durable lifecycle, dead-runner requeue with capped recovery, and three-attempt UI dispatch recovery.
- Bounded three-round Ollama edit/test/repair feedback loop plus canonical post-review repair actions that survive reload and create exactly one repair worktree with the prior patch, allowed paths, and command policy, including Ad Hoc Delegation and legacy TUI/CLI review paths. Target tracked/untracked state and prior-session repair overlays are staged, integrity-bound, and durably marked before worktree effects so process loss cannot reread changed sources or apply an overlay twice.
- Lock-safe runtime/session persistence and atomic same-revision actions for preferences, Workspace Queue, Mission Drafts, Working Context, Workstation, and Review state.
- Correlation recovery for Workstation actions: session launches carry their request marker, approve/assign/cancel co-persist a Mission-runtime marker with the mutation, and completed acknowledgements retain receipts. Exact retry finalizes or replays the acknowledgement, reconciles missing audit records idempotently, and rejects correlation reuse with a different request without duplicate history, journal entries, or sessions. Workspace Queue validates exact proposal/decision requests, history-derived revisions, acknowledgements, and already-durable issue/session effects. Mission Draft validates one ordered prior/request/effect/acknowledgement lifecycle, exact confirm/abandon reasons, canonical coverage, and idempotent accepted-Issue recovery while allowing later governed Issue evolution.
- Minimal Bubblewrap filesystem views, sanitized child environments, read/write-aware Shell mounts, Additional Path Grants, exact non-symlink `/tmp` executable/script containment, strict UTF-8 output/path handling, trusted-helper resolution independent of child `PATH`, bounded Git probes/worktree operations, bounded model file/command plans, and allowed-path change rejection. Governed processes enter user/PID namespaces before applying address-space/file/open-file/process-count limits; a namespace-local subreaper supervises descendants, including a child that creates a new session or survives its leader. Timeout, output overflow, missing sandbox support, and namespace failures all fail closed.
- Contextual Additional Path Grant requests are typed append-only records rather than UI parsing of rejection prose. Each request binds request/correlation/Mission/path/access/duration/reason/action/time; grant and denial decisions validate the exact pending boundary, reject changed or replayed authority, and reconcile causal Console/Activity audit phases.
- Shell submission and approval poll durable `executing` metadata. Lost responses reload and reuse the exact correlation; a dead owner is persisted as `outcome-unknown`, audited causally before later Console/Activity entries, and never automatically executed again. Approval, denial, completion, unknown-outcome, and partial-audit replay are idempotent.
- Non-Git source copies, dependency-parent discovery, and capability walks are non-symlink and explicitly bounded; capability discovery stops at 20,000 visited entries or 1,024 matches, and dependency parents stop at 256.
- Strict controller/router/worker/delegate governance, including boolean registry validation and repair-role enforcement.
- Atomic Review Workspace status guards that reject queued/running work and recheck terminal/evidence state at mutation time.
- Real bounded `review.diff` artifacts, secret-content redaction, accurate ready-for-review versus accepted evidence state, validated-evidence-only Activity entries, and a typed inline Session Artifact viewer for exact registered review-safe text without raw host-path navigation.
- Only reader-compatible evidence references cross into UI controls; unsupported relative links are dropped instead of rendering dead navigation. Cancelled sessions project as failed/terminal rather than completed. Backend-rejected Workstation actions persist request/rejection phases, and a bounded workspace-scoped local continuity record preserves transport-failed action outcomes across a desktop refresh without treating them as accepted canonical state.

## Architecture Impact

- `albert_mvp/capabilities.py` discovers skills through bounded front-matter reads and projects commands, controllers, workers, and live Ollama availability.
- `albert_mvp/core.py` owns worktrees, queued/cancellable execution, iterative model plans, repairs, process isolation, evidence, crash recovery, and role governance.
- `albert_mvp/process_supervisor.py` is the PID-namespace-local subreaper that waits for or terminates descendants before the sandbox exits.
- `albert_mvp/workspace.py` owns typed controller responses, unified durable history, exact-boundary automatic delegation, recovery markers/receipts, bounded whole-file session-artifact validation with prefix-only display, Shell governance, Mission-qualified actions, atomic workspace mutations, and Activity records.
- `mission-control/src/App.tsx` owns the prompt-dominant UI projection, unified chronology, smart follow, command palette, optimistic interaction, typed route convergence, Mission Work, inline artifact viewing, bounded dispatch retry, recent-workspace relaunch controls, and bounded local continuity for terminal negative action outcomes.
- `mission-control/src/workstation-projection.ts` maps canonical sessions and issues into Mission-qualified cards, real artifact links, and typed actions.
- At the time of this report, `mission-control/bin/alfredo.js` separated install/backend/workspace paths, preferred an explicit or bundled backend coherently, excluded PRD files from actionable tracker selection, chose controllers, performed preflight, and prewarmed the selected local model. Current launcher startup no longer performs that prewarm.
- `mission-control/scripts/stage-backend.js` stages and removes the publish-only bundled backend with symlink, bytecode, cache, and special-file exclusions.
- `mission-control/src-tauri/src/lib.rs` keeps typed non-blocking transport parity with the Python Orchestrator.

## Verification

| Gate | Result |
|---|---|
| Python unit/integration/security/concurrency | 416 run: 415 passed, 1 optional live-Ollama smoke skipped |
| React/client/launcher/release seam | 215 passed across 8 files, including 122 rendered App tests and the real Python-backed release seam |
| TypeScript | `npm run typecheck` passed |
| Production UI build | `npm run build` passed |
| Rust/Tauri bridge | 36 passed through the full manifest test suite; formatting passed |
| Production Chromium geometry | Production build and discovery of all 4 viewports passed. Chromium could not start in the restricted environment (`sandbox_host ... Operation not permitted`); the required unsandboxed rerun was refused because the approval service had exhausted its usage quota. No geometry assertion executed in this final environment, so this gate is not claimed green. |
| Persistent transport latency | Warm p95 contract below 150 ms passed |
| Launcher and package (historical; not a release gate) | 32 focused entrypoint/package tests passed, including an actual packed consumer with the bundled backend; managed source-layout dry-run passed; `npm pack --dry-run --json` produced 93 entries / 478,092 bytes and cleaned its staging directory. This evidence was later rejected as proof of a public install path; use `npm run release:verify` plus authenticated publication and the registry-only smoke. |
| Session Artifact boundary | Exact registration/containment, symlink, full-file streamed NUL/invalid-UTF-8 rejection, bounded prefix retention, code-point-safe truncation/redaction, typed transport, retry, scroll/focus, and focus-restoration coverage passed |
| Documentation | Standards validation passed at A/100%; the audit completed with three known report-title connection heuristics and no standards issue |
| Independent two-axis review | Spec review found 0 issues and confirmed both open gates are recorded correctly. Standards review found three cross-language boundary drifts; all reproduced red, fixed, and passed focused plus full deterministic regression gates. |
| Scoped diff hygiene | Implementation-scoped `git diff --check` passed; the unrelated user-owned `AGENTS.md` EOF blank line remains outside this change |

## Explicit Follow-Up

- The replacement two-package release candidate is locally verified only on the Ubuntu 24.04 x64/glibc 2.39 baseline. The unauthenticated registry `E404` responses observed on 2026-07-12 prove neither name availability nor publishing authority; authenticated publication and the registry-only smoke remain open.
- The expanded current-build Chromium geometry run must be repeated in an environment allowed to start Playwright; this report does not reuse the earlier production 4/4 result as proof for the final merged worktree.
- [Ticket 28](../issues/28-validate-alfredo-accessibility-and-responsive-use.md) remains `ready-for-human` for hierarchy, keyboard, screen-reader, zoom/reflow, low-vision, and reduced-motion judgment.
- The optional live-Ollama smoke remains intentionally skipped in the deterministic Python gate; it is not required to establish the changed policy and persistence boundaries.

## Traceability

| Delivered boundary | Issue Slices | Permanent documentation |
|---|---|---|
| Self-contained npm entrypoint and headless grammar | [20](../issues/20-ship-alfredo-npm-workstation-entrypoint.md), [21](../issues/21-add-headless-alfredo-cli-grammar.md) | [Architecture](../System/project_architecture.md), [development workflow](../SOP/development_workflow.md) |
| Prompt workstation, live cards, operational detail, and typed actions | [22](../issues/22-build-prompt-dominant-workstation-shell.md), [23](../issues/23-project-live-agent-workstation-cards.md), [24](../issues/24-expand-workstation-cards-for-operational-detail.md), [25](../issues/25-route-first-consequential-workstation-action.md), [26](../issues/26-cover-governed-workstation-action-family.md) | [UX guidelines](../System/ux_guidelines.md), [command boundaries](../System/api_endpoints.md) |
| Restart/refresh continuity and release seam | [27](../issues/27-persist-alfredo-workstation-continuity.md), [29](../issues/29-add-alfredo-release-seam-verification.md) | [Persistence schema](../System/database_schema.md), [domain terminology](../../CONTEXT.md) |
| Automated accessibility hardening plus independent human gate | [28](../issues/28-validate-alfredo-accessibility-and-responsive-use.md) | [UX guidelines](../System/ux_guidelines.md) |

The broader live-model history remains valid: Qwen and Gemma models generated runnable edits, and Gemma4-26B completed a failed-first-pass repair loop. During this final managed run, `ollama list` confirmed `qwen3:14b`, `qwen2.5-coder:14b`, `qwen3.6:27b`, both Gemma workers, and DeepSeek were installed. A second post-hardening live inference was not admitted by the execution approval service after its usage quota was reached; deterministic Bubblewrap/controller/router/Ollama harness tests cover the changed boundary.

## User Launch

From the repository root:

```bash
ALFREDO_RUNTIME_ROOT="$HOME/.alfredo/runtime" node mission-control/bin/alfredo.js workstation --agent qwen3-14b
```

For the browser-rendered UI skeleton only:

```bash
cd mission-control
npm run dev
```

For the Tauri development window without the managed launcher:

```bash
cd mission-control
npm run desktop
```

## Related Documents

- [Alfredo Agent Workstation PRD](../issues/19-alfredo-agent-workstation-prd.md)
- [Project architecture](../System/project_architecture.md)
- [Persistence schema](../System/database_schema.md)
- [Command boundaries](../System/api_endpoints.md)
- [UX guidelines](../System/ux_guidelines.md)
- [Domain terminology](../../CONTEXT.md)
- [Active orchestration context](../Tasks/context.md)
- [Accessible responsive workstation report](2026-07-06-alfredo-accessibility-responsive-workstation.md)
- [Governed Shell Terminal report](2026-06-29-governed-shell-terminal.md)
