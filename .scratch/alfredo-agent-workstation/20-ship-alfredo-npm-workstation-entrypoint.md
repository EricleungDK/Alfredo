# Ship the Alfredo Npm Workstation Entrypoint

Status: ready-for-human
Type: HITL

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Make Alfredo installable and launchable through the public npm command path. Running `alfredo` opens the desktop workstation by default, `alfredo --agent <agent-id>` opens it with that controller or model selected, and `alfredo workstation --agent <agent-id>` provides the explicit rich UI command. Startup preflight should keep the product install location, selected coding workspace, backend runtime, desktop shell, Ollama availability, model availability, and writable runtime locations legible as separate checks.

## Acceptance criteria

- [ ] The published npm bin exposes `alfredo`, launches the Tauri/React workstation by default, and sets the desktop app title to Alfredo.
- [x] `alfredo --agent <agent-id>` and `alfredo workstation --agent <agent-id>` select a valid controller or model before the first prompt turn.
- [x] Startup preflight reports actionable, copyable failures for npm/runtime setup, desktop shell launch, backend process, workspace access, Ollama, required model availability, and writable runtime locations.
- [x] The selected coding workspace is never inferred from the Alfredo installation location, and recent workspaces remain available after relaunch.
- [x] A deprecated `albert` public alias remains available for one compatibility window if practical, without requiring internal package or `.albert` path renames.

## Blocked by

Mission Commander decision to make the currently private GitHub repository public for npm provenance or explicitly accept a non-provenanced bootstrap; authorized commit/push of the exact reviewed source; configuration of the protected `npm-production` environment and first-release `NPM_TOKEN`; a successful manual promotion that publishes platform before meta and passes a fresh registry-only install/PATH/headless-GUI smoke; then one human `alfredo` launch on a real display confirming the visible window and title.

## Comments

### 2026-07-12 — prior completion claim retracted

- Exact registry reproduction proved `npm install -g alfredo` installs an unrelated 2014 package with no `alfredo` bin. The prior test manually extracted a tarball and launched its JS file, so it did not prove npm installation, PATH linking, registry identity, or a production GUI.
- The corrected release seam generates `alfredo-agent` plus the exact-version `alfredo-agent-linux-x64-gnu` native adapter. A clean-prefix real npm install resolves `alfredo` from PATH directly to a 77,769,208-byte AppImage built on the current Ubuntu 24.04 x64/glibc 2.39 baseline, executes `Alfredo Desktop 0.1.0` without Cargo/Tauri on PATH, and opens the real Tauri/WebKit process tree.
- As of 2026-07-12, unauthenticated registry lookups return `E404` for both exact package names. That observation and the passing local publish dry-runs prove neither name availability nor publisher authority. The remaining blocker is authenticated publication of the native package first and the meta package second, followed by a fresh registry-only install/PATH/visible-window smoke.

### 2026-07-12 — meta-only and GUI-readiness hardening

- The release verifier now serves both tarballs from an isolated registry but installs only `alfredo-agent@0.1.0`; the deterministic gate proves npm fetched both the meta and optional platform tarballs, created the PATH command, and resolved bundled backend/native paths with no developer overrides.
- Plain `alfredo` no longer fails startup only because Ollama or the selected model is absent. Those checks are advisory until model work is requested; desktop, backend, workspace, and writable-runtime failures remain blocking.
- Native SHA-256, package/Cargo/Tauri version coherence, exact pack manifests, catalog containment, signal forwarding, and a frontend-loaded/backend-snapshot GUI marker now fail closed. Python 418 run (417 passed, one optional skip), frontend/distribution 225/225, Rust 37/37, typecheck, and the production frontend build pass.
- The production AppImage must be rebuilt once with the new marker and pass the full GUI verifier before publication. That packaging rerun was temporarily unavailable because the execution service reached its credit limit; it is not waived or replaced by the earlier binary hash.

### 2026-07-13 — production gate passed; authenticated promotion remains

- The rebuilt production gate initially exposed a real shipped-only failure: AppImage's `AppRun.wrapped` injected a bundle `PYTHONHOME`/`PYTHONPATH` even though no Python standard library was bundled. The native binary worked, but the inherited environment killed the host-Python backend before the GUI readiness marker. The Rust bridge now strips those two loader variables from both persistent and isolated backend child processes; its child-process regression and all 38 Rust tests pass.
- A fresh `npm run release:verify` then passed the complete local path: production AppImage build, exact package generation, meta-only isolated-registry install, one fetch of each tarball, plain `alfredo` from the clean prefix's PATH, frontend load, and installed-backend workspace snapshot readiness. Bubblewrap is now a separate non-blocking startup preflight: a missing runtime does not prevent the GUI from opening, but the warning explains that governed coding/shell execution requires it.
- The authoritative final local matrix passes Python 418 run / 417 passed / one optional skip, frontend 227/227, Rust 38/38, and responsive Chromium geometry 4/4. Assignment, Queue, and Mission Draft projections are all zero. The staged artifact gate reports status pass and publishable true; its installed AppImage is 77,761,016 bytes with SHA-256 `3faec58bc4e4a0b1c825cb58a3ec5475e5daac36bb0c839e0699ae6ddf006be2`.
- The verifier stages and replaces `release/out/verified` with the exact audited tarballs. `npm run release:check` rejects fixtures and any tarball change relative to that same-job manifest, plus identity, bin/dependency, and publish-order drift; the co-located manifest is not an external signature. The preserved platform tarball is `f7bb312dc35463c093fd9a02619297b5a949483dc1cb194c3b02ed94b77ebe18`; the meta tarball is `b0a19555ae0e848a729d5415cb3dcdbed7d3ddc343b90de8b592f8ce548f0313`. Both exact artifacts pass `npm publish --dry-run --access public`.
- `.github/workflows/publish-npm.yml` is a manual, protected, GitHub-hosted promotion path with OIDC provenance. It accepts the first-release `NPM_TOKEN` bootstrap or token-free npm trusted publishing once configured, re-runs every release gate, verifies exact registry integrity plus cryptographic SLSA v1 provenance on both fresh and partial-retry paths, requires the platform attestation before any meta publication, installs only `alfredo-agent@0.1.0` from the public registry into a fresh prefix, invokes `alfredo --version`, and requires an actual frontend-plus-backend GUI marker. It has not run: no npm credential, commit/push authority, or registry mutation was inferred.
- GitHub reports `EricleungDK/Alfredo` as private. npm provenance requires public source, so the provenance workflow now fails closed unless it runs from reviewed `main` in a public repository. Xvfb proves the shipped frontend/backend process boundary but cannot prove a human-visible window; the final real-display title/window check remains HITL.
