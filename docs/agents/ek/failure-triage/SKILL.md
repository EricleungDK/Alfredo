---
name: failure-triage
description: Classify a red test, check, or CI job before editing code to fix it. Use when a test, build, or CI job fails, or the user pastes a failure and asks for cause or plan.
---

A red result is a claim about the code only once ruled in. Classify each failure into one class, with **evidence**, before any code edit. Loaded once per session: on later failures, classify against this, without re-invoking.

## Classes

Check them in this order; the first that fits wins.

1. **sandbox/env**: EPERM, EROFS, "Operation not permitted", read-only `.git`, listen EPERM, docker.sock, DNS resolve failure, missing tool.
   → Leave code and config as is; the next action is the same command rerun once outside the sandbox (Claude Code: Bash with `dangerouslyDisableSandbox: true`, which asks the user; Codex: request escalation). Record "passed only unsandboxed". If the rerun is denied or impossible, log it as **verification debt** (finish skill). Missing tool: use the preflight fallback.
2. **stale**: the test expects old text or an old seam; a stale server holds the port; local main is behind origin; the CI SHA ≠ HEAD; task docs are out of date.
   → Follow [stale.md](stale.md).
3. **flaky**: passes and fails on the same code.
   → Follow [flaky.md](flaky.md).
4. **unowned**: the failure is in files outside your write scope, or changed by someone else (compare `git status` / `git diff --stat` with what you touched).
   → Report it with file and error; keep your diff to your scope.
5. **real bug**: none of the above holds.
   → Reproduce it in a failing test, then fix it red→green (TDD; diagnosing-bugs for hard ones).

## Evidence

One line per failure, carried into the finish report:

`<test/check> → <class>: <evidence: command, exit code, decisive output line>`

One green run proves nothing about a flaky or env class; the class's own procedure decides.
