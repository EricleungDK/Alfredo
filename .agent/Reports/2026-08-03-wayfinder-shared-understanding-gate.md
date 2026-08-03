# Wayfinder Shared Understanding Gate

**Implemented:** 2026-08-03

**Issue:** [GitHub #60](https://github.com/EricleungDK/Alfredo/issues/60)

**Parent PRD:** [GitHub #56](https://github.com/EricleungDK/Alfredo/issues/56)

**Baseline:** `f7b7a49` on `main`

## Outcome

Agent Console now passes first contacts through the canonical Python `WayfinderService` before controller routing. A new project or consequential change enters persisted Chart mode. A fresh reference to an existing Wayfinder map, ticket, or issue enters persisted Work-through. Later turns continue one active durable flow without nesting or reinvocation. Read-only explanation, status, review, diagnosis, and inspection remain outside automatic Chart.

The project-level Shared Understanding Gate is persisted with the active flow. It opens only by an explicit Mission Commander confirmation receipt or a visible structured acknowledgement containing destination, scope, constraints, and uncertainty. An acknowledgement ends the turn; opening the gate never auto-invokes a skill, creates a canonical planning artifact, delegates, or starts production work.

## Guarded authority

While the gate is pending, the Python authority rejects Mission Draft creation/confirmation, Ad Hoc Delegation proposal/approval, and Workstation session launch with the recoverable `shared-understanding-required` code before mutation. Conversation, read-only inspection, bounded research, Grilling, and throwaway prototypes are not blocked by this guard.

`wayfinder-state.json` is a schema-versioned project-runtime store. It retains at most one flow with its mode, originating Agent Console message identity, gate status, and opening receipt. Missing state means no active flow; malformed state fails as structured persistence rather than substituting controller memory.

## Projection

The Agent Console response contract now includes a typed Wayfinder projection. The persistent Python transport emits it, Tauri decodes it, and React renders a named status line for Chart or Work-through plus `Shared Understanding Gate <status>`. This is a routing/status presentation, never an action receipt or success claim.

## Verification

- Red-green Python coverage proves persisted Chart entry, existing-reference Work-through, active-flow continuation, explicit Commander opening, visible agent acknowledgement, pre-gate planning/delegation/session blocking (including existing Mission Draft mutation), structured CLI failure, and read-only outside-Chart behavior: **8 focused tests passed**.
- Warm persistent transport preserves the typed Chart projection: **1 focused test passed**.
- `python3 -m py_compile` over the changed Python implementation/tests: **passed**.
- Rust typed projection decode assertion: **passed**.
- TypeScript typecheck: **passed**.
- `git diff --check`: recorded before commit.

The full Python repository matrix was also run on the supplied Darwin host: **466 total — 264 passed, 32 failed, 170 errors, 8 skipped**. The remaining failures are the established host constraints—missing Linux Bubblewrap and Darwin `/var` to `/private/var` aliases. The four controller regressions affected by the initial read-only matcher were rerun after the correction: three pass; the remaining command-sandbox case stops only because Bubblewrap is unavailable. The focused Wayfinder tests pass.

The full Vitest matrix finishes **100 passed, 154 failed** across 254 tests. The new rendered assertion is present, but this host's Node `v25.2.1` plus the repository's Vitest/jsdom stack fails during setup at `window.localStorage.clear is not a function`, before either the new or an unchanged App test body runs. The full Rust matrix finishes **35 passed, 10 failed** across 45 tests, with the same Bubblewrap and Darwin path aliases. TypeScript therefore provides the local frontend contract evidence; the browser suite requires the repository's supported Node runtime.

## User-visible result

In the managed desktop Agent Console, send a new-project or consequential-change prompt to see `Wayfinder / Chart mode` and its gate state. Send a fresh Wayfinder ticket/map reference to enter `Wayfinder / Work-through`. Use the explicit confirmation phrase or a visible structured acknowledgement only after shared understanding is truly present; the route then opens without taking a follow-on action.
