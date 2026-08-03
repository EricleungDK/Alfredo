# Wayfinder Shared Understanding Gate

**Implemented:** 2026-08-03

**Issue:** [GitHub #60](https://github.com/EricleungDK/Alfredo/issues/60)

**Parent PRD:** [GitHub #56](https://github.com/EricleungDK/Alfredo/issues/56)

**Baseline:** `f7b7a49` on `main`

## Outcome

Agent Console now passes first contacts through the canonical Python `WayfinderService` before controller routing. A new project or consequential change enters persisted Chart mode. A fresh reference to an existing Wayfinder map, ticket, or issue enters persisted Work-through. Later turns continue one active durable flow without nesting or reinvocation. Read-only explanation, status, review, diagnosis, and inspection remain outside automatic Chart.

The project-level Shared Understanding Gate is persisted with the active flow. It opens only by an explicit Mission Commander confirmation receipt or a visible structured acknowledgement containing destination, scope, constraints, and uncertainty. An acknowledgement ends the turn; opening the gate never auto-invokes a skill, creates a canonical planning artifact, delegates, or starts production work.

## Guarded authority

While the gate is pending, the Python authority rejects every Mission Draft mutation, Ad Hoc Delegation proposal/approval, legacy delegation route/approval, and production launch through Workstation, direct CLI, TUI, or headless worker execution with the recoverable `shared-understanding-required` code before mutation. Conversation, read-only inspection, bounded research, Grilling, and throwaway prototypes continue through their ordinary command/controller paths and retain the active Wayfinder projection.

`wayfinder-state.json` is a schema-versioned repository-runtime store under `runtime_root/wayfinder/<repository-hash>/`, independent of Mission runtime. It retains at most one flow with its mode, originating Agent Console message identity, gate status, and opening receipt across every Mission for that repository. One shared typed loader validates it at response and production-launch boundaries. Missing state means no active flow; malformed state returns structured `persistence-read-failure` and fails closed before production launch rather than substituting controller memory.

## Projection

The Agent Console response contract now includes a typed Wayfinder projection. The persistent Python transport emits it, Tauri decodes it, and React renders a named status line for Chart or Work-through plus `Shared Understanding Gate <status>`. This is a routing/status presentation, never an action receipt or success claim.

## Verification

- Red-green Python coverage proves persisted Chart entry, existing-reference Work-through, active-flow continuation, explicit Commander opening, visible agent acknowledgement, pre-gate planning/delegation/session blocking (including existing Mission Draft mutation, direct CLI, another Mission’s direct production launch, legacy delegation, and headless execution), safe discovery command availability, malformed-state structured failure (including a missing `active_flow` key), and read-only outside-Chart behavior: **12 focused tests passed**.
- Warm persistent transport preserves the typed Chart projection: **1 focused test passed**.
- `python3 -m py_compile` over the changed Python implementation/tests: **passed**.
- Rust typed projection decode assertion: **passed**.
- TypeScript typecheck: **passed**.
- `git diff --check`: recorded before commit.

The full Python repository matrix was also run on the supplied Darwin host: **466 total — 264 passed, 32 failed, 170 errors, 8 skipped**. The remaining failures are the established host constraints—missing Linux Bubblewrap and Darwin `/var` to `/private/var` aliases. The four controller regressions affected by the initial read-only matcher were rerun after the correction: three pass; the remaining command-sandbox case stops only because Bubblewrap is unavailable. The focused Wayfinder tests pass.

The full Vitest matrix finishes **100 passed, 154 failed** across 254 tests. The new rendered assertion is present, but this host's Node `v25.2.1` plus the repository's Vitest/jsdom stack fails during setup at `window.localStorage.clear is not a function`, before either the new or an unchanged App test body runs. The full Rust matrix finishes **35 passed, 10 failed** across 45 tests, with the same Bubblewrap and Darwin path aliases. TypeScript therefore provides the local frontend contract evidence; the browser suite requires the repository's supported Node runtime.

Independent closure review of `f7b7a49...c4d39d6` is clean on both axes. Standards found no remaining documented-standard violation or baseline smell after the shared state-loader correction. Spec found all five Issue #60 acceptance criteria satisfied, including repository-scoped state, safe pending-gate availability, legacy/headless guard coverage, and acknowledgement-only gate opening.

## User-visible result

In the managed desktop Agent Console, send a new-project or consequential-change prompt to see `Wayfinder / Chart mode` and its gate state. Send a fresh Wayfinder ticket/map reference to enter `Wayfinder / Work-through`. Use the explicit confirmation phrase or a visible structured acknowledgement only after shared understanding is truly present; the route then opens without taking a follow-on action.
