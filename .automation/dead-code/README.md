# Alfredo dead-code automation state

This directory lives on the dedicated `automation-state` branch and is the machine-readable state surface for the daily conservative dead-code cleanup governed by issue #79.

## Sources of truth

- Policy and safety contract: issue #79 body.
- Current pull-request state: live GitHub PR data only. Never infer open/closed/merged state from issue text or historical comments.
- Current workflow state: `.automation/dead-code/state.json` on `automation-state`.
- Proven live/compatibility exclusions: `.automation/dead-code/known-live.json`.
- Per-run evidence: one GitHub issue titled `[Dead-code run] YYYY-MM-DD`.
- Historical machine records and usage: `.automation/dead-code/runs/YYYY-MM-DD.json` on this branch when available.
- Legacy v1 comment bodies on issue #79 were intentionally cleared on 2026-09-10 after the v2 migration. The #79 comment timeline is not an operational state source.

## State rules

`active_run` is either `null` or an object containing the current run ID, run issue URL/number, status and timestamps. At most one active run is allowed. A run is terminal after `published`, `superseded`, `rejected`, `no_change`, or `verification_failed` is established. Terminal runs clear `active_run` and update `last_terminal_run`.

Before starting a new run, the timer must query live GitHub for currently open PRs whose title begins `chore: remove proven dead code`. Closed or merged PRs never block a new run. It then reads the active run, if any, and performs a cheap orchestration preflight before deciding that the run blocks another day.

A valid pending record younger than 24 hours may block a new run while publication is in progress. It must not block if its schema/checksum is malformed or if its patch is structurally outside policy. In particular, reject pending changes that touch tests (`tests/`, test directories, `*.test.*`, `*.spec.*`), fixtures, generated/vendored code, migrations, or policy/orchestration state; reject binary/rename/mode changes, oversized scope, and additions that are not merely a syntactic contraction of an existing line. Timer rejection is orchestration-only: never apply the patch.

Any unresolved active run older than 24 hours is stale. Terminalize it as `superseded` with classification `stale_run_timeout`, clear `active_run`, close the run issue, and allow the current day's fresh analysis to proceed. Never publish or reuse a stale patch. The timer must also search for an existing run issue for today's Europe/Copenhagen date to prevent duplicates.

## Candidate exclusions

Read `known-live.json` before choosing a cleanup candidate. Entries are not blind permanent suppressions: perform the cheap invalidation/reference check described by each entry. While the documented observable contract still exists, do not spend a full baseline/candidate verification cycle trying to remove that binding again. If the contract disappears or relevant code changes materially, re-evaluate the entry.

## Outcome semantics

Use terminal statuses consistently:

- `no_change`: analysis/verification completed sufficiently to make a safe decision, but there is no eligible cleanup to publish. This includes: no candidate found; all candidates are proven live; a linter candidate is actually a compatibility/public/re-export surface; or a candidate is safely rejected by focused checks proving it changes behavior.
- `verification_failed`: the workflow cannot establish a trustworthy safety conclusion because verification itself is incomplete, unavailable, incomparable, ambiguous, newly/worsening flaky beyond the allowed paired retries, missing a required baseline, or otherwise cannot prove whether the candidate is safe.

A candidate-only test failure that clearly proves an observable compatibility regression is successful evidence that the candidate must stay. Revert it and return `no_change`; do not call that run `verification_failed`.

## Per-run issue

Each day that passes the prechecks gets one issue named `[Dead-code run] YYYY-MM-DD`. The issue contains the Cloud trigger, Cloud evidence/pending record, local publication result and usage metadata for that run. Close the run issue once terminal. Do not append routine run records to issue #79.

New records use v2 markers so legacy #79 comments cannot be mistaken for live queue state:

- `<!-- alfredo-run-v2 -->`
- `<!-- alfredo-pending-v2 -->`
- `<!-- alfredo-outcome-v2 -->`
- `<!-- alfredo-publication-v2 -->`

## Usage accounting

Track the timer, Cloud producer and local publisher separately. Use exact values only from authoritative runtime/account metadata. Never back-calculate tokens from text length or translate tokens into subscription-quota percentage without the platform's actual allowance denominator.

Use these source labels rather than a generic `unavailable` when possible:

- `not_exposed_to_automation`: the ChatGPT scheduled-task runtime did not expose its own token/credit metadata.
- `not_exposed_to_github_connector`: the Codex Cloud task completed, but token/credit metadata was not included in the connector-delivered GitHub result. Store the task/thread URL when available for later reconciliation.
- `not_run`: that stage did not execute, for example the local publisher on a no-change run.
- `available_only_in_chatgpt_or_codex_usage_ui`: account allowance/credit/reset information exists only in the user's authenticated OpenAI usage surface and was not available to the automation.
- `runtime_metadata`: exact values were exposed directly to the executing runtime.
- `usage_ui_reconciled`: exact values were later copied from the authenticated ChatGPT/Codex usage UI and associated with this run.

For each run, preserve timer/cloud/local model, input tokens, cached input tokens, output tokens, credits and source when available. Preserve account quota used/remaining/unit/reset only when authoritative values are available.

OpenAI usage UI data and Codex thread-level usage are authoritative sources when exposed to the user, but they are not currently passed automatically into this GitHub connector workflow. Therefore missing GitHub-side usage must be recorded explicitly, not estimated. A later reconciliation may update a terminal run record without changing its cleanup outcome.
