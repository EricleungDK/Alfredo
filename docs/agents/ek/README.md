# ek workflow skills (vendored copy)

Read-only copy of the skills from Eric's agent workflow kit,
[EricleungDK/ek](https://github.com/EricleungDK/ek), at commit `1858d50`.

The scheduled scan, build and security-fix runs read these files instead of cloning ek
and running its installer, which the cloud sandbox blocks as "Code from External".

| Skill | Used for |
|---|---|
| `orchestrate-agent/` (with `brief-template.md`) | dividing tickets into parallel sub-agent units, briefs, the review relay, verification |
| `failure-triage/` (with `flaky.md`, `stale.md`) | classifying a red test or CI job before editing code |
| `finish/` | the done/blocked report every agent ends with |

Don't edit these files here. Change them in EricleungDK/ek, then refresh this copy
(copy `skills/orchestrate-agent`, `skills/failure-triage` and `skills/finish` over these
folders and update the commit above).
