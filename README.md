# Alfredo Local Coding-Agent Workstation

**Last updated:** 2026-08-03

Alfredo is a local-first coding-agent workstation with a prompt-dominant React/Tauri interface and an authoritative Python Orchestrator. Use it for fast project discussion, skills and slash commands, governed coding tasks, and visible Local Agent work. The secondary Mission Work lane shows real subagent sessions, workable Issue Slices, evidence, and typed review/retry/cancel actions.

The repository retains `Albert` and `Mission Control` compatibility names in Python modules and older documentation.

## Install and Start the Workstation

The production release candidate is a small `alfredo-agent` CLI/backend package plus an exact-version `alfredo-agent-linux-x64-gnu` AppImage package. On 2026-07-13 the rebuilt production gate passed a meta-only isolated-registry install, plain-PATH `alfredo` launch, frontend load, and installed-backend workspace snapshot. The artifact gate reports pass/publishable true for the 77,761,016-byte AppImage with SHA-256 `3faec58bc4e4a0b1c825cb58a3ec5475e5daac36bb0c839e0699ae6ddf006be2`; the exact audited tarballs also pass the independent `release:check` and npm publish dry-runs. The packages are still not public: local npm authentication is absent, and the protected hosted provenance/publish/public-reinstall workflow has not run. Do not treat the following registry command as available until ticket 20 records that final gate.

After publication, start Alfredo from any directory you want to use as the Starting Location:

```bash
npm install --global alfredo-agent
cd /path/to/your/projects
alfredo
```

The invocation directory is not silently selected. Agent Console asks you to choose an exact existing Git repository or create one below the Starting Location; only the acknowledged repository becomes the Coding Workspace. Mission choice follows separately. Use `alfredo --agent qwen3-14b` or `alfredo workstation --agent qwen3-14b` to select the initial controller explicitly. The installed command launches the packaged native desktop directly; it does not invoke Cargo, Vite, the Tauri CLI, or a source checkout. The AppImage extraction fallback is applied internally, so the command does not require the user to know AppImage/FUSE flags.

The currently verified native artifact baseline is Ubuntu 24.04 x64 with glibc 2.39; broader Ubuntu or glibc compatibility is not yet claimed. Consumer prerequisites include Node.js 20+ and npm for installation/the CLI shim, plus Python 3 and Bubblewrap for the backend. Missing Ollama or a selected model no longer blocks the desktop from opening; those are required only when local-model work is actually requested. Use `ALBERT_PYTHON` if Python is not available as `python3`, or `ALBERT_BACKEND_ROOT` for an intentional backend override.

To build and verify the exact release locally, including a real AppImage build, meta-only install through an isolated npm registry, PATH resolution, plain `alfredo` invocation, and a bounded GUI-plus-backend readiness smoke:

```bash
cd /path/to/local-coding-agent/mission-control
npm install
npm run release:verify
npm run release:check
```

For source development, the repository launcher deliberately retains the Tauri development path:

```bash
cd /path/to/local-coding-agent/mission-control
npm ci
cargo --version
cd ..
node mission-control/bin/alfredo.js workstation --agent qwen3-14b
```

The source launcher checks for the lockfile-installed local Tauri CLI and Cargo before spawning the desktop process. A missing prerequisite fails with an Alfredo preflight message and a copyable repair command instead of exposing a raw child-process error. These development requirements do not apply to the packaged native desktop.

For the preferred persistent browser-development workstation on a Mac with Apple's `container` CLI (not Docker), run this once from the repository root:

```bash
./scripts/apple-container-dev setup
```

Then keep `http://127.0.0.1:1420` open. The named `alfredo-dev` container runs detached, keeps its Linux Node dependencies, Cargo output, Rust toolchain, and Alfredo runtime in named volumes, and bind-mounts the repository's parent so host code changes appear immediately and sibling repositories remain available below `/workspace`. Polling-backed Vite watching makes bind-mounted host edits reload reliably. The first setup installs Apple's recommended Linux kernel if needed, then bootstraps Python, Git, Bubblewrap, pinned Rust 1.88.0, and lockfile-exact npm dependencies inside the persistent container. It does not use Docker or Compose.

Daily commands are:

```bash
./scripts/apple-container-dev status
./scripts/apple-container-dev start
./scripts/apple-container-dev restart
./scripts/apple-container-dev logs
./scripts/apple-container-dev stop
```

`start` and `restart` wait for a successful canonical Rust/Python bridge response, not only for an open HTTP port. Use `rebuild` only when you intentionally want to recreate the named container; the npm, Cargo, and runtime volumes remain preserved. The service is published only on the host loopback address.

The host-process fallback remains available:

```bash
cd mission-control
npm run dev
```

Open `http://127.0.0.1:1420`. Do not run this fallback while `alfredo-dev` owns the port. Both paths start a typed Rust bridge to the same authoritative Python Orchestrator used by Tauri, so repository selection, Mission choice, snapshots, and governed actions use canonical state rather than preview fixtures. By default, Alfredo's parent directory is the Starting Location, allowing a new sibling repository without overlapping the forbidden backend; set `ALFREDO_STARTING_LOCATION` explicitly when another projects directory is intended. The bridge is capability- and same-origin-guarded and exists only while its Vite process is running. Stop the host fallback with `Ctrl+C`.

For the Tauri development window without the managed launcher:

```bash
cd mission-control
npm run desktop
```

Native development uses a separate loopback Vite port (`1422`), so a browser workstation on `1420` no longer blocks the Tauri launcher.

## What Works

- Selection-required startup from a distinct Starting Location, with acknowledged existing/new Git repository selection and no fabricated Mission or Workspace Session.
- Explicit exact Resume Mission or distinct Start New Mission choice after workspace acknowledgement, with canonical workspace/Mission restoration across process and desktop restart.
- One durable Agent Console chronology for controller discussion, commands, skills, coding requests, proposals, approvals, and outcomes.
- `/help`, `/skills`, `/use`, `/run`, `/task`, and `/status`, plus natural-language coding-task routing.
- Governed automatic delegation only after an exact canonical Mission/scope/goal/path/policy/worker boundary check.
- Queued, cancellable, crash-recoverable Local Agent sessions in isolated worktrees with bounded iterative repair.
- Reload-safe canonical repair actions for Review Workspace, TUI, CLI, and Ad Hoc sessions, with inherited authority and exactly-once child launch.
- Persistent Mission Work cards, Issue Assignment, Workspace Queue, Review Workspace, Activity Journal, and Context Inspector.
- Timestamp-backed last activity, safe copyable recent-workspace relaunch commands, and bounded refresh continuity for transport-failed workstation outcomes.
- Minimal Bubblewrap process views, PID-namespace descendant supervision, resource/output caps, allowed-path enforcement, typed exact-boundary path-grant requests, mutation-coincident recovery markers, idempotent audit reconciliation, and whole-file evidence validation with bounded display memory.
- Real bounded review artifacts opened through an inline safe-text viewer rather than raw local-file navigation.
- Responsive two-lane desktop layout that stacks cleanly for tablet and phone widths.

## Verification

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests

cd mission-control
npm test -- --run
npm run test:gateway
npm run test:browser
npm run test:performance
npm run typecheck
npm run build
npm run test:layout
npm run release:verify
npm run release:check

cd src-tauri
cargo fmt --check
cargo test
```

The production Chromium layout gate passes 4/4 at 1440×900, 1100×760, 820×900, and 390×844. See the [current acceptance-correction report](.agent/Reports/2026-07-12-alfredo-install-queue-acceptance-correction.md) for exact release status; the [2026-07-11 implementation report](.agent/Reports/2026-07-11-alfredo-one-shot-workstation.md) is retained as superseded history.

Production performance evidence uses exact installed artifacts, clean committed
source, immutable fixtures, separately hashed correctness gates, and at least 30
process-cold or 100 process-warm randomized AB/BA pairs. See the
[performance cohort operator guide](mission-control/performance/README.md).
No current source-tree measurement is an accepted speed result.

## Current Model Roles

- `qwen3-14b` — default low-latency controller.
- `qwen3.6-27b` — selectable frontier/router.
- `gemma4-12b` and `gemma4-26b` — normal local workers.
- `qwen2.5-coder-14b` and `deepseek-r1-14b` — delegate-only escalation targets.

Controller routing and worker assignment are separate: a controller may classify or discuss a request, but only an eligible Local Agent role can execute a persisted session.

## Documentation

- [Documentation index](.agent/README.md)
- [Project architecture](.agent/System/project_architecture.md)
- [Development workflow](.agent/SOP/development_workflow.md)
- [API and command boundaries](.agent/System/api_endpoints.md)
- [Persistence schema](.agent/System/database_schema.md)
- [UX guidelines](.agent/System/ux_guidelines.md)
- [Active orchestration context](.agent/Tasks/context.md)
- [Domain terminology](CONTEXT.md)
- [Production performance cohort operator guide](mission-control/performance/README.md)
