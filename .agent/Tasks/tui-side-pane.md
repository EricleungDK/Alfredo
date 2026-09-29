# TUI side pane, agent view, spacing

Owner decisions (2026-09-29): option A. Missions + work tree with agent activity
inline; braille spinner; enter an agent to watch and instruct it; less cramped UI.

Sources: `mission-control/src/prototypes/MissionExecutionTreePrototype.tsx`
(record icons ticket/branch/robot, statuses, attention tags).

## Phase 1: layout and spacing

### Left pane (persistent in every view)

```
┌ missions ──────────────────┐┌ chat | task detail | agent view ─────┐
│ ● default    1/2   00:12   ││                                      │
│ · docs-cleanup     idle    ││                                      │
│                            ││                                      │
├ work  1/2 done ────────────┤│                                      │
│ ◈ ⠹ architect   planning   ││                                      │
│ ▾ textutil module      2   ││                                      │
│   ▤ ✓ #1 Create textutil   ││                                      │
│   ▤ ⠼ #2 Create tests      ││                                      │
│       check  qwen2.5  0:09 ││                                      │
│     ⑂ ✗ #3 Repair of #2    ││                                      │
│ ◈ ○ chat 1      ready      ││                                      │
└────────────────────────────┘└──────────────────────────────────────┘
```

- Width: clamp(28, 25% of terminal, 44). Below 88 columns: collapsed to one
  summary row; F6 opens it as overlay.
- One column of padding inside every pane. One blank row between sections.
- Running task rows get a dim second line: stage, model, elapsed.
- Architect requests and chats are rows with the agent (robot) icon.
- Completed groups collapse by default when another group is active.

### Record icons

| Record | nerd | unicode (default) | ascii |
|---|---|---|---|
| Task | nf-md-ticket | ▤ | # |
| Repair / delegation | nf-dev-git_branch | ⑂ | Y |
| Agent session | nf-md-robot | ◈ | @ |

`--icons nerd|unicode|ascii`, env `ALFREDO_ICONS`. One cell wide. No emoji.

### Status glyphs

| State | Glyph | Color |
|---|---|---|
| working | braille spinner ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏, 100 ms | lime |
| queued for model | ◌ | dim |
| awaiting review | ◐ | cyan |
| decision needed | ● | amber |
| failed | ✗ | red |
| blocked | ‖ | magenta |
| complete | ✓ | green |
| idle chat | ○ | dim |

`--no-motion` / `ALFREDO_NO_MOTION=1`: static ▶. Spinner redraws only while
something is working.

### Header: two rows maximum

```
 ALFREDO  default · Danish-Immigration-Assistant            ollama ✓ qwen2.5-coder warm
 Autopilot ✓ done   1/1   01:02   alfredo/go-1cdb9a81
```

- Row 1: mission, repository directory name (full path only in F4/`/workspace`),
  health right-aligned. Attention items appear here only when non-zero
  (`1 review`, `1 decision`, `dispatch on`). No `Work 0 local`, no
  `no pending review`, no `dispatch off`.
- Row 2 only while an autopilot run exists. No goal text; the goal is the group
  title in the tree.
- Fields separated by three spaces, not chains of ` · `.

### Detail pane: labeled sections, no instructions

```
 ✓ #1  Check README version
 Accepted · qwen2.5-coder:14b · 00:41

 Files    README.md
 Check    grep -q "Version 1.0" README.md

 Result   check passed · exit 0

 Diff
 ...
```

- Labels dim, values normal, aligned in a column. Blank row between sections.
- Group detail: goal (wrapped), progress, list of tasks. Nothing else.
- Remove from detail panes: key help (`Alt+← collapses`), `Select a task with
  ↑↓`, `This group has no task action target`, `Showing 1 / 1 tasks · 0 local
  workers · 0 start errors`, `Filter: all · /tasks QUERY`. Keys live in the
  footer and F1. Filter shows only while a filter is active.
- Bug: group title shows planner retry text (`… | The previous plan was rejected
  by validation: …`). Title must be the user's original goal only.
- Long text wraps at word boundaries with a hanging indent under its label.

## Phase 2: agent view (enter an agent, watch, instruct)

- F6 focus left pane, Up/Down, Enter on a task/agent row opens agent view in the
  right pane. Esc returns.
- Agent view is a transcript of that agent, newest at bottom, following the tail:
  instruction sent to the model (collapsed to summary, expandable), read-only
  references (names only), streamed answer as code per file, check command and
  output, outcome, each repair attempt as a new turn.
- Prompt title shows the target: `To worker #2 · Enter send · Esc back`.
- Sending text to an agent:

| Agent state | Effect |
|---|---|
| worker generating | steer: cancel the current generation, rerun the same task with the note added to its conversation. Not counted as a repair. |
| worker running check | note queued; applied after the check if it fails, discarded with notice if it passes |
| failed / rejected | repair with the note as reason, continuing the agent conversation |
| awaiting review | same as failed: repair with note (result not accepted) |
| accepted | follow-up task depending on it, same files/check policy |
| architect planning or draft | plan revision with the note |
| chat | normal chat message |

- A direct instruction is the owner's approval for the inherited file/check
  policy. It never widens files or changes the check. Risk/human-hold reviews
  still stop.
- Every instruction is recorded in the agent conversation and task activity.
- Autopilot keeps running; a steered task is not double-handled by autopilot.
