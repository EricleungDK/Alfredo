# Conversational action claims bound to Orchestrator receipts

**Implemented:** 2026-08-02

**Issue:** [GitHub #59](https://github.com/EricleungDK/Alfredo/issues/59)

**Parent PRD:** [GitHub #56](https://github.com/EricleungDK/Alfredo/issues/56)

**Baseline:** `1ad8e72` on `main`

**Related diagnosis:** [Workspace selection and false-success routing](2026-07-24-workspace-selection-false-success-diagnosis.md)

## Outcome

Agent Console controller prose is now explicitly non-authoritative. Every persisted controller reply carries either `no-action` or `awaiting-orchestrator` with visible explanatory text. Raw model reply prose is never retained as the action result: valid output becomes a fixed discussion or coding-route template, while malformed output becomes fixed non-action discussion. A controller route may therefore classify work, but it cannot claim proposal, approval, launch, mutation, review, or completion through an unexpected success idiom.

Canonical Queue items retain and validate the exact proposal and decision correlation ids that created their current projection. Session summaries derive exact launch, evidence, and Review Decision identities from the authoritative Mission runtime. React transports and renders those identities beside separate proposal, decision, queued, running, evidence, Review Decision, and accepted-completion chronology entries. Controller commentary is visually distinct and never receives receipt styling.

## Contract changes

- `AgentConsoleMessage` adds `action_outcome` and `action_message`; the existing `correlation_id` and `action_phase` now reach Tauri and TypeScript instead of being dropped.
- `model-commentary` rejects receipt identity at both append and persistence-read boundaries, so controller prose cannot be forged into a canonical event by a caller or a tampered history file.
- `WorkspaceQueueItem` adds `proposal_correlation_id` and `decision_correlation_id`. Current stores validate each identity against the exact receipt kind, item, request, acknowledgement, and canonical state. Legacy empty fields backfill only from one unique valid receipt; React suppresses the effect chronology when no chain can be proven.
- `MissionSessionSummary` adds `launch_correlation_id`, `evidence_correlation_id`, and `review_correlation_id`. Launch requires one exact Queue/Workstation receipt; Evidence Packages persist a Mission-qualified identity with exact replay; Review requires matching request, workspace event, and Journal record. Forged nested runtime strings project empty.
- Controller prompts explicitly prohibit effect claims. Structurally malformed output becomes fixed non-action discussion, and all valid untrusted reply prose is replaced by a deterministic template selected from the typed discussion/coding route instead of an enumerable verb blacklist.
- React emits no generic session success line. A running claim requires both canonical runner start state and a launch correlation; evidence and review/completion milestones require their respective canonical identities.
- Common accepted Workstation turns verify and render the correlation returned by their typed acknowledgement. A mismatch becomes a failed, unreceipted turn.
- Queue attention remains visible in Mission Work, but is not synthesized into Agent Console chronology. Receipt-bound Queue proposal, decision, and queued entries are the only conversational Queue authority source.
- Exact Evidence Package replay reconciles a missing Activity Journal receipt after a runtime-first write failure and remains correlation-idempotent after the Journal phase exists.

## Replay and failure boundaries

Existing correlation protocols remain authoritative. An exact proposal or decision retry replays the original acknowledgement and creates no duplicate Queue item or Local Agent session. A changed Mission, Conversation Scope, goal, acceptance criteria, paths, policy, worker, origin, actor, target, decision, reason, or Review request fails closed. Lost Queue-decision responses recover an already-durable session only for the exact original approval. Projected receipt identities that are missing from, point to, or conflict with another durable receipt fail the persistence read instead of becoming UI authority.

## Verification

Focused red/green evidence completed during implementation and review correction:

- Python action-truth, malformed-route, commentary-forgery, exact typed-message, legacy Queue backfill/suppression, projected-receipt forgery, evidence replay, and session lifecycle regressions: **17 passed** in the initial consolidated focused run; the final structural-controller, Queue-attention, and evidence-Journal correction matrix passed **11 tests**.
- Complete React `App.test.tsx` boundary suite, including commentary/receipt, Workstation acknowledgement correlation, legacy Queue suppression, durable proposal-decision-queued ordering, and running/evidence/review/completion milestones: **127 passed**.
- TypeScript typecheck: **passed**.
- Real Python through Tauri: Agent Console action truth and ad hoc proposal/approval/launch identities: **2 passed**.
- Production Vite build: **passed** (37 modules).
- Responsive Chromium geometry suite: **4 passed** after repairing its stale Tauri-detection fixture.
- Rust formatting and `git diff --check`: **passed**.

The complete matrices were also run once on the supplied Darwin host, while this repository's supported development environment is Ubuntu:

- Python: **459 total — 253 passed, 30 failed, 168 errors, 8 skipped**. Failures are dominated by unavailable Linux `bwrap`/`prlimit`/`/proc` boundaries and macOS `/var` → `/private/var` aliases. The prior #58 broad run had the same platform classes; the isolated workstation receipt-recovery case reaches the rejection-audit path because this host cannot start the trusted Bubblewrap boundary, rather than its intended mid-write recovery path.
- Vitest: **253 total — 227 passed, 26 failed** across 13 files. `App.test.tsx` is 127/127 green; remaining failures are Darwin packaging/runtime permissions, Linux sandbox requirements, and temporary-path aliases.
- Desktop Rust: **45 total — 35 passed, 10 failed**. The two new real-Python bridge tests pass; remaining failures require Bubblewrap or hit Darwin temporary-path aliases.
- The rendered release seam now passes canonical temporary-path setup and displays the ad hoc proposal receipt, then stops only when the real Linux-bounded runner cannot start on Darwin.
- `npm run release:verify` stops at the unsupported Darwin AppImage bundle target; `npm run release:check` then correctly reports that no verified artifact set exists.

Documentation consolidation links the prior false-success diagnosis to this implementation, records the API and UX contracts, and updates discoverability. The standards validation is **A / 96.5%** with **86% traceability**; the audit reports seven remaining historical connection/orphan heuristics unrelated to #59.

The initial independent Standards/Spec review of `b6996b4` drove the first fail-closed corrections. Re-review of `0a85cf0` then replaced the remaining finite prose blacklist, removed attention-derived transcript authority, and added interrupted evidence-Journal recovery. The closure re-review of `1ad8e72...fed8155` was clean on both Standards and Spec. Issue #59 is closed completed with all five criteria checked and the exact commits/evidence in its [authoritative completion comment](https://github.com/EricleungDK/Alfredo/issues/59#issuecomment-5164457877).

## User-visible result

In the Agent Console chronology, model commentary uses a dashed/non-authoritative treatment and a prominent amber action-truth line. Canonical events show `Receipt <correlation> · <phase>` and stay distinct through restart. The functional UI uses the managed native launcher; the standalone Vite page remains a visual-only preview without authoritative Tauri/Python state.
