# Development Workflow

**Last Updated**: 2026-07-23
**For**: Alfredo/Albert contributors

## Related Docs

- [Project Architecture](../System/project_architecture.md) — components, authority, and trust boundaries
- [API Endpoints](../System/api_endpoints.md) — Python/Tauri/React command contracts
- [Persistence Schema](../System/database_schema.md) — versioned JSON stores and locking
- [UX Guidelines](../System/ux_guidelines.md) — prompt-first layout and interaction rules
- [Active Orchestration Context](../Tasks/context.md) — current mission, ownership, blockers, and release state

## Setup

Alfredo is developed in Ubuntu. Python uses the standard library; the desktop application uses Node/npm, React, TypeScript, Tauri, and Rust. Ollama is optional for live local-model checks.

```bash
cd /path/to/local-coding-agent
cd mission-control
npm install
cd ..
python3 -m albert_mvp --help
```

For the native Tauri window, install the operating-system prerequisites documented by Tauri in addition to a current Rust toolchain. Browser development and all non-native frontend tests work without launching Tauri.

## Before Making Changes

1. Read `AGENTS.md`, `.agent/README.md`, and `.agent/Tasks/context.md`.
2. Refresh the `## Active Orchestration Context` block before planning if it is stale or incomplete.
3. Read the relevant files under `.agent/System/`, `.agent/SOP/`, and `.agent/Tasks/`.
4. If no active planning artifact exists, read the relevant GitHub `[PRD]` parent and its ordered Issue Slice sub-issues; use `.scratch/` only for migrated-history provenance.
5. Record implementation ownership, active delegations, blockers, and material decisions in `.agent/Tasks/context.md` as they change.
6. Preserve unrelated work already present in the worktree.

Python is authoritative for mission state and policy. Tauri transports typed data; React renders acknowledged projections. A UI implementation must not fabricate accepted work, launch state, evidence, or review outcomes.

## Development Loops

### Python orchestrator and CLI

```bash
python3 -m unittest discover -s tests
python3 -m albert_mvp --help
```

Use a temporary runtime root for manual commands so development checks do not mutate the normal workstation state.

```bash
python3 -m albert_mvp agents \
  --target-repo . \
  --tracker-dir .agent/issues \
  --runtime-root /tmp/alfredo-dev-runtime \
  --mission-id agent-issues \
  --agent-config .albert/agents.json
```

### Browser UI

```bash
cd mission-control
npm run dev
```

Vite prints the local URL. This is the quickest way to inspect the current GUI skeleton; backend-only actions require the desktop bridge or a test client.

### Tauri desktop UI

```bash
cd mission-control
npm run desktop
```

### Managed workstation launcher

From the repository root:

```bash
ALFREDO_RUNTIME_ROOT="$HOME/.alfredo/runtime" \
  node mission-control/bin/alfredo.js workstation --agent qwen3-14b
```

The launcher validates required tools, starts the persistent Python transport, and opens the desktop application. Startup no longer prewarms the selected controller.

### Focused frontend checks

```bash
cd mission-control
npm test -- --run src/App.test.tsx
npm run typecheck
```

Prefer a focused red/green loop while implementing, then run every release gate below.

## Release Gates

Run from the repository root unless a command changes directory:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests

cd mission-control
npm test -- --run
npm run typecheck
npm run build
npm run test:layout
npm run release:verify
npm run release:check

cd src-tauri
cargo fmt -- --check
cargo test
```

The Playwright layout gate builds the production bundle and checks real Chromium geometry at desktop, compact-desktop, tablet, and phone viewports. It must remain free of page-level horizontal overflow and control/panel overlap. Test discovery or a successful production build is not a substitute for an actual browser run; if the execution environment blocks Chromium startup, record the gate as unverified and rerun it in an allowed environment.

`npm run release:verify` is the local public-distribution gate. It builds a production AppImage, generates and exact-manifest-audits the minimal `alfredo-agent` meta package plus exact-version native adapter package, serves both from an isolated local npm registry, and installs **only** `alfredo-agent@<version>` into a clean global prefix. The gate asserts that npm fetched the optional platform tarball, resolves plain `alfredo` through PATH with no developer overrides, validates package/native versions and the AppImage SHA-256 manifest, and opens the installed application until the frontend and backend have returned the versioned `selection-required` launch context. That readiness marker records Starting Location with null Coding Workspace/Mission; it must not fabricate a Workspace Session snapshot. Only after every assertion passes does the gate stage those same two packed tarballs and replace `release/out/verified/` with their publish-order/digest manifest. The replacement fails closed but is not a concurrent-reader transaction; do not run a checker or publisher concurrently with release generation. The currently verified artifact baseline is Ubuntu 24.04 x64 with glibc 2.39; do not infer broader Ubuntu or glibc compatibility from this gate. Tauri's first AppImage build downloads its official Linux packaging helpers and may require an unrestricted Linux packaging environment.

`npm run release:check` reopens `release/out/verified/manifest.json` and fails closed unless the set declares production-AppImage verification. It verifies regular-file containment, exact names/version/order, byte counts, SHA-256 and npm SHA-512 integrity, contained package manifests, both CLI aliases, and the meta package's exact optional platform dependency. The tarballs and manifest are one same-job verification set, not an external signature: replacing both coherently is outside this local checker's trust boundary. Fixture output is deliberately non-publishable. Never repack `release/out/<package>/` or publish from those mutable staging directories after verification; any later test that rebuilds or cleans release output requires a fresh production `release:verify`.

The fast `tests/alfredo-entrypoint.test.js` gate uses a deterministic native fixture to test both package structure and the meta-only isolated-registry resolver without rebuilding the AppImage. It also proves that plain `alfredo` can open its desktop process when Ollama or the default model is absent. It cannot replace the production AppImage/GUI run in `release:verify`. Manual extraction, direct `node bin/alfredo.js`, and dry-run intent alone are never sufficient release evidence.

### Registry promotion

Publishing is an external maintainer action. As of 2026-07-13, local `npm whoami` returns `ENEEDAUTH`, and unauthenticated registry lookups return `E404` for both exact names; neither result proves name availability or publishing authority. npm provenance must be generated on a supported cloud-hosted CI runner, not by a local `npm publish --provenance` command. The manual `.github/workflows/publish-npm.yml` workflow is the authoritative promotion path:

1. Decide the provenance boundary. GitHub currently reports this repository as private, while npm provenance requires a public repository. Make it public only with explicit user authorization; otherwise obtain an explicit decision to bootstrap without provenance and change/review the workflow accordingly.
2. Review the complete diff, then commit and push the exact source revision only with user authorization. The provenance workflow fails closed unless it runs from `main` in a public repository; it also verifies exact SLSA v1 attestations with `npm audit signatures`, including when an exact-integrity version already exists after a partial run.
3. In GitHub, create a protected `npm-production` environment with required reviewer approval. For the first publication, add a granular npm automation token as the environment secret `NPM_TOKEN`; never paste the token into repository files, logs, or chat. The workflow also supports token-free OIDC after both packages have npm trusted publishers configured, so the long-lived secret need not remain.
4. Manually dispatch **Publish Alfredo npm release** with the exact version. The GitHub-hosted Ubuntu job runs the full Python/frontend/Rust matrix, `release:verify`, and `release:check`; publishes or safely reuses the exact verified platform version before publishing or reusing the exact meta version; then removes publish authentication. npm CLI tries trusted-publisher OIDC before falling back to `NPM_TOKEN`. An existing version is skipped only when its registry `dist.integrity` matches the verified manifest and its exact SLSA v1 provenance verifies cryptographically.
5. Let the same job install only `alfredo-agent@<version>` from `https://registry.npmjs.org/` into a new prefix, prove the PATH target and backend root stay inside that prefix, and require a frontend-plus-installed-backend marker under Xvfb. Then run `alfredo` once from a coding workspace on a real display and confirm the visible Alfredo window/title; the headless CI marker cannot replace this HITL check.
6. After the packages exist, configure npm trusted publishing separately for `alfredo-agent-linux-x64-gnu` and `alfredo-agent`, then remove `NPM_TOKEN` from the environment. Use GitHub owner `EricleungDK`, repository `Alfredo`, workflow filename `publish-npm.yml`, environment `npm-production`, and allow `npm publish`. A trusted publisher cannot bootstrap a package that does not yet exist.

Before requesting promotion, the local candidate may be regenerated and inspected without mutating npm:

```bash
cd mission-control
npm run release:verify
npm run release:check
npm publish release/out/verified/alfredo-agent-linux-x64-gnu-0.1.0.tgz --dry-run --access public
npm publish release/out/verified/alfredo-agent-0.1.0.tgz --dry-run --access public
```

If manual post-publication confirmation is needed, install from the registry—not local tarballs—and open a real window before updating ticket 20:

```bash
npm install --global --prefix /tmp/alfredo-registry-check alfredo-agent@0.1.0
cd /path/to/a/coding-workspace
PATH="/tmp/alfredo-registry-check/bin:$PATH" alfredo
```

Ticket 20's remaining release blocker is the repository-visibility/provenance decision, authorized hosted publication of both packages, registry-only install/PATH/headless-GUI verification, and the final human-visible window/title smoke. If any is missing, keep the ticket open.

Also run a launcher dry-run from the repository root:

```bash
ALFREDO_DESKTOP_DRY_RUN=1 \
ALFREDO_RUNTIME_ROOT=/tmp/alfredo-launcher-dry-run \
  node mission-control/bin/alfredo.js workstation --agent qwen3-14b
```

For documentation-bearing changes, run:

```bash
python3 /home/ericl/.codex/skills/documentation-consolidator/scripts/audit_documentation.py
python3 /home/ericl/.codex/skills/documentation-consolidator/scripts/validate_standards.py
```

Record exact final counts and any intentional optional skips in `.agent/Tasks/context.md` and the current implementation report. Do not claim a live-model path was verified unless it actually ran.

## Production performance evidence

Validate measurement code and fixture templates during ordinary development:

```bash
cd mission-control
npm run test:performance
npm run performance:fixtures
```

Do not generate or compare product latency from a dirty source tree, Vite,
jsdom, a reducer prototype, or the early GUI smoke marker. A production cohort
must first record all five correctness gates against the same clean commit,
fixture, and exact installed artifact, then pass `npm run performance:check`.
Run the sequential cohort only through `npm run performance:run`; its driver
enforces 30 process-cold and 100 process-warm pairs, preserves invalid samples,
and exits 2 when no speed claim is eligible. The complete plan and command
contract is in `mission-control/performance/README.md`.

## Implementation Rules

- Keep Python as the authority for scope, proposals, approvals, assignments, sessions, evidence, and review.
- Use typed contracts at the CLI, persistent transport, Tauri, and TypeScript boundaries.
- Make mutations correlation-idempotent and expected-revision guarded; retrying a lost response must not duplicate work.
- Keep model work deferred, cancellable, observable, resource-bounded, and independent of UI polling.
- Treat controller routing and worker assignment as different decisions. Controllers may classify/discuss; only eligible Local Agent roles may execute sessions.
- Qualify mutable work by Mission and entity identity. An Active Mission switch must not redirect already-bound work.
- Keep full Agent Console chronology durable while bounding only the Working Context assembled for a model turn.
- Store bulky output and diffs as session artifacts. Expose only registered, review-safe, workspace-contained text through the bounded artifact reader.
- Render failures inline while preserving the last acknowledged projection and offering a meaningful retry where safe.
- Use `Ubuntu Sans` for interface copy and `Ubuntu Mono` only for code-like content; keep flexible children shrink-safe and long values wrappable.
- Add regression tests at every changed boundary, including restart or replay behavior for persisted state.

## Git Workflow

Use conventional commits when the user asks for a commit:

```text
feat(console): route coding intent into governed delegation
fix(runtime): replay an acknowledged launch receipt
test(layout): cover compact desktop geometry
docs(workflow): refresh Alfredo release gates
```

Before committing, inspect `git status`, review the scoped diff, and preserve unrelated user changes. Do not commit, push, create a pull request, or delete branches unless the user asks.

## Debugging and Recovery

- Reproduce a failure at the narrowest public boundary before changing implementation.
- Inspect structured stderr/error codes rather than parsing display copy.
- Use a fresh `/tmp` runtime to distinguish corrupted local state from deterministic behavior.
- If a persistent transport request fails, verify the one-process CLI path with the same arguments.
- If the UI loses connection, preserve the last canonical state and reload a fresh snapshot; never infer completion from an interrupted request.
- If a runner owner dies, use the bounded canonical recovery/requeue flow. Do not edit runtime JSON manually.
- For a layout regression, add or tighten a rendered App assertion and the production Chromium geometry test.

**Document Owner**: Engineering Team
**Review Frequency**: whenever runtime contracts, launch flow, or release gates change
