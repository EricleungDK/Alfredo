# Documentation Index

**Last updated:** 2026-09-29

## Alfredo (current product)

The native terminal `alfredo-tui` is documented outside this folder:

- [Root README](../README.md) covers install, quickstart, keys, flags and troubleshooting.
- [alfredo-tui/README.md](../alfredo-tui/README.md) covers day-to-day use, the agent view, build, test and packaging.
- [Terminal reference](../alfredo-tui/docs/reference.md) records detailed behavior and guarantees.
- [Install reference](../alfredo-tui/docs/install-reference.md) records state formats and upgrade details.
- [Contributing](../CONTRIBUTING.md) explains the test-first workflow and CI gates.
- [Changelog](../CHANGELOG.md) lists user-visible changes.
- [Current status](../.agent/Tasks/STATUS.md) is the source of truth for release readiness.

## Legacy desktop workstation (Albert / Mission Control)

Superseded by the terminal; kept for reference.

- [Legacy overview](legacy.md) is the previous root README for the React/Tauri app and Python orchestrator.
- [Architecture and design](albert-architecture.md) explains the orchestrator boundaries, runtime flow, model roles, and design constraints.
- [MVP mapping and status](albert-mvp-status.md) maps the original product idea to implemented code.
- [Usage guide](albert-usage.md) gives the basic commands for the Python MVP.
- [Architecture decision records](adr/) capture the Tauri, journal and provider decisions.

## Agent Configuration

- [Issue tracker](agents/issue-tracker.md) explains the GitHub PRD-parent, ordered sub-issue, and dependency conventions.
- [Triage labels](agents/triage-labels.md) records the default triage vocabulary.
- [Domain docs](agents/domain.md) points agents at the root project context.

## Historical Sources

The `.agent/` directory contains implementation reports and orchestration history; start at [.agent/README.md](../.agent/README.md). `.scratch/` is the read-only archive of tracker records migrated to GitHub on 2026-07-23. Use the docs above and GitHub Issues as the current view, then consult the archive for provenance.
