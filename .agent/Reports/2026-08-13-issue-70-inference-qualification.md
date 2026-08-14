# Issue #70 — Local Inference Profile qualification and promotion

## Outcome

Issue #70 adds a bounded qualification workflow for Local Inference Profiles. It evaluates repeated governed fixture observations and reviewed task quality plus goal-to-reviewed-Evidence-Package latency. It does not treat raw token speed, prompts, model streams, plans, Evidence Packages, authority decisions, or source-dependent outcomes as cache truth.

## Implemented boundary

`albert_mvp.inference_qualification` is the public qualification seam:

- `build_governed_fixture_family()` returns the stable v1 family for discussion, routing, small and multi-file edits, repair, malformed output, policy violations, cancellation, long context, model swaps, and queued Local Agents.
- `InferenceQualificationService.qualify()` runs each fixture repeatedly through a supplied governed runner and retains only bounded outcome flags, validity counts, decomposed load/prompt-evaluation/first-token/decoding timings, reviewed latency, model digest, and digest-keyed context/prefix measurements.
- `ContextProfilePlanner`, `compare_context_profiles()`, and `default_context_profiles()` compare bounded controller (8,192 → 16,384) and normal-worker (16,384 → 32,768) context sizes. Expansion is selected only when required material fits after output headroom and reviewed quality or reliability improves.
- `DeterministicContextSelector` keys reusable selection only by role, bounded budget, required source ids, and source digests. `PromptPrefixReuseTracker` reports exact-prefix reuse and invalidation; changing a source changes the digest and invalidates the prefix observation.
- `QualificationReportStore` atomically persists bounded report metadata below `runtime_root/inference/qualification/`, serializes promotion state under a cross-process lock, pins the exact Profile/runtime/binary/configuration digests, requires the canonical fixture family/repetitions, an existing stored baseline, a service-issued rollback-test receipt, and a qualified Profile, rejects withdrawn or non-inferior candidates, supports correlation/revision-guarded exact promotion and rollback replay, and retains an explicit rollback path to the previous active record.
- The one-process CLI and persistent newline transport expose the same `inference-qualification`, `inference-qualification-promote`, and `inference-qualification-rollback` projections over that store.

The promotion state remains scheduling/profile configuration metadata. Python Mission, session, file, command, Evidence Package, review, and accepted-state authority remains unchanged.

## Acceptance mapping

| Issue #70 criterion | Evidence |
| --- | --- |
| Repeated governed fixtures | `tests/test_inference_qualification.py::GovernedFixtureFamilyTests.test_full_family_records_outcomes_for_each_governed_workload` runs all 11 fixture kinds twice. |
| Reports include quality, repairs, escalations, timings, and reviewed latency | `QualificationReport.metrics` exposes route/plan/evidence validity, accepted outcomes, repairs, escalations, policy blocks, cancellations, swaps, queued Local Agents, reliability/quality rates, p50/p95 decomposed timings, and p50/p95 reviewed latency. |
| Bounded controller/worker context comparison | `ContextProfilePlanner`, `compare_context_profiles`, and the default-profile test cover the bounded initial/expanded sizes and output headroom rule. |
| Exact prefix and digest-keyed selection | `ContextQualificationTests` covers cache hits, exact-prefix reuse, changed-prefix invalidation, changed source digests, and no cached result/evidence fields. |
| Safe promotion and rollback | `QualificationReportStore` tests cover exact runtime pinning, withdrawn-runtime rejection, non-inferior quality/reliability rejection, qualified-profile/fixture/repetition gates, rollback receipt and baseline existence, idempotent promotion, persistence reload, and rollback. |

## Verification

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -p 'test_inference_qualification*.py' -v` — 23 passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 -m compileall -q albert_mvp tests/test_inference_qualification*.py` — passed.
- `ruff check` and `ruff format --check` passed for the new qualification module and tests.
- Documentation audit note: the SOP-referenced `audit_documentation.py` and `validate_standards.py` scripts were not present in this checkout, so those optional commands were not claimed; the documentation was manually reconciled against the System/API/schema/UX/domain sources.
- Live Ollama/GPU qualification was not claimed; the tests use deterministic governed-runner fixtures and do not represent production model quality or latency.

## Follow-up boundary

The report store is intentionally separate from Mission runtime authority. A later product surface may expose report inspection or connect promotion state to profile selection, but it must preserve the same exact runtime pin, rollback, and no-source-truth-cache rules.
