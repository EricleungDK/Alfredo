# Alfredo / Albert Project Documentation

**Last Updated**: 2026-08-13
**Status**: Prompt-first Alfredo workstation with a persistent Apple-container canonical localhost environment, explicit Coding Workspace/Mission continuity, and receipt-bound conversational action truth; authenticated npm publication, registry-only install, and the documented human/release follow-up remain explicit

## Quick Start

1. Read [Project Architecture](System/project_architecture.md) for the current workstation, Orchestrator, and runner boundaries.
2. Follow [Development Workflow](SOP/development_workflow.md) for local development.
   On macOS, reuse `./scripts/apple-container-dev start` and keep `http://127.0.0.1:1420` open instead of starting a competing host Vite process.
3. Check [Current Tasks](Tasks/context.md) for the model registry, exact verified release state, publication boundary, and independent human follow-up.
4. Read the [2026-07-12 install and Queue acceptance correction](Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md) before relying on the superseded 2026-07-11 packaging evidence.
5. Run every release gate recorded in the correction report and keep ticket 20 open until registry publication plus a registry-only smoke.

## Documentation Structure

```text
.agent/
├── System/                        # System architecture and design
│   ├── project_architecture.md    # Current architecture, command surface, runner boundaries
│   ├── database_schema.md         # Versioned JSON persistence and relationships
│   ├── api_endpoints.md           # Python/Tauri/React command boundaries
│   └── ux_guidelines.md           # Design principles and UX rules
│
├── Tasks/                         # Roadmap and implementation status
│   ├── context.md                 # Central context file
│   └── README.md                  # Phase roadmap
│
├── SOP/                           # Standard operating procedures
│   ├── development_workflow.md    # Dev setup and daily workflow
│   └── database_migrations.md     # No-SQL persistence migration policy
│
├── Reports/                       # Implementation reports
│   ├── 2026-06-15-local-coding-agent-mvp.md
│   ├── 2026-06-16-albert-tui-ollama-completion.md
│   ├── 2026-06-16-albert-repair-relaunch.md
│   ├── 2026-06-16-gemma-live-verification.md
│   ├── 2026-06-16-gemma26-repair-loop-verification.md
│   ├── 2026-06-18-qwen-controlled-delegation.md
│   ├── 2026-07-11-alfredo-one-shot-workstation.md
│   ├── 2026-07-12-alfredo-install-queue-acceptance-correction.md
│   ├── 2026-08-09-issue-67-retirement-lifecycle.md
│   ├── 2026-08-09-issue-68-retirement-storage.md
│   └── 2026-08-13-issue-70-inference-qualification.md
│
└── README.md                      # This file
```

## System

- [Root README](../README.md) provides the source launcher, GUI skeleton, Tauri window, and release-gate commands.
- [Domain terminology](../CONTEXT.md) is the single-context ubiquitous-language reference.
- [Project architecture](System/project_architecture.md) is the authoritative component and trust-boundary map.
- [Persistence schema](System/database_schema.md) documents the JSON stores, identities, locking, and migrations.
- [API endpoints](System/api_endpoints.md) documents the CLI, persistent transport, Tauri, and React request/response boundaries.
- [UX guidelines](System/ux_guidelines.md) records the prompt-first layout, readability, reflow, interaction, and accessibility rules.

## Tasks

- [Active orchestration context](Tasks/context.md) is the source of truth for the current mission, assignments, blockers, and release state.
- [Consolidated project context](Tasks/consolidated_context.md) is a generated read-only snapshot of recent reports and System docs; `Tasks/context.md` remains authoritative.
- [Roadmap](Tasks/README.md) summarizes completed work and explicitly separate human follow-up.
- [Alfredo Agent Workstation PRD](issues/19-alfredo-agent-workstation-prd.md) and [Issue Slices 20–29](issues/) retain acceptance and current triage state.

## SOP

- [Development workflow](SOP/development_workflow.md) provides setup and verification commands.
- [Database migrations](SOP/database_migrations.md) explains the no-SQL persistence migration policy.

## Reports

- [Alfredo install and Queue acceptance correction](Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md) is the current release/acceptance report and explicitly supersedes the 2026-07-11 package-completion claim.
- [Conversational action receipt binding](Reports/2026-08-02-conversational-action-receipts.md) records the Issue #59 implementation, public seams, and verification evidence.
- [Wayfinder Shared Understanding Gate](Reports/2026-08-03-wayfinder-shared-understanding-gate.md) records the Issue #60 canonical entry, gate, and projection contract.
- [Functional localhost workstation diagnosis](Reports/2026-08-03-functional-localhost-workstation.md) records the browser/native launch root causes, development bridge boundary, macOS path correction, lifecycle/concurrency prevention, and verification.
- [Persistent Apple container development environment](Reports/2026-08-03-apple-container-development-environment.md) records the named-container lifecycle, isolated volumes, loopback forwarding boundary, canonical health check, restart evidence, and agent handoff.
- [Governed Mission Work actions](Reports/2026-08-06-issue-64-governed-mission-work-actions.md) records Issue #64's retained completed-history archive/restore, inherited repair preview, blocker truth, and validation evidence.
- [Deterministic runner supervision](Reports/2026-08-09-issue-65-deterministic-runner-supervision.md) records Issue #65's advisory observation ledger, exact recovery boundary, receipt replay, healthy silence, and Mission Work decision projection.
- [Retirement preservation proof](Reports/2026-08-09-issue-66-retirement-preservation.md) records Issue #66's pre-execution budget reservation, lifecycle lock, Worktree Identity and quiescence gates, manifest integrity, and clean-room reconstruction boundary.
- [Retirement lifecycle](Reports/2026-08-09-issue-67-retirement-lifecycle.md) records Issue #67's outcome policy, passive grace, durable recovery phases, bounded removal attempts, exact Git/directory removal, and verified repair lineage.
- [Retirement storage and blocked outcomes](Reports/2026-08-09-issue-68-retirement-storage.md) records Issue #68's aggregate Snapshot Storage Budget, retention/reclamation/pinning, deterministic inspection, and replay-safe retry/export/discard actions.
- [Local Inference Profile qualification and promotion](Reports/2026-08-13-issue-70-inference-qualification.md) records Issue #70's governed fixture family, reviewed-outcome/timing reports, bounded context and prefix measurements, exact runtime pinning, and rollback state.
- [Workspace selection and false-success diagnosis](Reports/2026-07-24-workspace-selection-false-success-diagnosis.md) records the defect and the receipt-truth blueprint resolved by Issue #59.
- [Alfredo one-shot workstation correction](Reports/2026-07-11-alfredo-one-shot-workstation.md) remains the historical implementation report.
- Historical implementation reports remain in [`Reports/`](Reports/) and are indexed from [project architecture](System/project_architecture.md#implementation-report-index).

## How do I...

| Question | Document |
|----------|----------|
| Understand the architecture? | [project_architecture.md](System/project_architecture.md) |
| Set up dev environment? | [development_workflow.md](SOP/development_workflow.md) |
| Start or restart the persistent browser workstation? | [Apple container workflow](SOP/development_workflow.md#persistent-apple-container-browser-ui-preferred-on-macos) |
| See current model assignments and pending work? | [context.md](Tasks/context.md) |
| See the roadmap? | [README.md](Tasks/README.md) |
| Review current implementation evidence? | [Install and Queue acceptance correction](Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md), then the historical [one-shot workstation report](Reports/2026-07-11-alfredo-one-shot-workstation.md) |
| Review earlier model/runner evidence? | [Qwen delegation report](Reports/2026-06-18-qwen-controlled-delegation.md), [TUI/Ollama report](Reports/2026-06-16-albert-tui-ollama-completion.md), [repair relaunch report](Reports/2026-06-16-albert-repair-relaunch.md), [Gemma live verification](Reports/2026-06-16-gemma-live-verification.md), and [Gemma26 repair loop](Reports/2026-06-16-gemma26-repair-loop-verification.md) |
