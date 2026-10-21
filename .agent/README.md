# Alfredo Project Documentation

> **Start here: [Tasks/STATUS.md](Tasks/STATUS.md)** — current truth. The product is
> the native terminal `alfredo-tui`.
> `Tasks/context.md` is a large historical log; do not rely on it for current state.

**Last Updated**: 2026-10-20
**Status**: Product is the native terminal `alfredo-tui` (v0.1.0 on `main` plus the `[Unreleased]` work listed in `Tasks/STATUS.md`: wake-based run loop, headless driver, `/clean`, `--on-finish`, `--num-ctx`, `@path`; the `v0.1.0` release is still a draft, live-model run open).

## Quick Start

Plans live in [Tasks/](Tasks/README.md); runnable commands in
[`alfredo-tui/README.md`](../alfredo-tui/README.md); current evidence in the
[terminal foundation report](Reports/2026-09-13-rust-terminal-foundation.md).

## Structure

- `System/`: architecture.
- `Tasks/`: [status](Tasks/STATUS.md), [roadmap](Tasks/README.md), plans, historical `context.md`.
- `SOP/`: [development workflow](SOP/development_workflow.md), [database migrations](SOP/database_migrations.md), [README recording](SOP/recording.md).
- `Reports/`: dated evidence (JSON/Markdown) for the terminal.

- [Domain terminology](../CONTEXT.md)
- [Project architecture](System/project_architecture.md)
