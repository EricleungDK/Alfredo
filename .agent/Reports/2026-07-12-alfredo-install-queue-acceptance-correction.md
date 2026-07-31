# Alfredo Install and Queue Acceptance Correction

**Date reported:** 2026-07-12  
**Date locally fixed:** 2026-07-12  
**Production gate fixed:** 2026-07-13  
**Severity:** Critical product acceptance failure  
**Status:** Verified local release candidate; public registry promotion pending  
**Reporter:** Mission Commander

## Summary

The prior completion claim was wrong on both user-visible outcomes. `npm install -g alfredo` installed an unrelated registry package and exposed no `alfredo` command. The locally packed project artifact contained source and launched `tauri dev`, so even its private path was a developer checkout rather than a consumer desktop release. Separately, an empty Queue view still stacked the 27-row historical/manual Issue Assignment Board, Mission Draft creation, Ad Hoc Delegation creation, and governance decisions into one surface.

The corrected local release candidate generates a small public CLI/backend package (`alfredo-agent`) and an exact-version AppImage package (`alfredo-agent-linux-x64-gnu`). The currently verified native artifact baseline is Ubuntu 24.04 x64 with glibc 2.39; broader Ubuntu or glibc compatibility is not claimed. A real clean-prefix npm install creates the `alfredo` PATH command and resolves directly to the installed native adapter. Queue is now a decision-only lower-pane replacement. With the final AFK layout slice complete, current Queue, pending Mission Draft, and assignment projections are all empty.

## Exact Reproduction

### Registry and launch

```bash
npm install --global --prefix /tmp/alfredo-prefix alfredo
PATH="/tmp/alfredo-prefix/bin:$PATH" alfredo
```

The registry installed unrelated `alfredo@0.2.0` from 2014. It had no npm `bin`; `alfredo` exited 127. The project README simultaneously admitted that its package was not published. Its prior release test manually ran `tar -xzf` and invoked `node <package>/bin/alfredo.js`, bypassing npm installation and PATH linking. Its launch plan was `npm run desktop`, whose script was `tauri dev`.

### Queue and assignment projection

A clean runtime using the launcher-selected `.agent/issues` tracker returned zero canonical Queue items but 27 Issue Assignment rows. The projection included 24 completed historical slices plus manual HITL `ISS-14` and `ISS-28`; the empty Queue still rendered both standing creation workflows.

```text
RED queue=0 expected=ISS-22 rows=27 manual=ISS-14,ISS-28 historical=24
```

## Root Causes

1. The unscoped npm name was assumed available without querying the registry.
2. Release tests asserted source-layout/dry-run intent instead of performing `npm install -g`, PATH resolution, and native execution.
3. The public artifact bundled build dependencies and Rust/Tauri source but no production desktop binary.
4. Markdown metadata parsing stopped at the blank line after the H1, silently defaulting every ticket to `ready-for-agent` / `AFK`.
5. The assignment projection mapped every ordered issue and the Queue detail stacked under the assignment board.
6. Manual Mission Draft and Ad Hoc proposal forms survived the console-first redesign as standing Queue administration.

## Fix

- `mission-control/package.json` is now a private development workspace.
- `scripts/build-desktop-release.js` builds the production AppImage in a Linux-writable Cargo target, and `scripts/build-npm-release.js` emits two minimal publish directories from committed source plus the built artifact.
- The meta package owns CLI grammar and the bundled Python backend. The platform package owns only the AppImage and a typed `desktop.json` adapter manifest.
- The launcher resolves the exact host package/version, rejects missing, mismatched, escaping, symlinked, or non-executable adapters, and directly spawns the AppImage. Its extraction fallback is internal; no Cargo, Vite, Tauri CLI, source checkout, or user AppImage flag appears in the consumer command.
- The adapter manifest carries the AppImage SHA-256 and the launcher dynamically probes the installed native version before startup. Ollama/model readiness is advisory, so a fresh user can open the workstation before configuring local models.
- The release verifier serves both tarballs from an isolated registry but asks npm to install only the meta package. It then invokes plain `alfredo` and requires a GUI marker written only after the frontend calls a successful backend workspace snapshot.
- Verification realpath-confines both resolved packages to the fresh prefix, counts only tarball body fetches, matches the GUI marker's backend root to the installed bundle, and uses bounded process-group termination so failed window smokes cannot leave AppImage/WebKit descendants behind.
- `_metadata()` reads Status/Type metadata through the pre-section header block; snapshot contracts expose `work_type`.
- Issue Assignment shows active AFK/recoverable session work only. Completed/merged history and ready-for-human/HITL work are excluded.
- Opening Queue replaces the lower assignment board. It shows pending governance and pending Mission Draft decisions only, removes standing creation forms, and renders `No decisions pending` only after both authoritative projections load.

## Verification

### Release seam

`npm run release:verify` now performs a production AppImage build, exact-manifest-audits both generated packages, serves them from an isolated local registry, installs only `alfredo-agent@0.1.0` into a clean prefix, invokes plain `alfredo` only through that prefix's PATH, dynamically checks native version/digest without Cargo/Tauri on PATH, and waits for frontend-plus-backend GUI readiness before closing the application.

The same meta-only registry path passes with a deterministic executable fixture and proves npm fetched both tarballs:

```json
{
  "status": "pass",
  "install_spec": "alfredo-agent@0.1.0",
  "install_source": "isolated local registry with test fixture",
  "command": "alfredo",
  "invocation": "alfredo",
  "package_version": "0.1.0",
  "native_package": "alfredo-agent-linux-x64-gnu",
  "native_version": "Alfredo Desktop 0.1.0",
  "registry_tarballs_fetched": {
    "alfredo-agent": 1,
    "alfredo-agent-linux-x64-gnu": 1
  },
  "gui_smoke": { "status": "not_run_fixture" }
}
```

The required production rebuild initially failed correctly: the installed AppImage exited before writing readiness even though the freshly built native binary stayed alive and returned a snapshot. Boundary comparison narrowed the difference to AppImage's `AppRun.wrapped`, which unconditionally exported a bundle `PYTHONHOME` and `PYTHONPATH` without bundling a Python standard library. Host Python then failed before importing `encodings`, so the persistent backend closed its response stream.

The Rust bridge now removes those two AppImage loader variables from both persistent and isolated backend child commands. A child-process regression deliberately poisons the parent environment and proves a real backend snapshot succeeds without racing process-global environment mutation. The focused test was observed red at the snapshot boundary before the fix and green afterward; the full Rust suite passes 38 tests.

A fresh production `npm run release:verify` then passed the complete local release path: production AppImage build, exact package audit, meta-only isolated-registry install, one fetch of each package tarball, plain `alfredo` through the clean prefix's PATH, native version/digest validation, frontend load, and installed-backend workspace snapshot readiness. The authoritative native result is 77,761,016 bytes with SHA-256 `3faec58bc4e4a0b1c825cb58a3ec5475e5daac36bb0c839e0699ae6ddf006be2`. The verifier stages the two tarballs from that passing run, then replaces `release/out/verified/` with them and a typed publish manifest. `npm run release:check` validates the resulting same-job set's order, containment, sizes, SHA-256/SHA-512 integrity, contained package identities, aliases, and exact meta-to-platform dependency; its co-located manifest is not an external signature or a concurrent-reader transaction. The final platform and meta tarball SHA-256 values are respectively `f7bb312dc35463c093fd9a02619297b5a949483dc1cb194c3b02ed94b77ebe18` and `b0a19555ae0e848a729d5415cb3dcdbed7d3ddc343b90de8b592f8ce548f0313`. Both exact tarballs pass `npm publish --dry-run --access public`.

### Queue seam

```text
GREEN queue=0 drafts=0 assignment_rows=0 manual=0 historical=0 ready_issue_ids=[]
```

- Metadata regression: passed.
- Assignment projection: 23/23 passed.
- Rendered App Queue/assignment coverage: 121/121 passed, including combined-source loading semantics.
- Full frontend suite: 227/227 passed across 10 files.
- Full Python suite: 418 run; 417 passed and one optional Ollama smoke skipped.
- Full Rust bridge suite: 38/38 passed.
- Native adapter, package builder, installed-package, and meta-only registry focused suite: 44/44 passed.
- Production Chromium layout suite: 4/4 passed at desktop, compact desktop, tablet, and mobile. Its first unrestricted run failed tablet with 84 px of horizontal overflow and an off-viewport Send control; the prompt/composer grid-track correction then passed every geometry assertion.

## Remaining External Gate

As of 2026-07-13, local `npm whoami` returns `ENEEDAUTH`, unauthenticated registry lookups return `E404` for both exact package names, and both exact verified tarballs pass local publish dry-runs. None proves that npm will allow the names or that the maintainer has publishing authority. Local `npm publish --provenance` is also not an acceptable substitute because npm provenance requires a supported cloud-hosted CI environment.

The manual `.github/workflows/publish-npm.yml` job now encodes the remaining protected path: it restricts promotion to public `main`, checks out one exact revision, runs the full release matrix, regenerates and integrity-checks the publish inputs, accepts a first-release token or configured npm trusted-publisher OIDC, safely resumes a partial publish only when existing registry integrity matches and `npm audit signatures` cryptographically verifies the exact SLSA v1 provenance, publishes platform before meta, clears publish authentication, installs only the meta package from the public registry into a fresh prefix, and launches plain `alfredo` until the frontend/backend marker appears. The platform attestation must verify before any meta publication; the meta attestation is checked afterward. GitHub reports the source repository as private, so this provenance path correctly fails closed until the Mission Commander decides whether to make it public; dropping provenance for a private bootstrap is a separate explicit policy choice.

Ticket 20 and the product goal remain open until an authorized maintainer:

1. authorizes commit/push of the reviewed source revision;
2. decides public-repository provenance versus an explicitly non-provenanced private bootstrap;
3. configures the protected GitHub `npm-production` environment and first-release `NPM_TOKEN`;
4. approves and successfully runs the reviewed manual promotion path;
5. confirms its registry-only install/PATH/frontend/backend headless-GUI smoke;
6. types `alfredo` on a real display and confirms the human-visible Alfredo window/title.

## Prevention

- A registry lookup or local publish dry-run cannot prove name availability or publishing authority. A release claim requires authenticated publication followed by a registry-only package-manager install, PATH invocation, and shipped-native execution—not manual extraction or a launch-plan assertion alone.
- Development and public manifests remain separate so build tools cannot silently become consumer dependencies.
- Platform packages publish before the meta package, and exact versions are validated at runtime.
- Publish only the tarballs retained by the passing production verifier. Repacking mutable staging directories after audit breaks artifact identity.
- Strip AppImage's bundle-only Python loader variables at the backend child-process boundary whenever host Python supplies the standard library.
- Queue acceptance tests start from zero actionable items and assert absence of standing creation workflows and unrelated assignment/history content.

## Traceability

- Planning/acceptance: [Issue 20](../issues/20-ship-alfredo-npm-workstation-entrypoint.md), [Issue 22](../issues/22-build-prompt-dominant-workstation-shell.md)
- Current state: [Active orchestration context](../Tasks/context.md)
- Architecture: [Project architecture](../System/project_architecture.md)
- Procedure: [Development workflow](../SOP/development_workflow.md)
- Superseded evidence: [2026-07-11 workstation report](2026-07-11-alfredo-one-shot-workstation.md)
