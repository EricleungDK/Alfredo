# TUI side pane: missions + agents

Owner decisions (2026-09-29): task tree moves to a right-pane tab; missions and
agents are both shown; working indicator is a braille spinner.

Sources: `mission-control/src/prototypes/MissionExecutionTreePrototype.tsx`
(record icons: ticket, branch, robot; statuses; attention), 
`prototypes/alfredo-workstation-side-pane/` (side pane content, summary counters).

## Layout (width >= 88)

```
┌ missions ──────────────┐┌ right pane: chat | task | tasks tree ────┐
│● default               ││                                           │
│  go-8c1 · 1/2 · 00:12  ││                                           │
│· docs-cleanup          ││                                           │
│  idle                  ││                                           │
├ agents · 2 working · 1 review · 1 failed ──────────────────────────┤
│⠹ ◈ architect  planning ││                                           │
│    qwen3:14b     00:04 ││                                           │
│⠼ ◈ worker #2  writing  ││                                           │
│    qwen2.5-coder 00:09 ││                                           │
│◐ ◈ worker #1  review   ││                                           │
│✗ ◈ repair #3  check    ││                                           │
│○ ◈ chat 1     ready    ││                                           │
└────────────────────────┘└───────────────────────────────────────────┘
```

Side pane width 26-30. Missions section: up to 1/3 of pane height, min 3 rows,
scrolls. Agents section: rest. Two-line rows; second line dim.

## Record icons (prototype: ticket / branch / robot)

| Record | nerd | unicode (default) | ascii |
|---|---|---|---|
| Task (issue slice) | nf-md-ticket | ▤ | # |
| Delegation / repair branch | nf-dev-git_branch | ⑂ | Y |
| Agent session (robot) | nf-md-robot | ◈ | @ |

`--icons nerd|unicode|ascii`, env `ALFREDO_ICONS`. Every glyph must be one cell
wide (assert with unicode-width in tests). No emoji.

## Status column

| State | Glyph | Color | Motion |
|---|---|---|---|
| working (planning, writing, checking) | braille spinner ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ | lime/green | 100 ms/frame |
| queued for model | ◌ | dim | none |
| evidence ready / awaiting review | ◐ | cyan | none |
| decision needed (hold, approval) | ● | amber | none |
| failed | ✗ | red | none |
| blocked | ‖ | magenta | none |
| complete | ✓ | green | none |
| idle chat | ○ | dim | none |

`--no-motion` or env `ALFREDO_NO_MOTION=1`: spinner replaced by static ▶.
Spinner ticks must redraw only when at least one agent is working.

## Behavior

- Side pane persists in chat and task views.
- F6 focuses side pane; Up/Down moves; Tab switches missions/agents section;
  Enter on agent opens its live output (worker/architect) or conversation (chat);
  Enter on mission switches to it through the existing /workspace handoff rules;
  Esc returns focus to prompt.
- F2 cycles right pane: chat -> task detail -> task tree.
- Width < 88: pane collapses to one summary row; F6 opens it as overlay.
- Missions list reads other missions' autopilot state read-only; never takes
  their locks; unreadable state shows `?` and never blocks.
