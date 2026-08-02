# Conversational action claims bound to Orchestrator receipts

**Implemented:** 2026-08-02

**Issue:** [GitHub #59](https://github.com/EricleungDK/Alfredo/issues/59)

**Parent PRD:** [GitHub #56](https://github.com/EricleungDK/Alfredo/issues/56)

**Baseline:** `1ad8e72` on `main`

**Related diagnosis:** [Workspace selection and false-success routing](2026-07-24-workspace-selection-false-success-diagnosis.md)

## Outcome

Agent Console controller prose is now explicitly non-authoritative. Every persisted controller reply carries either `no-action` or `awaiting-orchestrator` with visible explanatory text. Malformed routes discard the model's prose, and success-sounding controller effect claims are replaced by fixed unverified-effect feedback. A controller route may therefore classify work, but it cannot claim proposal, approval, launch, mutation, review, or completion.

Canonical Queue items retain and validate the exact proposal and decision correlation ids that created their current projection. Session summaries derive exact launch, evidence, and Review Decision identities from the authoritative Mission runtime. React transports and renders those identities beside separate proposal, decision, queued, running, evidence, Review Decision, and accepted-completion chronology entries. Controller commentary is visually distinct and never receives receipt styling.

## Contract changes

- `AgentConsoleMessage` adds `action_outcome` and `action_message`; the existing `correlation_id` and `action_phase` now reach Tauri and TypeScript instead of being dropped.
- `model-commentary` rejects receipt identity at both append and persistence-read boundaries, so controller prose cannot be forged into a canonical event by a caller or a tampered history file.
- `WorkspaceQueueItem` adds `proposal_correlation_id` and `decision_correlation_id`. Current stores validate each non-empty identity against the exact receipt kind, item, request, acknowledgement, and canonical state; empty fields remain a legacy-read compatibility path.
- `MissionSessionSummary` adds `launch_correlation_id`, `evidence_correlation_id`, and `review_correlation_id`, derived only from canonical session task packets, validated evidence state, and persisted Review Decision metadata.
- Controller prompts explicitly prohibit effect claims. Structurally malformed output becomes fixed non-action discussion, and a bounded first-person/completion claim guard prevents known unaudited success wording from surviving as the primary answer.
- React emits no generic session success line. A running claim requires both canonical runner start state and a launch correlation; evidence and review/completion milestones require their respective canonical identities.

## Replay and failure boundaries

Existing correlation protocols remain authoritative. An exact proposal or decision retry replays the original acknowledgement and creates no duplicate Queue item or Local Agent session. A changed Mission, Conversation Scope, goal, acceptance criteria, paths, policy, worker, origin, actor, target, decision, reason, or Review request fails closed. Lost Queue-decision responses recover an already-durable session only for the exact original approval. Projected receipt identities that are missing from, point to, or conflict with another durable receipt fail the persistence read instead of becoming UI authority.

## Verification

Focused red/green evidence completed during implementation:

- Python action-truth, malformed-route, commentary-forgery, Queue identity, projected-receipt forgery, replay, and session lifecycle regressions: **12 passed** in the consolidated focused run.
- Complete React `App.test.tsx` boundary suite, including commentary/receipt, durable proposal-decision-queued ordering, and running/evidence/review/completion milestones: **125 passed**.
- TypeScript typecheck: **passed**.
- Real Python through Tauri: Agent Console action truth and ad hoc proposal/approval/launch identities: **2 passed**.
- Production Vite build: **passed** (37 modules).
- Responsive Chromium geometry suite: **4 passed** after repairing its stale Tauri-detection fixture.
- Rust formatting and `git diff --check`: **passed**.

The complete matrices were also run once on the supplied Darwin host, while this repository's supported development environment is Ubuntu:

- Python: **453 total — 243 passed, 32 failed, 170 errors, 8 skipped**. Failures are dominated by unavailable Linux `bwrap`/`prlimit`/`/proc` boundaries and macOS `/var` → `/private/var` aliases. The prior #58 broad run had the same 32-failure baseline; two isolated workstation receipt-recovery failures also reproduce in pre-existing journal/atomic ordering code outside this change.
- Vitest: **251 total — 225 passed, 26 failed** across 13 files. `App.test.tsx` is 125/125 green; remaining failures are Darwin packaging/runtime permissions, Linux sandbox requirements, and temporary-path aliases.
- Desktop Rust: **45 total — 35 passed, 10 failed**. The two new real-Python bridge tests pass; remaining failures require Bubblewrap or hit Darwin temporary-path aliases.
- The rendered release seam now passes canonical temporary-path setup and displays the ad hoc proposal receipt, then stops only when the real Linux-bounded runner cannot start on Darwin.
- `npm run release:verify` stops at the unsupported Darwin AppImage bundle target; `npm run release:check` then correctly reports that no verified artifact set exists.

Documentation consolidation links the prior false-success diagnosis to this implementation, records the API and UX contracts, and updates discoverability. The standards validation is **A / 96.5%** with **86% traceability**; the audit reports seven remaining historical connection/orphan heuristics unrelated to #59.

The independent Standards/Spec review and exact commit are recorded in the Active Orchestration Context and authoritative Issue #59 completion comment after they run.

## User-visible result

In the Agent Console chronology, model commentary uses a dashed/non-authoritative treatment and a prominent amber action-truth line. Canonical events show `Receipt <correlation> · <phase>` and stay distinct through restart. The functional UI uses the managed native launcher; the standalone Vite page remains a visual-only preview without authoritative Tauri/Python state.
