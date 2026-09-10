# Alfredo dead-code automation state

This directory lives on the dedicated `automation-state` branch and is the machine-readable state surface for the daily conservative dead-code cleanup governed by issue #79.

## Sources of truth

- Policy and safety contract: issue #79 body.
- Current pull-request state: live GitHub PR data only. Never infer open/closed/merged state from issue text or historical comments.
- Current workflow state: `.automation/dead-code/state.json` on `automation-state`.
- Per-run evidence: one GitHub issue titled `[Dead-code run] YYYY-MM-DD`.
- Historical machine records and usage: `.automation/dead-code/runs/YYYY-MM-DD.json` on this branch when available.
- Issue #79 comments dated on or before 2026-09-10 are legacy evidence only and must never be parsed as the current queue.

## State rules

`active_run` is either `null` or an object containing the current run ID, run issue URL/number, status and timestamps. At most one active run is allowed. A run is terminal after `published`, `superseded`, `rejected`, `no_change`, or `verification_failed` is established. Terminal runs clear `active_run` and update `last_terminal_run`.

Before starting a new run, the timer must query live GitHub for currently open PRs whose title begins `chore: remove proven dead code`. Closed or merged PRs never block a new run. It must then read `state.json`; a non-terminal active run blocks another run. It must also search for an existing run issue for today's Europe/Copenhagen date to prevent duplicates.

## Per-run issue

Each day that passes the prechecks gets one issue named `[Dead-code run] YYYY-MM-DD`. The issue contains the Cloud trigger, Cloud evidence/pending record, local publication result and usage metadata for that run. Close the run issue once terminal. Do not append routine run records to issue #79.

New records use v2 markers so legacy #79 comments cannot be mistaken for live queue state:

- `<!-- alfredo-run-v2 -->`
- `<!-- alfredo-pending-v2 -->`
- `<!-- alfredo-outcome-v2 -->`
- `<!-- alfredo-publication-v2 -->`

## Usage accounting

Track the timer, Cloud producer and local publisher separately. Report usage only from authoritative runtime or OpenAI usage metadata when available. Never manufacture token counts or convert tokens into a subscription-quota percentage without an explicit platform-provided allowance denominator.

Use this shape in each run record:

```json
{
  "usage": {
    "timer": {
      "model": null,
      "input_tokens": null,
      "cached_input_tokens": null,
      "output_tokens": null,
      "credits": null,
      "source": "unavailable"
    },
    "cloud": {
      "model": null,
      "input_tokens": null,
      "cached_input_tokens": null,
      "output_tokens": null,
      "credits": null,
      "source": "unavailable"
    },
    "local": {
      "model": null,
      "input_tokens": null,
      "cached_input_tokens": null,
      "output_tokens": null,
      "credits": null,
      "source": "unavailable"
    },
    "quota": {
      "used": null,
      "remaining": null,
      "unit": null,
      "reset_at": null,
      "source": "unavailable"
    }
  }
}
```

If exact task/thread credits or token breakdowns are exposed by Codex/ChatGPT, copy them exactly and identify the source. If only the account Usage page exposes quota, leave quota fields unavailable in GitHub rather than estimating them.
