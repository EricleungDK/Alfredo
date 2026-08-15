# Issue #69 — Local Inference Runtime Governance

**Status:** Implemented and hardened on the isolated Issue #69 feature branch; ready for authoritative tracker handoff.

## Contract

Issue #69 replaces the normal opaque Ollama model-turn path with a versioned Local Inference Profile, a bounded HTTP adapter, and one non-authoritative Local Inference Lease. Python remains the authority for Mission state, session lifecycle, files, commands, evidence, and review.

Each turn records the resolved model digest, keep-alive, context/output budgets, thinking and sampling settings, declared schema, quantization, residency, requested processor policy plus observed GPU/total bytes, admission decision, lease identity, and load/prompt-evaluation/first-token/decoding timings. The adapter sends Alfredo's already-composed prompt with Ollama `raw: true` and uses a tokenizer-independent one-token-per-UTF-8-byte bound, so admission rejects insufficient output headroom before generation without hidden template expansion.

The adapter bounds installed/running metadata, stream lines, response and thinking bytes, output tokens, timeout, and cancellation. A candidate completion must carry every Ollama usage/timing metric and must match exactly one `/api/ps` running-model digest with bounded `size_vram`/`size` evidence. It becomes authoritative only after that runtime proof, complete JSON, the bounded declared schema, and the caller's domain validator all pass. Partial, malformed, oversized, timed-out, cancelled, transport-failed, metadata/digest, queue, and lease outcomes remain structured and non-authoritative.

The lease persists one active entry, a bounded queue, resident model identity, and bounded audit history. Only an authoritative completion with matching running-model evidence updates residency. Selection is highest priority first, then qualified resident-model affinity, then FIFO sequence. The lease cannot authorize Mission work or alter accepted state.

## Projections

`runtime.json` stores at most 128 validated inference receipts without prompts or raw streams. Workspace snapshots expose aggregate turn/last-turn metadata and lease state; session summaries expose the last turn for the exact Local Agent session; the workstation-session CLI response exposes exact-session turns plus the lease projection. React receives explicit TypeScript contracts and renders a Local Inference overview in Mission Work plus per-session telemetry in the Mission Execution inspector. Idle, queued, running, complete, and non-authoritative states remain distinct.

## Verification

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_inference`: 23 run, 22 passed, 1 Bubblewrap-dependent test skipped.
- Combined Issue #69/#70 inference and qualification boundary: 69 run, 68 passed, the same 1 Bubblewrap-dependent test skipped.
- `npm run typecheck`: passed; `npm run build`: passed.
- `npm test -- --run src/App.test.tsx src/styles.test.ts`: 147 passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 -m py_compile ...`, Ruff, Ruff format, and `git diff --check`: passed for the hardened inference files.
- The merged Issue #71 execution contract/integration boundary passed 34 tests on the isolated branch, confirming the Profile integration preserved the committed host-effect seam.
- Persistence reload rejects malformed receipt shape; invalid processor evidence and corrupt lease entries fail closed; completed output without stage metrics or matching running-model evidence remains non-authoritative.
- A wider isolated selection covering Mission selection, Workspace selection, and Workspace snapshot behavior executed 285 tests with 21 failures, 60 errors, and 6 skips. Those failures are outside the focused inference/qualification boundary and are dominated by unavailable trusted Bubblewrap and macOS `/private/var` alias assumptions; this is recorded as non-admissible broad evidence, not as a green matrix or an Issue #69 focused failure.
- The persistent Apple workstation could not start in this environment (`Operation not permitted`), so visual acceptance remains unverified.
- Live Ollama/GPU verification was not claimed and is not an Issue #69 closure prerequisite: deterministic fake HTTP coverage exercises `/api/tags`, `/api/generate`, and `/api/ps` against the documented Ollama contract. A live supported-machine run remains a useful optional smoke. The explicit-command compatibility path remains separate from the normal HTTP Profile path.

## Closure Boundary

All five ticket criteria have deterministic implementation evidence. Local Ollama is not a closure prerequisite. The isolated feature-branch commit carries only the integrated #69 contract on top of committed #70/#71/#72 history and leaves the stale default index and active main worktree untouched.

## Files

- Backend contract and adapter: `albert_mvp/inference.py`, `albert_mvp/core.py`
- Agent/profile decoding and projections: `albert_mvp/agents.py`, `albert_mvp/cli.py`, `albert_mvp/workspace.py`
- Frontend contract and Mission Work telemetry: `mission-control/src/contracts.ts`, `mission-control/src/MissionExecutionTree.tsx`, `mission-control/src/App.tsx`, `mission-control/src/styles.css`
- Regression coverage: `tests/test_inference.py`, `mission-control/src/App.test.tsx`
