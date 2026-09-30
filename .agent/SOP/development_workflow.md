# Development Workflow

**Last Updated**: 2026-09-30
**For**: Alfredo contributors

## Related Docs

- [Project Architecture](../System/project_architecture.md)
- [Persistence migrations](database_migrations.md)
- [Status](../Tasks/STATUS.md) and [historical context](../Tasks/context.md)

## Setup

Ubuntu/WSL2 Linux x86-64, Git, Rust 1.96.0, `bubblewrap` and `util-linux`. Python 3
is test-only. Ollama is optional for live local-model checks.

## Before Making Changes

1. Read `AGENTS.md`, [STATUS](../Tasks/STATUS.md) and the relevant plan under `.agent/Tasks/`.
2. If no active planning artifact exists, read the relevant GitHub `[PRD]` parent and its ordered Issue Slice sub-issues.
3. Preserve unrelated work already present in the worktree.

## Build and test

Run `cargo run --locked --manifest-path alfredo-tui/Cargo.toml -- --model qwen3:14b`
from the repository root with Rust 1.96+. Its current scope and test
commands are in [`alfredo-tui/README.md`](../../alfredo-tui/README.md). Provider
regressions need loopback socket permission; the Linux PTY gate needs a real PTY.
The explicitly ignored live test contacts the installed model only when requested.
Build a development archive with `python3 alfredo-tui/scripts/package_release.py
--output /tmp/alfredo-candidate`, then run `python3 alfredo-tui/tests/release_smoke.py
/tmp/alfredo-candidate/*.tar.gz` to exercise the installed binary outside the checkout.
Rust 1.96.0 and Python 3.11+ are required for packaging; application runtime does not
require them. Use a new output directory for each candidate. Run
`python3 -m unittest discover -s alfredo-tui/tests -p 'test_*.py'` for notice integrity
fixtures. Packaging needs the locked target dependency graph and original cached
crates available offline; it checks each archive against Cargo.lock and retains
nested license/notice texts in THIRD_PARTY_NOTICES.txt with a DEPENDENCIES.json
inventory. Unsupported sources or missing/altered documents refuse packaging.
The installed check verifies payload hashes and individual notice byte ranges.
This is a conservative resolved graph, not linked-code or license compatibility
qualification. First-party code uses the root MIT LICENSE, which is fingerprinted
and included in the seven-member archive. Third-party terms remain separate.
See the [migration plan](../Tasks/rust-terminal-migration.md) for remaining scope.

## Implementation Rules

- Write the failing test first, then make it pass.
- Keep model work cancellable, observable and resource-bounded; the UI never waits on inference.
- Make mutations receipt-idempotent and expected-revision guarded; retrying a lost response must not duplicate work.
- Model output is evidence, not authority: policy, approval and review stay with recorded receipts.
- Add regression tests at every changed boundary, including restart or replay for persisted state.

## Git Workflow

Use conventional commits when the user asks for a commit:

```text
feat(autopilot): bound repair attempts
fix(tasks): replay an acknowledged receipt
test(side-pane): cover narrow layout
docs(workflow): refresh release gates
```

Before committing, inspect `git status`, review the scoped diff, and preserve unrelated user changes. Do not commit, push, create a pull request, or delete branches unless the user asks.

## Debugging and Recovery

- Reproduce a failure at the narrowest public boundary before changing implementation.
- Use a fresh temporary `--state-dir` to tell corrupted local state from deterministic behavior.
- Never edit state JSON by hand; use `/recover` and the review and repair commands.

## Native dependency advisory gate

Install pinned tooling with `cargo install cargo-audit --version 0.22.2 --locked`.
From `alfredo-tui`, run `cargo audit --file Cargo.lock --deny warnings --json`
with network access to refresh RustSec and registry data. The local `.cargo/audit.toml`
sets no advisory exclusions and takes precedence over personal audit configuration.
Do not add target filters or suppress warnings to obtain a green release gate.
From the repository root, `python3 alfredo-tui/tests/audit_smoke.py` then verifies
that the auditor rejects RUSTSEC-2022-0051 in a separate synthetic lockfile, using
the already-fetched database and no fixture compilation. The application lockfile
is untouched. CI retains the JSON report and runs this failure check.

On 2026-09-14, refreshing the database revealed RUSTSEC-2026-0285 in rustls 0.23.44;
the lockfile now uses patched 0.23.45. All 213 locked dependencies then audited
without findings or warnings. This point-in-time scan covers published advisories,
not undisclosed defects, application security or license compatibility. See the
[recorded before/after evidence](../Reports/2026-09-14-native-dependency-audit.json).

## Optional concurrent real-worker acceptance

With local Ollama and Bubblewrap available, run:

```bash
ALFREDO_SMOKE_MODEL=qwen3:14b cargo test --locked --manifest-path alfredo-tui/Cargo.toml --test worker live_parallel_workers_pass_independent_edge_case_checks -- --ignored --exact --nocapture
```

Two workers share one provider and independently edit temporary committed repositories.
One implements strict ASCII port parsing (valid boundaries and invalid types/text);
the other merges intervals across 3,375 combinations plus explicit edge cases, without
mutating inputs. Approved checks are outside model-writable files. Both must create
review-ready candidate commits while the source repositories remain unchanged.
`ALFREDO_SMOKE_PARALLEL_MODELS=1` overrides the test’s default two model slots
for controlled admission comparisons. `LIVE_WORKER_SAMPLE` lines record case, exact requested model, first-content/total
wall time, optional server metrics and outcome. This test uses synthetic fixtures
and is not a general model qualification or sustained workload benchmark. It stays
ignored in ordinary CI; run it explicitly against an installed local model.
