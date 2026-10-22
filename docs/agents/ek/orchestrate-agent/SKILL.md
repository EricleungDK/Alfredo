---
name: orchestrate-agent
description: Orchestrate subagents on a task (free-form or GitHub issue): divide the work, delegate, verify acceptance before completing.
disable-model-invocation: true
---

You are the **orchestrator**. Subagents do the legwork; you hold the acceptance checklist, coordinate, verify, and own every human gate. Your context stays small: you keep conclusions, agents keep file dumps.

Input: a task. It may name GitHub issues (link or `#N`), or be free-form with no issue at all; both run the same flow.

## 1. Ask the model

First action, before any read, search or plan: ask the user which model the subagents use (AskUserQuestion: Opus, Sonnet, Haiku, Fable). Ask again every time this skill is invoked, never reuse an earlier answer. Pass it as `model` on every Agent call this run.

Done when: the user has picked a model.

## 2. Ground

- Sync the branch: `git fetch`, compare local with its remote, fast-forward. Confirm branch and HEAD with the user if the task names a branch you are not on.
- Handoff files and paths from another session (cloud `/tmp`, scratchpads) may not exist here. Verify each; when missing, recover intent from the repo's own task docs and say so.
- Read the repo's agent docs (`AGENTS.md`, `CLAUDE.md`, whatever they point to) for owner rules: TDD, commit style, attribution, doc upkeep.
- **Spec source**, one of:
  - Issues: `gh issue view <N> --comments` for each. Their acceptance criteria are the spec. Carry the issue link into every brief, commit context, and PR.
  - Free-form: the task text plus the repo's plan and task docs are the spec. Turn it into concrete, checkable outcomes yourself. Ask the user only about a gap that would change what gets built; fill everything else with the repo's conventions and state your assumptions in the checklist. Creating issues is the user's call, never a prerequisite.

Done when: you know the branch, HEAD, owner rules, and the spec.

## 3. Acceptance checklist

Write the checklist: each spec outcome (issue criterion or derived outcome), plus the repo gates (tests red-first, typecheck, the test suites the change touches, CI green, docs updated). Show it to the user before dispatching, together with the division of labor (step 4). Each item will later be ticked with **evidence** (a command and its result, a commit, a CI job), never with an agent's say-so.

Done when: the user has seen the checklist.

## 4. Divide and dispatch

**Division of labor** is yours, whatever the spec source:
- Map the change first (a read-only Explore agent when the area is unfamiliar): which files and modules each outcome touches.
- Cut units along file ownership: each unit owns a set of files no other unit edits, maps to named checklist items, and is verifiable on its own. Shared files (config, shared types, docs index) go to one owner unit, or to you after the units land.
- Order dependent units: a unit that needs another's output waits for it, or gets a stated interface to build against.
- One unit is the right answer for a small change; split only where parallel work or context size earns it.

Show the units (name, files owned, checklist items, depends on) with the checklist in step 3.

- One agent per unit; units touching disjoint files run in parallel, in one message. Run them in the background.
- Every brief follows [brief-template.md](brief-template.md). An agent starts with none of your context, so the brief is self-contained.
- **Subagents cannot spawn subagents.** A subagent whose flow (e.g. `/implement`) reaches a step that fans out, code review above all, hands that step to you through the **review relay** below. Research skills and workflows stay with you from the start.
- Fix loop: each failing unit gets at most **3 rounds** (initial fix plus two retries after CI or verification fails). After round 3, stop and escalate to the user with evidence.

Done when: every unit has a running agent.

### Review relay

Review stays inside the subagent's own implement flow; only the dispatch moves to you.

1. At its review step, the subagent commits its work and stops with a **review request** as its report: branch, base and head SHAs, checklist items covered, files changed, what to focus on. It stays alive, context intact.
2. You run the review for that session: `mp-code-review` (or `code-review`) scoped to base..head, reviewers on the model the user picked. The review is yours to run, never skip it because the agent's tests are green.
3. Send the findings back to that same agent with SendMessage (its ID, not a new agent): each finding with file, line, and failure scenario, plus which ones you judge real. Filter out findings you verified as false, and say why.
4. The agent fixes, re-verifies, and either finishes its flow or sends a new review request. Each review-fix cycle counts as one of its 3 rounds.

Several agents may request review at once: run each review separately, scoped to that agent's diff, and route findings back to the right agent.

## 5. Coordinate

- While an agent owns a branch, you do not push to it or edit its files. When you must (a change that affects its work), send the agent a message first: what changed, `git pull --ff-only` before its next commit.
- A push cancels in-flight CI runs under `cancel-in-progress`; an agent watching CI must hear about it.
- Parallel work of your own goes in a separate worktree and lands after, or coordinated with, the agent.
- Status line to the user at each milestone: agent started, agent committed, CI started, CI finished, escalation. When asked "is it running", answer with evidence: `git log`, uncommitted files, live processes, CI state.

## 6. Verify acceptance

When an agent reports, its report is a claim, in the finish skill's format. Verify it row by row:

1. Read its diff (`git show --stat`, then the diff of the core files). The fix addresses the root cause, and tests are not weakened to pass.
2. Rerun the decisive test yourself.
3. **Review**: every agent's work passed through the review relay. An agent that finished without a review request gets one now: run the review on its diff and send findings back to it.
4. CI per job: `gh pr checks`. Classify each failure with the failure-triage skill. CI specifics: pre-existing = reproduces on the parent commit; infrastructure = rerun once with `gh run rerun --failed`; expected by design = a release gate. Explain every non-green check to the user; never leave one unexplained.
5. Tick each checklist item with its evidence.

Done when: every checklist item is ticked with evidence, or escalated.

## 7. Human gates

Some steps belong to the user: owner approvals, merges, changes that relax a test or a security check, deploys and promotions. Also any action the permission classifier denies: stop, do not route around it through another tool or a later turn.

Hand each one over as a Needs-you block (finish skill, blocked branch), or the settings rule that would let you do it. A `!` command runs only when typed as the first character in the terminal on the PC; via Remote Control from a phone it arrives as text, so give the plain-shell form too.

After the user acts, verify the result (`git status`, a count, the test) before moving on. Edits the user makes by hand may miss a line; check the file compiles and tests pass.

## 8. Close

- PR: title, concise body, issue link (`Closes #N` when it completes the issue), test evidence, what is not done. Merge only with the user's explicit approval and every check green or explained.
- Update the repo's task and system docs per its rules.
- Final report to the user: the finish skill's report over the whole checklist, plus commits and PR.
