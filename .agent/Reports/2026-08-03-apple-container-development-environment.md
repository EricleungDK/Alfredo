# Persistent Apple container development environment

**Requested:** 2026-08-03

**Implemented and verified:** 2026-08-03

**Status:** Running locally; not committed or pushed

## Goal

Keep Alfredo's functional browser workstation available at `http://127.0.0.1:1420` without requiring each coding agent to start a host Vite process or open a new browser for routine manual inspection. The Mission Commander uses Apple's `container` runtime rather than Docker.

## Environment

- Host runtime: Apple `container` CLI 1.2.0 on Apple silicon.
- Guest base: `docker.io/library/node:22-bookworm` (`v22.23.2` observed).
- Bootstrapped guest tools: Python 3.11, Git, Bubblewrap, build-essential, pinned Rust 1.88.0, and lockfile-exact npm dependencies.
- Named container: `alfredo-dev`, four CPUs, 4 GiB memory.
- Published boundary: guest 1420 to host `127.0.0.1:1420` only.

Apple's runtime required a one-time recommended Kata Linux kernel and VM init-image installation. A minimal Alpine container then passed, proving the runtime before Alfredo was created.

## Implementation

`scripts/apple-container-dev` owns `setup`, `start`, `restart`, `stop`, `status`, `logs`, `shell`, `test-layout`, and `rebuild`. It creates a persistent named container directly from the Node image, bind-mounts the source repository's parent at `/workspace`, enables polling-backed Vite watching for reliable host edits, and preserves four named volumes:

| Volume | Guest mount | Purpose |
|---|---|---|
| `alfredo-dev-node-modules` | `/workspace/Alfredo/mission-control/node_modules` | Keep Linux npm dependencies separate from host macOS dependencies |
| `alfredo-dev-cargo-target` | `/workspace/Alfredo/mission-control/src-tauri/target` | Reuse the Linux Rust bridge build across restarts |
| `alfredo-dev-toolchain` | `/var/lib/alfredo/toolchain` | Preserve pinned Rustup/Cargo tools and Playwright browser cache across container recreation |
| `alfredo-dev-runtime` | `/var/lib/alfredo/runtime` | Preserve canonical Alfredo development runtime state |

The named container's own root filesystem retains system packages and Rust across normal stop/start. `mission-control/dev/apple-container-entrypoint.sh` hashes `package-lock.json` and reruns `npm ci` only when the Linux dependency volume is missing or stale.

The helper's readiness check fetches Vite's per-process capability and sends a read-only, same-origin `alfredo_launch_context` request through `POST /__alfredo/invoke`. It reports ready only after the Rust bridge and authoritative Python Orchestrator respond successfully.

## Container transport boundary

`npm run dev:container` uses the explicit `apple-container` Vite mode and binds guest `0.0.0.0:1420`, which Apple port forwarding requires. The external browser origin remains exactly `http://127.0.0.1:1420`.

Port forwarding reaches Vite from the Apple VM network address, so the dedicated container mode cannot apply host mode's socket-loopback peer assertion. It retains the exact Host, Origin, immutable per-process capability, HTTP method, media type, body size, request grammar, command allowlist, and typed Rust/Python authority checks. The lifecycle helper fixes the host publisher to `127.0.0.1`, and ordinary host mode keeps the peer assertion. No production build receives this capability.

## BuildKit finding

Apple BuildKit 0.13.0 repeatedly stalled at `[resolver] fetching image` for `node:22-bookworm`, even with a minimal build context. `container run` fetched and ran that exact image immediately. The supported workflow therefore uses Apple's persistent `container create`/`start` path and bootstraps inside the named container. It does not depend on Docker, Compose, or the failing builder VM. The unused builder was stopped after diagnosis.

## Verification

- Apple service status: running with CLI/API server 1.2.0.
- Minimal Apple container: Alpine 3.22 completed successfully after kernel/init installation.
- Lifecycle scripts: `bash -n` passed.
- Gateway security and lifecycle regression: `npm run test:gateway` passed 17/17 tests, including container-forwarded peer acceptance with unchanged Origin/token rejection.
- TypeScript: `npm run typecheck` passed.
- Repository whitespace validation: `git diff --check` passed before documentation consolidation.
- Live container: `alfredo-dev` ran Linux arm64 with four CPUs and 4096 MB.
- Live canonical UI: Chromium rendered `AGENT CONSOLE / CODING WORKSPACE` with Starting Location `/workspace`, no unavailable screen, no page errors, and six `POST /__alfredo/invoke` responses at HTTP 200.
- Persistent restart: after the final toolchain-volume recreation, stop, start, and canonical bridge readiness completed in 2.66 seconds without reinstalling the toolchain or npm dependencies. Logs began directly at Vite startup, all four named volumes were present, polling variables were set, and the persisted toolchain reported `rustc 1.88.0`.

## Agent and operator workflow

One-time setup:

```bash
./scripts/apple-container-dev setup
```

Normal use:

```bash
./scripts/apple-container-dev status
./scripts/apple-container-dev start
./scripts/apple-container-dev restart
./scripts/apple-container-dev logs
./scripts/apple-container-dev test-layout
```

Open `http://127.0.0.1:1420` and leave the named container running for user-led visual testing. Future agents should use focused automated tests for implementation evidence but should not start a second host Vite or browser merely to provide the already-running manual preview.

`test-layout` exists for macOS environments where a host Playwright Chromium process cannot register its Mach bootstrap service. It reuses the running guest, caches Chromium in the named toolchain volume, reinstalls OS dependencies after a guest rebuild when necessary, and runs the production layout suite inside Linux. It does not restart or stop the canonical workstation.

## Related documentation

- [Development workflow](../SOP/development_workflow.md#persistent-apple-container-browser-ui-preferred-on-macos)
- [Development localhost bridge architecture](../System/project_architecture.md#development-localhost-bridge-boundary)
- [API and gateway boundary](../System/api_endpoints.md)
- [Functional localhost workstation diagnosis](2026-08-03-functional-localhost-workstation.md)
