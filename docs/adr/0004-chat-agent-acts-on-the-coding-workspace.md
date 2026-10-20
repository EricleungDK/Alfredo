# Chat Agent acts on the Coding Workspace

Status: accepted (2026-10-20). Issue #232, PRD #231.

Chat was read-only: it could not read the repo, see git state, edit files, run commands or touch git, so every small operation (fix a README, check the branch, merge a `/go` branch, push) needed `/go` and a Worktree. Chat becomes the **Chat Agent**: an Ollama tool-calling loop that acts directly on the Coding Workspace (the main checkout), with every action beyond reads gated by owner **Approval**.

This supersedes, for chat only, the invariants "Alfredo never modifies the Coding Workspace's branch, index or working files" and "nothing is pushed". Workers, Autopilot, Worktrees, Permits, Checks and Review are unchanged: governed work still runs isolated and never touches the main checkout or a remote.

## Approval tiers

- **Read** (files, search, git status/log/diff): auto, no prompt.
- **Edit** (write or patch files), **shell** (`run_command`) and **git write** (commit, branch, merge, checkout, reset): ask each call.
- **Push** (any write to a remote): always asks. It can never be added to an allow rule.
- No skip-approval mode.

## Shell boundary

- `run_command` runs inside the bubblewrap Sandbox by default: Coding Workspace mounted, network off.
- The model may request **host escalation** for one call (real git, network, `gh`, installs). It runs outside the Sandbox only after owner approval, shown on a distinct warning card.
- An escalated call is never auto-approved by a sandboxed allow rule; only an explicit escalated allow rule can approve it. Push stays non-allowlistable even when escalated.

## When chat hands off

Chat suits small, local, reviewable changes. It should suggest `/plan` or `/go` instead when work is large, spans many files, needs Checks or Review, or is risky (migrations, broad refactors, deletions), so it runs in a Worktree under Permits.

## Consequences

- The main checkout can now change during a session; `/go` still bases on committed HEAD and dirty-tree warnings still apply.
- Approval prompts are the safety boundary for chat; their wording and the escalation card must stay unambiguous.
