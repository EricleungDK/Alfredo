# Build the Prompt Dominant Workstation Shell

Status: complete
Type: AFK

## Parent

`.agent/issues/19-alfredo-agent-workstation-prd.md`

## What to build

Replace the superseded three-zone Mission Control framing with Alfredo's prompt-dominant two-zone workstation. The main pane is the durable prompt transcript with a pinned composer and compact status line; the secondary persistent pane is Agent Workstations. The Mission Commander should be able to read prior turns, keep steering from the bottom composer, and always see the selected controller/model and active execution state near the prompt.

## Acceptance criteria

- [x] The desktop workstation opens to a two-zone layout with the prompt pane as the dominant working surface and Agent Workstations as a persistent side pane.
- [x] The prompt transcript renders user prompts, assistant/controller responses, consequential workstation actions, and meaningful outcomes without mixing in raw telemetry.
- [x] The active prompt composer remains pinned at the bottom while prior turns scroll independently.
- [x] A compact status line near the composer shows connection, selected controller/model, Conversation Scope, workspace, and active execution state.
- [x] Constrained-width behavior keeps the prompt, composer, critical status, and side-pane decisions reachable without overlapping content.

## Blocked by

None for this source/layout slice. Public package installation remains tracked by `20-ship-alfredo-npm-workstation-entrypoint.md`.

## Comments

### 2026-07-12 — implementation complete; browser geometry rerun pending

- The full 215-test frontend suite, TypeScript, production build, style checks, and rendered constrained-width interaction coverage pass.
- The expanded Playwright file discovers four cases and exercises the palette, enabled evidence acceptance, expanded operational detail, and artifact viewer at every viewport.
- Actual Chromium startup was blocked before any geometry assertion by `sandbox_host ... Operation not permitted`; the required unsandboxed rerun was refused because the approval service had exhausted its usage quota. Keep the final constrained-width criterion and ticket open until `npm run test:layout` genuinely passes 4/4.

### 2026-07-13 — constrained layout accepted

- An unrestricted Chromium run reached all four geometry cases. Desktop, compact desktop, and mobile passed immediately; tablet correctly failed with 84 px of horizontal page overflow and the Send control outside the 820 px viewport.
- Diagnostic geometry traced the failure to implicit `auto` columns in the prompt pane and nested composer dock. A long nowrap status value established a 890 px intrinsic track. Both grids now use `minmax(0, 1fr)`, allowing long status text to shrink/ellipsis while the composer action remains in view.
- `npm run test:layout` now passes 4/4 at 1440×900, 1100×760, 820×900, and 390×844 with page overflow, panel/control overlap, and critical-control containment assertions executed.
