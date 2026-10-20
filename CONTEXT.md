# Context

## Terms

### Alfredo

Native Rust (ratatui) terminal, crate `alfredo-tui`, that orchestrates local Ollama coding agents on a Git repository. Everything runs locally. Only the Chat Agent may change the Coding Workspace or push, and only with Approval.

### Coding Workspace

The Git repository Alfredo was opened on. It needs at least one commit. Workers and Autopilot never modify its branch, index or working files; the Chat Agent may, with Approval (`docs/adr/0004-chat-agent-acts-on-the-coding-workspace.md`). State is stored outside it.

### Chat Agent

The conversation's model acting as a tool-calling agent on the Coding Workspace (main checkout). Reads run automatically; edits, shell and git writes need Approval; push always does. Hands large, multi-file or risky work to `/plan` or `/go`.

### Approval

The owner's per-call consent to a Chat Agent action, or a per-mission always-allow rule granting it ahead. Tiers: read auto; edit, shell, git write ask; push always asks and is never allowlistable. Shell runs in the Sandbox unless the owner approves a host escalation for that call. No skip-approval mode.

### State Directory

Where Alfredo keeps durable state (`~/.local/state/alfredo`, or `--state-dir`). It must be outside the Coding Workspace.

### Mission

A named line of work in one Coding Workspace (default `default`). Each mission restores its own conversations, drafts, tasks and autopilot state.

### Conversation

A named chat set inside a mission. Up to eight are restored per set, and only one terminal may hold a named conversation at a time.

### Compaction

Replacing a long Conversation's oldest turns with a short model-written summary note (automatic near the size limit, or `/compact`). The note is saved in place of those turns; the newest turns stay verbatim. `/clear` restarts a Conversation instead.

### Side Pane

The left pane listing the repository's missions and the Work Tree. F6 focuses it, and below 88 columns it becomes an overlay.

### Work Tree

The side pane's projection of the architect, plan groups, tasks, repairs and chats, each with a status glyph.

### Agent View

Transcript of one agent's turns: instruction, references, answer, check output, outcome and repairs. The prompt can send the agent an Owner Instruction while it is open.

### Owner Instruction

A note to an agent from the prompt (Enter in Agent View or `/tell ID TEXT`). It steers, queues, repairs or creates a follow-up depending on the agent's state, and never widens approved files or check.

### Local Agent

A local Ollama model acting in a role (architect, worker, repairer) under Alfredo's policy. Its output is evidence, not authority.

### Architect

The Local Agent role that turns a request into a plan of tasks with dependencies and checks. Plans are linted before they can be saved.

### Worker

The Local Agent role that produces file changes for one approved task inside an isolated Worktree.

### Task

The durable unit of governed work: files, an approved check, dependencies, a state and retained evidence. Stored in a versioned task schema.

### Plan

A draft set of tasks from `/plan`, revised with `/plan-revise` and saved with `/plan-save`. Acceptance criteria are explicit per task.

### Plan Group

Tasks from one saved plan, shown together with their goal and progress.

### Permit

The approved policy for a task: writable files and the exact check argv. `/permit` records it and `/approve` acknowledges it; it cannot be widened later without fresh approval.

### Check

The exact command, run in the Sandbox, whose pass or fail decides whether a worker's files are acceptable.

### Reference

A committed file a worker may read but never write, such as a test the check names.

### Worktree

A detached Git worktree from committed HEAD in which a worker runs. It is isolated from the Coding Workspace.

### Sandbox

The bubblewrap and prlimit boundary around worker and check execution, and around eval grading of model-written code.

### Execution Provider

Module (`execution.rs`) that runs a prepared host effect with bounded resources and returns a typed receipt.

### Evidence

The retained record of a run: files, diff, check result, digests, model identity and requested generation settings. Reviews bind to it by digest.

### Receipt

A saved, identified record of a command or decision. Receipt IDs and revisions appear in F3 evidence and F4 activity.

### Review

The decision on a task's evidence: accept, needs repair, reject, or held for human review. Criterion-level outcomes are recorded with `/review ID JSON`.

### Human Hold

A review that always waits for the owner, for example when risk is escalated.

### Repair

A linked follow-up run for a failed or rejected task, carrying the prior failure as its reason. Repairs are bounded by `--max-repairs`.

### Autopilot

The `/go GOAL` loop: plan, approve, dispatch, auto-accept passing tasks, auto-repair failures, then integrate. It restores paused after a restart and never replays work.

### Integration Branch

The local `alfredo/go-<id>` branch that autopilot composes accepted tasks onto. HEAD and working files stay untouched.

### Dispatch

Automatic starting of approved tasks, toggled with `/dispatch on|off`.

### Recovery

Reconciliation of a run interrupted at a recorded check boundary. `/recover` acknowledges valid evidence or records a Failed outcome without replay.

### Wayfinder

The scope-agreement flow for new projects and consequential changes: a draft of Destination, Scope, Constraints and Uncertainty, confirmed by revision. It needs no model inference.

### Scope Receipt

The saved confirmation of a Wayfinder scope revision. It grants no task approval or acceptance.

### Local Inference Profile

A versioned, qualified combination of local model identity and inference boundaries (context, output, sampling, residency, concurrency). Qualification rests on reviewed outcomes, not token speed.

### Inference Admission

Shared capacity control for one Ollama endpoint: queued turns wait for admission before loading or generating.

### Runtime Pin

The observed runtime identity (version, binary and configuration SHA-256) recorded with diagnostics. Hashes do not attest the upstream runtime, and no profile is promoted yet.

### Qualification Report

A bounded, repeatable record of fixture observations for one profile and runtime pin. Model output in it is never authority.

### Doctor

The noninteractive `--doctor` preflight of workspace, state, Ollama and sandbox prerequisites.

### Release Archive

The Linux x86-64 tarball built by `package_release.py` on a `v*` tag, published as a draft GitHub release.
