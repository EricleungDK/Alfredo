# Testing and mechanical review

The testing standard is [ek's](https://github.com/EricleungDK/ek/blob/v0/docs/testing-standard.md): what a good test is, the forbidden cheats, and what mech-review checks versus code review. Alfredo specifics:

- **Gates:** `alfredo-tui/scripts/agent-gates.sh` (see `automation-runbook.md`).
- **Test paths:** `alfredo-tui/tests/**`.
- **Protected paths:** `.github/**`, `AGENTS.md`, `docs/agents/**`, `alfredo-tui/scripts/agent-*`, `alfredo-tui/.cargo/**` (cargo and cargo-mutants config). A change there is a `needs-human` entry: a human decides.
- **Red first:** on `claude/` branches that change production code, at least one new or changed Rust test under `alfredo-tui/tests/` must fail on `main`'s code (mech-review runs the PR's tests there); tests that pin existing behaviour are guards beside it. A PTY smoke (`tests/*.py`) does not count: R1 runs Rust tests only, so logic only a smoke reaches (the `main.rs` run loop, key handling) moves into a pure fn with a Rust test. Fix an R1 fail with a new commit; no history rewrite.

## Reading the result

Every PR gets one `<!-- mech-review -->` comment from `github-actions[bot]`, edited in place, with a JSON block. Trust only the bot's comment, and only when its `head` is your head SHA.

- `fail` entry: fix it, using `hint`. Never take `forbidden`. The fix commit carries the body line `mech: <id>`.
- `confirm` entry: a reviewer decides.
- `needs-human`, or the same id still failing after 2 fix rounds: stop and hand it to the owner.
- `skipped`, `partial`, `n/a`: not fully run, so they back nothing.
