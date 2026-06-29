# Albert - Project Documentation

**Last Updated**: 2026-06-18
**Status**: Tested MVP complete; Qwen-controlled delegation configured; Gemma and repair relaunch workflows live-verified

## Quick Start

1. Read [Project Architecture](System/project_architecture.md) for the current Orchestrator, TUI, and runner boundaries.
2. Follow [Development Workflow](SOP/development_workflow.md) for local development.
3. Check [Current Tasks](Tasks/context.md) for current model registry, completed backlog, and pending local delegate smoke-test work.
4. Run `python3 -m unittest discover -s tests` before claiming changes are complete.

## Documentation Structure

```text
.agent/
├── System/                        # System architecture and design
│   ├── project_architecture.md    # Current architecture, command surface, runner boundaries
│   ├── database_schema.md         # Placeholder; no database in this stdlib MVP
│   └── ux_guidelines.md           # Design principles and UX rules
│
├── Tasks/                         # Roadmap and implementation status
│   ├── context.md                 # Central context file
│   └── README.md                  # Phase roadmap
│
├── SOP/                           # Standard operating procedures
│   ├── development_workflow.md    # Dev setup and daily workflow
│   ├── database_migrations.md     # Placeholder; no database migrations in this MVP
│   └── deployment.md              # Packaging and distribution notes
│
├── Reports/                       # Implementation reports
│   ├── 2026-06-15-local-coding-agent-mvp.md
│   ├── 2026-06-16-albert-tui-ollama-completion.md
│   ├── 2026-06-16-albert-repair-relaunch.md
│   ├── 2026-06-16-gemma-live-verification.md
│   ├── 2026-06-16-gemma26-repair-loop-verification.md
│   └── 2026-06-18-qwen-controlled-delegation.md
│
└── README.md                      # This file
```

## How do I...

| Question | Document |
|----------|----------|
| Understand the architecture? | [project_architecture.md](System/project_architecture.md) |
| Set up dev environment? | [development_workflow.md](SOP/development_workflow.md) |
| See current model assignments and pending work? | [context.md](Tasks/context.md) |
| See the roadmap? | [README.md](Tasks/README.md) |
| Review implementation evidence? | [Qwen delegation report](Reports/2026-06-18-qwen-controlled-delegation.md), [2026-06-16 TUI/Ollama report](Reports/2026-06-16-albert-tui-ollama-completion.md), [repair relaunch report](Reports/2026-06-16-albert-repair-relaunch.md), [Gemma live verification](Reports/2026-06-16-gemma-live-verification.md), and [Gemma26 repair loop](Reports/2026-06-16-gemma26-repair-loop-verification.md) |
