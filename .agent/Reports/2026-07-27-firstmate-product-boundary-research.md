# FirstMate product-boundary research

**Captured:** 2026-07-27  
**Question:** What does FirstMate actually provide, how does it compare with Alfredo's local-model workstation boundary, and what facts should inform—but not decide—the direction of the existing modernization Wayfinder map?

## Scope and method

This report is intentionally decision-neutral. It does not decide whether to continue, revise, or replace [Alfredo's modernization Wayfinder map](https://github.com/EricleungDK/Alfredo/issues/41).

FirstMate claims were checked against its repository README, architecture/configuration documents, operating contract, adapter source, GitHub metadata, and CI history. Alfredo claims were checked against this repository's domain context, architecture document, implementation reports, and the authoritative Wayfinder issue.

The FirstMate source links below are pinned to commit [`a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499`](https://github.com/kunchenguid/firstmate/commit/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499), the `main` head observed during this research, except for explicitly time-sensitive GitHub metadata links.

## Executive boundary

### Verified

FirstMate and Alfredo address the same high-level operator problem: one human-facing orchestrator coordinates parallel coding workers, isolates their changes, exposes their progress, and retains human authority over consequential outcomes.

They are not currently the same product shape:

- FirstMate describes itself as an **agent distro**, not a model, harness, MCP server, CLI, or installable app. A supported third-party coding-agent harness reads its `AGENTS.md`, skills, Bash tooling, policies, and local state conventions and thereby becomes the "first mate." ([FirstMate README](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/README.md#what-it-is))
- Alfredo is an **owned local coding-agent workstation**: a prompt-first React/Tauri desktop shell over an authoritative local Orchestrator, with Local Agents powered primarily through Ollama or governed local commands. ([Alfredo domain context](../../CONTEXT.md), [Albert architecture](../../docs/albert-architecture.md))

### Inference

The repositories overlap most strongly at the orchestration, worktree lifecycle, supervision, recovery, and observability layers. FirstMate is therefore evidence about an adjacent implementation/product layer, not by itself evidence that Alfredo's local-model workstation boundary is redundant.

A literal FirstMate clone would change Alfredo's product boundary unless the clone were substantially adapted: FirstMate delegates core agent execution to existing coding-agent CLIs, while Alfredo owns a local model/runtime, deterministic authority, and desktop interaction surface.

## Verified FirstMate facts

### Product goal and form

FirstMate's stated goal is to replace manual "tab juggling" with one liaison that dispatches and supervises a crew, then returns PRs, approved local merges, or investigation reports. The repository itself is the distribution: instructions, skills, helper scripts, policies, and state conventions; there is no separate FirstMate application install. ([README: What it is](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/README.md#what-it-is))

The first mate is explicitly the user's sole contact and is instructed not to perform project-specific work itself. It delegates coding, investigation, planning, reproduction, and audit work to disposable crewmates or persistent secondmates. ([FirstMate operating contract](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/AGENTS.md#1-identity-and-prime-directives))

### Supported coding-agent harnesses

The README names five verified **primary** harnesses:

- Claude Code
- Grok
- Pi
- Codex
- OpenCode

Claude Code, Grok, and Pi are the three co-primary recommendations. Codex and OpenCode are verified primary harnesses with additional supervision tradeoffs. ([README: Requirements and recommended harnesses](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/README.md#requirements))

The worker adapter layer additionally includes **Kimi Code CLI**. FirstMate's adapter reference contains verified facts for `claude`, `codex`, `opencode`, `pi`, `grok`, and `kimi`; Kimi is supported for worker dispatch but is outside the primary turn-end-guard scope. ([Harness adapter reference](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/harness-adapters/SKILL.md#primary-turn-end-guard))

Crewmates mirror the primary harness by default. A static crew setting or per-task dispatch profile can choose a different harness, model, and reasoning-effort axis; secondmates have a separate harness setting, and a task-specific operator instruction overrides the standing profile. ([Harness adapter routing](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/harness-adapters/SKILL.md#harness-adapters), [architecture: dispatch profiles](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#dispatch-profiles))

FirstMate passes model choices through each harness's verified model-selection surface. Its adapter document explicitly treats model/provider availability as environment- and account-dependent discovery rather than a stable FirstMate-owned model namespace. ([Harness model support discovery](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/harness-adapters/SKILL.md#model-support-discovery))

Codex CLI is supported as a harness, but the **Codex Desktop app is not a selectable runtime backend**. FirstMate documents the absence of a supported shell-callable transport that can create, continue, inspect, and archive the same visible Desktop-owned thread over its lifecycle. ([Codex App backend boundary](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/codex-app-backend.md))

No `ollama` occurrence or explicit "local model" runtime contract was found in FirstMate's public source through GitHub code search on 2026-07-27. That negative result does **not** prove that a supported harness cannot itself be configured for a local provider; it establishes only that Ollama/local inference is not a FirstMate-owned integration documented in the reviewed repository. ([GitHub code search: `ollama`](https://github.com/search?q=repo%3Akunchenguid%2Ffirstmate+ollama&type=code), [GitHub code search: `"local model"`](https://github.com/search?q=repo%3Akunchenguid%2Ffirstmate+%22local+model%22&type=code))

### Orchestration model

The normal topology is:

1. The user talks to one first mate.
2. The first mate selects a project, task shape, harness/model profile, and runtime backend.
3. Each crewmate runs as one autonomous coding-agent process in its own endpoint and isolated Git worktree.
4. The first mate supervises status and delivery, then returns the outcome to the user.

FirstMate supports two direct task shapes: `ship` tasks change a project and follow its delivery mode, while `scout` tasks produce standalone reports and never push. ([Architecture: task shapes](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#two-task-shapes))

Delivery modes are explicit:

- `no-mistakes` runs a validation pipeline;
- `direct-PR` opens a PR without that pipeline;
- `local-only` keeps work local until an approved fast-forward merge;
- `+yolo` optionally grants standing authority for routine decisions while retaining stronger destructive, irreversible, and security-sensitive boundaries.

([Architecture: project modes](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#project-modes-are-explicit), [operating contract: authority](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/AGENTS.md#7-task-lifecycle))

Optional **secondmates** are persistent subordinate supervisors with their own `FM_HOME`, state, backlog, project clones, session lock, charter, and crewmates. They remain direct reports of the primary first mate, are idle by default, and do not spawn further secondmates. ([Architecture: optional secondmates](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#optional-secondmates))

The first mate is forbidden from writing project code directly, merging without the configured human authority, or tearing down unlanded work. Crewmates make changes; Git worktrees and guarded merge/teardown paths isolate lifecycle ownership. ([FirstMate hard rules](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/AGENTS.md#1-identity-and-prime-directives))

### Supervision, recovery, and observability

FirstMate's default observability is terminal- and record-based, not a FirstMate-owned desktop GUI:

- each worker appears in a visible tmux window, Herdr/Zellij tab, cmux workspace, or Orca terminal;
- the operator can watch a worker or type directly into its endpoint;
- bounded capture and send helpers allow supervision without attaching to the terminal;
- tmux is the verified reference backend, while Herdr, Zellij, Orca, and cmux are documented as experimental task-spawn backends.

([README: visible crew](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/README.md#features), [tmux backend](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/tmux-backend.md#watching-the-crew), [architecture: runtime backends](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#runtime-session-backends))

A Bash watcher polls or waits on the fleet, classifies status/liveness signals without an LLM turn, writes actionable wakes to a durable queue, and wakes the first mate only when attention is needed. Harness-specific turn-end guards prevent a primary session from silently ending while work still needs supervision. ([Architecture: event-driven supervision](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#event-driven-supervision))

Crew status files are append-only event logs rather than trusted current-state fields. Current state is reconciled from run identity, backend liveness/busy evidence, metadata, endpoint probes, and status history. A structured fleet snapshot includes backlog items, task metadata, current worker state, endpoint observations, PR/report pointers, scout reports, and bounded secondmate summaries; a separate view renders it as Markdown. ([Architecture: current state and fleet snapshot](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md#event-driven-supervision))

Built-in operator views include `/ahoy` for recent event/decision recap and `/bearings` for a standalone current-status report. ([README: built-in skills](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/README.md#built-in-skills))

Durable records live under `data/`, volatile coordination and append-only runtime events under `state/`, operating choices under `config/`, and project clones under `projects/`. Startup reconciles durable records with the active session backend so a restart is intended to be a non-event rather than relying on conversation memory. ([Configuration: home layout](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/configuration.md#operational-home-layout-and-state), [operating contract: recovery](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/AGENTS.md#5-recovery))

### Architecture and technology stack

FirstMate is overwhelmingly Shell. GitHub's language API reported 4,484,344 Shell bytes, 70,022 JavaScript bytes, 44,370 TypeScript bytes, and 11,979 Python bytes—approximately 97.3% Shell in the detected source set. ([GitHub language metadata](https://api.github.com/repos/kunchenguid/firstmate/languages))

The core implementation consists of:

- Markdown instruction, skill, policy, and configuration contracts;
- plain Bash helper and test scripts;
- a Bash watcher, durable local files, and Git worktree lifecycle tooling;
- small JavaScript/TypeScript harness integrations such as OpenCode plugins and Pi extensions.

([Contributor conventions](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/CONTRIBUTING.md#repo-conventions), [architecture](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md))

The baseline external toolchain includes a supported coding-agent harness, Git, authenticated GitHub CLI, and a session backend; tmux is the default. FirstMate also uses `treehouse` for worktree allocation on most backends and `tasks-axi` for its default Markdown backlog mutations. ([README: requirements](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/README.md#requirements), [configuration: backlog backend](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/configuration.md#backlog-backend-taskstoml--configbacklog-backend))

FirstMate's safety boundary combines a read-only primary orchestrator, isolated Git worktrees, explicit delivery/merge authority, guarded teardown, and adapter-specific liveness/submission checks. The worker launch source also deliberately starts common harnesses in autonomous modes: Claude with `--dangerously-skip-permissions`, Codex with `--dangerously-bypass-approvals-and-sandbox`, OpenCode with all permissions allowed, Grok with `--always-approve`, Kimi with `--auto`, and Pi through its autonomous extension path. ([Worker launch templates](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-spawn.sh#L402-L447), [Pi adapter fact](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/harness-adapters/SKILL.md#pi-verified-2026-06-11))

### License, activity, and maturity signals

FirstMate is MIT licensed, allowing use, modification, redistribution, sublicensing, and sale subject to preserving the copyright and license notice. ([MIT license](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/LICENSE))

Time-sensitive GitHub snapshot captured 2026-07-27:

- repository created 2026-06-12 and last pushed 2026-07-26;
- 1,983 stars, 646 forks, and 12 watchers;
- 275 commits on the default branch;
- 88 open issues and 279 open pull requests;
- no tags and no releases;
- 18 listed contributors; GitHub credited 245 contributions to the owner and at most 4 to any other contributor.

([Repository metadata API](https://api.github.com/repos/kunchenguid/firstmate), [commit history](https://github.com/kunchenguid/firstmate/commits/main/), [open issues](https://github.com/kunchenguid/firstmate/issues?q=is%3Aissue+state%3Aopen), [open pull requests](https://github.com/kunchenguid/firstmate/pulls?q=is%3Apr+is%3Aopen), [tags](https://github.com/kunchenguid/firstmate/tags), [releases](https://github.com/kunchenguid/firstmate/releases), [contributors API](https://api.github.com/repos/kunchenguid/firstmate/contributors?per_page=100))

The repository has substantial automated quality machinery for its age: pinned ShellCheck, a coverage-partition guard, parallel and serial behavior-test lanes, a real Herdr lane, stock macOS Bash 3.2 compatibility, and repository invariants. ([CI workflow](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.github/workflows/ci.yml))

The ten most recent `main` push CI runs inspected on 2026-07-27 all completed successfully; the newest was [CI run 30194066930](https://github.com/kunchenguid/firstmate/actions/runs/30194066930) for the pinned source commit.

These signals point in different directions and should not be collapsed into one maturity label:

- **Strong activity/adoption signals:** rapid commits, substantial stars/forks, extensive operational docs, empirical adapter verification, and broad CI.
- **Early/stability-risk signals:** approximately six weeks old, no versioned tags or releases, very high open issue/PR counts, several runtime backends explicitly experimental, and contribution history concentrated in one maintainer.

## Verified Alfredo baseline relevant to the comparison

Alfredo defines its Orchestrator as the authority process that validates task graphs, creates isolated workspaces, enforces allowed paths and command rules, records factual status, collects evidence, and blocks invalid work. Local Agents run primarily through Ollama or governed local commands and must return an Evidence Package. ([Domain context](../../CONTEXT.md))

The current backend is a dependency-light Python runtime that owns contracts, lifecycle state, command policy, evidence validation, repair routing, and PR preparation. React/TypeScript and Tauri provide the desktop product shell over that authoritative backend. ([Albert architecture](../../docs/albert-architecture.md))

Alfredo's current product surfaces include a continuous Agent Console, a Mission Work lane, a work-centered Mission Execution Tree, Workspace Queue governance, Review Workspace, Activity Journal, Session Artifact Viewer, and governed Shell Terminal. Its worker boundary includes isolated worktrees, explicit allowed paths, command classification, a minimal Bubblewrap filesystem view, and Evidence Package review. ([Domain context](../../CONTEXT.md))

The existing Wayfinder map retains React/Tauri, evaluates the backend authority boundary, requires reliable Mission formation and dispatch, chooses a work-centered Mission Execution Tree instead of flat agent cards, separates control-plane latency from local-model inference and outcome latency, and requires incremental migration with rollback unless evidence justifies a big-bang rewrite. ([Authoritative modernization map](https://github.com/EricleungDK/Alfredo/issues/41))

The current performance baseline found that the Python control plane operates at millisecond scale while the live local-model worker step dominated the sampled end-to-end path at about 37.8 seconds. ([Architecture and performance baseline](./2026-07-23-alfredo-architecture-performance-baseline.md))

The latest workflow diagnosis found two product-boundary defects before effectful orchestration: Starting Location is collapsed into an already-selected Coding Workspace, and controller prose can sound successful without a correlated Orchestrator effect receipt. The healthy governed proposal-to-session path was not implicated. ([Workspace selection and false-success diagnosis](./2026-07-24-workspace-selection-false-success-diagnosis.md))

## Verified capability overlap and differences

| Dimension | FirstMate | Alfredo |
|---|---|---|
| Human interaction | One first mate is the only normal liaison. | One continuous Agent Console and Mission Commander relationship. |
| Parallelism | Autonomous workers in separate session endpoints and Git worktrees; optional persistent secondmates supervise their own crews. | Local Agent sessions in isolated worktrees; Mission Execution Tree projects Issue Slices, Ad Hoc Delegations, nested work, and review/repair state. |
| Agent runtime | Existing Claude Code, Grok, Pi, Codex, OpenCode, or Kimi CLI adapters; harness owns model/provider execution. | Alfredo-owned runner boundary, currently Ollama-first and provider-neutral at the assignment model; deterministic Python Orchestrator remains authority. |
| Orchestrator implementation | A supported general-purpose agent follows `AGENTS.md` plus skills and Bash scripts. | A deterministic local backend validates and mutates canonical state; models act only within roles and governed routes. |
| Observability | Live terminal windows/tabs/workspaces, bounded pane capture, append-only status events, structured fleet snapshots, `/ahoy`, `/bearings`, and watcher notifications. | React/Tauri Mission Execution Tree, Agent Console, queues, evidence review, artifact viewing, Activity Journal, and authoritative workspace projections. |
| Recovery | Disk state plus session backend inventory; durable wake queue; liveness/current-state reconciliation; watcher and turn-end guards. | Persisted canonical snapshots/events, idempotent correlations and receipts, restart recovery, repair-session lineage, and runtime reconciliation. |
| Isolation and authority | Separate Git worktrees; primary does not edit projects; guarded merge/teardown; worker CLIs run autonomously with permission bypasses appropriate to unattended operation. | Separate Git worktrees plus command policy, allowed paths, Additional Path Grants, sanitized execution, Bubblewrap view, evidence validation, and Mission Commander review. |
| Delivery objects | Ship tasks yield PR/local changes; scout tasks yield reports; delivery modes govern validation and merging. | Issue Slices and Ad Hoc Delegations yield Evidence Packages, review/repair decisions, and PR-ready state without equating completion with merge. |
| Local-model ownership | No FirstMate-owned Ollama/local-inference contract found; model selection is passed through the chosen harness. | Local-model execution and its latency, availability, routing, and evidence are explicit product concerns. |
| Product surface | Repository-based agent distro using terminal/session backends; no FirstMate-owned app. | Installable desktop workstation with React/Tauri UI and local backend. |
| GitHub dependency | Quick start requires authenticated GitHub CLI, although `local-only` delivery exists. | GitHub Issues is the project tracker in this repository, but local worker execution and local-only evidence/review are owned inside Alfredo. |

## Clearly marked inferences for the Wayfinder discussion

The following are interpretations of the verified facts, not claims made by either project:

1. **The closest reusable FirstMate seams are below the UI.** Its durable wake queue, liveness reconciliation, harness-specific adapter contract, supervision backstops, worktree teardown proofs, dispatch profiles, and secondmate recovery rules are concrete reference material for Alfredo's reliability work.
2. **FirstMate's heterogeneous harness support and Alfredo's provider-neutral model assignment solve different abstraction levels.** FirstMate chooses a complete coding-agent runtime; Alfredo currently chooses a model/runner inside its own governed coding-agent system.
3. **FirstMate's "visible crew" is operational visibility, while Alfredo is pursuing productized work visibility.** Terminal panes and structured status records can complement, but do not directly substitute for, Alfredo's Mission Execution Tree, Queue, review, and Activity Journal contracts.
4. **Git worktree isolation is not equivalent to Alfredo's command/filesystem sandbox.** FirstMate intentionally bypasses worker-harness approval/sandbox prompts and relies on delegation briefs, worktrees, project modes, merge authority, and teardown guards. Alfredo's Bubblewrap, path grants, argv command policy, and evidence validation represent a distinct containment/governance boundary.
5. **A "FirstMate for local models" is not merely a UI or naming change.** It would still need a defined local-agent harness/runtime, GPU and model-capacity scheduling, model availability and failure semantics, bounded tool execution, evidence truth, and recovery behavior—areas Alfredo already partially owns.
6. **FirstMate's traction is evidence of interest, not yet evidence of a stable compatibility target.** Its activity and CI are strong, but the lack of releases/tags, rapid adapter changes, experimental backends, and maintainer concentration make copying internal contracts verbatim a potential churn risk.
7. **The FirstMate discovery does not answer the Python-versus-Rust question in the current map.** FirstMate is predominantly Bash and prompt contracts; its architecture demonstrates that orchestration reliability can be built without a Rust control plane, but it does not provide comparative evidence about Alfredo's Python/Tauri latency, state authority, or migration cost.
8. **The MIT license permits reference, reuse, and modification with attribution, but license compatibility does not decide product fit.** Architectural fit, security posture, long-term maintenance, and desired user experience remain separate questions.

## Unanswered questions

These are still open after the primary-source review:

1. Is Alfredo's non-negotiable identity an owned, local-first model workstation, or is local execution one provider option inside a broader coding-harness orchestrator?
2. What does "local" need to guarantee: inference locality, offline operation, no third-party agent subscription, source-code privacy, local state, or all of these?
3. Should Alfredo ever dispatch complete external coding-agent harnesses, or should it continue to own the worker loop and integrate models/providers below that boundary?
4. Can a supported FirstMate harness be configured with a fully local provider while preserving its supervision and tool semantics? The FirstMate repository does not establish this.
5. How should multiple local workers share constrained GPU/VRAM and model residency? FirstMate's terminal parallelism does not document local accelerator scheduling.
6. Which observability experience do target users value: direct terminal intervention, a work-centered GUI tree, or both? FirstMate adoption metrics do not answer that interaction question.
7. Which FirstMate reliability mechanisms outperform Alfredo's current equivalents under the same failures: dropped completion signal, dead worker, stale status, lost acknowledgement, restart, dirty worktree, or merge race?
8. Does Alfredo want FirstMate-style persistent second-tier supervisors, and if so, what canonical Mission/authority boundary would they own?
9. Is FirstMate's autonomous permission-bypass posture acceptable for a local-model product, or must Alfredo retain its finer command/path approval model?
10. Which FirstMate contracts are stable enough to reuse directly given the absence of tagged releases and the current pace of changes?
11. Do FirstMate's 279 open PRs represent a meaningful contributor backlog, automation behavior, or another workflow pattern? Counts alone cannot establish project health.
12. Should the current Wayfinder destination remain an Alfredo modernization blueprint, or should a separate product-boundary decision precede its remaining Rust and rollout tickets? This is a human product decision, not resolved by this research.

## Source index

### FirstMate primary sources

- [Repository](https://github.com/kunchenguid/firstmate)
- [README](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/README.md)
- [Architecture](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/architecture.md)
- [Configuration](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/configuration.md)
- [Operating contract](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/AGENTS.md)
- [Harness adapters](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.agents/skills/harness-adapters/SKILL.md)
- [Worker spawn implementation](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/bin/fm-spawn.sh)
- [tmux backend](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/tmux-backend.md)
- [Codex App backend boundary](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/docs/codex-app-backend.md)
- [Contributing and test conventions](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/CONTRIBUTING.md)
- [CI workflow](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/.github/workflows/ci.yml)
- [License](https://github.com/kunchenguid/firstmate/blob/a5fe1bcc8c2ac01951e6a68d1c8b1b1ecae21499/LICENSE)

### Alfredo primary sources

- [Domain context](../../CONTEXT.md)
- [Albert architecture](../../docs/albert-architecture.md)
- [Authoritative modernization Wayfinder map](https://github.com/EricleungDK/Alfredo/issues/41)
- [Architecture and performance baseline](./2026-07-23-alfredo-architecture-performance-baseline.md)
- [Workspace selection and false-success diagnosis](./2026-07-24-workspace-selection-false-success-diagnosis.md)
