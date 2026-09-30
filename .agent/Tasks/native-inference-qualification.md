# Native inference runtime and profile qualification

Date: 2026-09-26  
Status: bounded diagnostic harness implemented and verified. The corrected live cohort completed 8 cases with 1 canonical accepted outcome; model/profile quality remains unqualified. Production defaults and profile promotion remain unchanged.  
Authority: the active Rust rewrite and disconnect/response-latency request; GitHub [#69](https://github.com/EricleungDK/Alfredo/issues/69) and [#70](https://github.com/EricleungDK/Alfredo/issues/70), reconciled through the [native regression inventory](rust-terminal-regression-inventory.md).

## Problem and evidence

Native Alfredo bounds streams and shares inference capacity across terminals, but cannot yet connect a repeated workload result to a resolved model digest, observed runtime context, and exact requested profile. `provider.rs::models()` retains installed names only; native `/api/chat` requests specify `num_predict: 4096` and omit `num_ctx`. Larger source/history requests therefore lack qualified context-fit and reviewed-quality evidence.

The [September 14 repeated-worker observations](../Reports/2026-09-14-generation-and-repeated-workers.json) recorded Ollama 0.34.0, qwen3:14b Q4_K_M, advertised context 40,960, and running context 4,096. Ten small task runs passed independent checks and reached ReviewReady. These are historical observations on one host, not proof of today's runtime configuration, a truncation diagnosis, accepted review outcomes, or broad model qualification. The structured-thinking EOF workaround has separate [bounded evidence](../Reports/2026-09-14-structured-thinking-workaround.json); it does not justify another guessed timeout, thinking, or concurrency change.

This slice should make one comparison reproducible: can an explicit bounded context profile improve required-source task outcomes and goal-to-reviewed-result latency while preserving native admission and task authority?

## Authority and legacy comparison bounds

The Issue #69 implementation report defines instrumented profiles, bounded metadata, digest/runtime evidence and separate inference admission. The Issue #70 report defines repeated governed outcomes, context/prefix measurements and exact runtime/configuration identity. The live #70 body fetched on September 26 still requires reviewed quality and latency, bounded context comparisons, exact-prefix/digest context, and pinned promotion with rollback. Its closed state is not native acceptance evidence.

The actual legacy values are in `default_context_profiles()` and are asserted by `test_default_controller_and_worker_profiles_use_bounded_v1_sizes` in `tests/test_inference_qualification.py`:

| Legacy role | Initial context | Expanded context | Legacy output budget | Legacy keep-alive |
| --- | ---: | ---: | ---: | --- |
| Controller | 8,192 | 16,384 | 1,024 | 5m |
| Normal worker | 16,384 | 32,768 | 2,048 | 10m |

Legacy profiles use `/api/generate` with `raw: true`, temperature 0.2 and top-p 0.9. Native structured calls use templated `/api/chat`, output limit 4,096, temperature 0 and requested thinking off; ordinary discussion leaves thinking/sampling at server defaults. Copying all legacy settings would confound a context comparison. Legacy `qualified` metadata is not evidence that any native profile is qualified.

Proposed first comparison: unchanged native wire behavior versus explicitly requested controller context 8,192 and worker context 16,384, preserving every other native wire setting. Treat the candidate as a new native experimental profile, not full legacy profile parity. Controller includes foreground planning/Architect calls; scheduling class remains explicit and independent of JSON formatting. Expanded 16,384/32,768 candidates require a separately declared follow-up comparison after initial evidence shows missing required material and a plausible fit. Never expand automatically.

The selected model/digest, supported host, controller-role mapping and applicability of those legacy sizes must be confirmed from fresh runtime observations before a live cohort. The table records existing definitions, not an agreement to change native defaults.

## Integrated implementation boundary

### Bounded runtime inspection

Add a read-only inspection seam adjacent to `provider.rs`, reusing the configured HTTP origin and transport restrictions. Inspect `/api/version`, `/api/tags` and `/api/ps`; retain only selected-model metadata. Reuse model discovery's ten-second deadline, 1 MiB response ceiling and 256-entry catalog ceiling for each inspection response/list. Bound retained strings, integers and collection depth through an explicit allowlist; never retain full arbitrary server JSON. Preserve the existing 200-byte model-name bound. Define the remaining identity/string limits in the report contract before implementation.

Keep requested, resolved and observed facts distinct: selected model name; catalog digest and quantization; server version; running digest, context length, total and GPU bytes; requested context/output/thinking/sampling/format/keep-alive. Unsupported or absent fields are unknown, not zero or inferred from advertised model capacity. Duplicate or mismatching selected-model identities cannot provide qualification proof. Record metadata before and after each scenario and flag drift within a cohort. A matching `/api/ps` snapshot is an observation, not atomic proof that a mutable tag used those bytes throughout a request.

Inspection must not acquire inference capacity, load/unload models, mutate missions, or run at provider construction. Existing discovery and ordinary chat remain usable when optional qualification metadata is unavailable. Explicit qualification records missing/malformed proof as non-qualifying; it cannot silently substitute a model name for a digest. Capture actual executable/configuration digests where available; a server version string alone is not a verified runtime binary pin. Remote or otherwise unverified runtime pins remain explicit limitations.

### Exact experimental profile and prompt identity

Add an immutable opt-in request profile shared by the qualification runner and provider's actual payload construction. Baseline serialization must stay byte-for-byte equivalent in meaning to current native requests. Represent omitted settings distinctly from explicit values, especially `num_ctx`, thinking, temperature, keep-alive and format. Profile identity includes schema/version, role, selected/resolved model identity, every wire option, format/schema digest, endpoint origin, admission class/capacity and deadlines. Record a canonical profile digest and the digest of the actual serialized request; do not reconstruct claimed settings from UI labels or defaults later.

The context-only candidate adds explicit `options.num_ctx` at the declared role size. It preserves structured thinking/output/sampling, ordinary chat omission semantics, queue-inclusive total deadline, idle deadline and shared admission policy. Cloning into planner/worker must retain the exact profile. Existing cancellation and post-admission canonical-state guards still run immediately before generation HTTP.

Use the native prompt builders and record bounded source-id/content-digest manifests, source ordering, message roles/order and exact serialized reusable-prefix digest separately from fixture suffix and full-request digest. Changed source bytes, order, system text, schema or profile invalidate corresponding reuse observations. Measure equality of the transmitted prefix; do not claim Ollama cache hits from client prefix equality. Do not cache plans, generated edits, evidence, review decisions or accepted state as source truth.

Templated `/api/chat` has server-controlled expansion. The legacy one-token-per-UTF-8-byte proof for raw prompts cannot establish native token headroom. Retain byte limits, required-source manifests and observed prompt counts; mark exact token fit unknown unless a compatible tokenizer/template proof is implemented separately. A recall failure alone cannot prove truncation. Never silently discard required source material to make a candidate appear to fit.

### Governed native cohorts

Provide an explicit opt-in runner in isolated temporary workspaces/missions using real native scope, planner, approval, worker, check and review seams. No user's active mission becomes an experimental fixture. Use the same source bytes, check definitions, model digest and initial state for paired baseline/candidate scenarios; use fresh task identities and restore fixture state between repetitions. Pin the fixture-definition and run-manifest digests before the first request.

The proposed initial diagnostic cohort has four scenarios, three paired repetitions each: 24 scenario executions, not a statistically established non-inferiority study. This repetition count is a proposal, not a legacy agreed value. A fixed manifest must also cap generation attempts, repair attempts, elapsed run time and report bytes before implementation; exhausted bounds become recorded failures and never trigger an unbounded retry.

| Scenario | Governed path and independent proof |
| --- | --- |
| Small edit | Native planner draft → explicit save/approval → worker edit → independent edge-case check → canonical review decision. Retain existing strict-port/interval fixtures where suitable. |
| Required-source multi-file/long context | Deterministic repository with necessary facts in separated source locations; record exactly which bytes the native builder supplied. Checks require those facts, not a generic plausible answer. If the current builder cannot supply the fixture material, record a context-selection limitation before drawing model conclusions. |
| Repair | Deliberately failing initial candidate → recorded review/repair lineage → separately approved repair → independent check and review. Record all model attempts and elapsed repair cost. |
| Queued foreground work | Occupy shared capacity with an actual background worker, then submit foreground discussion/planning. Verify class/admission identity, queue delay and useful checked response while retaining background completion evidence. |

The deterministic fixture oracle may issue existing review transitions only in isolated test missions after its independent checks. Keep generated output, valid plan/evidence, ReviewReady, checked outcome and canonical Accepted distinct. Failed or unreviewed work has no successful reviewed-latency sample. Record repairs, escalations, policy refusals, cancellations and incomplete runs without dropping them from denominators or converting missing times to favorable zeros.

Declare paired ordering and cold/warm classification in the manifest. Observe residency and load timings; do not call a run cold merely because it is first, or unload another user's model to manufacture a cold cohort. Preserve ordinary timing units and separate client queue, first content, stream/total, server load/prompt/decode, and goal-to-reviewed-result latency. Keep individual observations and sample counts; small diagnostic distributions cannot support a broad speed or quality claim.

### Bounded report, separate from task authority

Write a versioned bounded qualification artifact containing the exact manifest/profile/runtime/source identities, individual outcomes and timing observations, canonical task/run/review references, missing-proof reasons and derived summaries. Keep raw prompts, streams and source contents outside the report; hashes identify the separately reproducible fixtures. Atomic report write/reload must validate bounds, allowlisted fields, digest relationships, expected repetitions, timing eligibility and derived outcome counts. A partial cohort stays incomplete after restart and never resumes inference automatically.

Report inspection must distinguish reproducible native fixture evidence from observations lacking a verified upstream runtime pin. Missing metadata, runtime/model drift, request/profile mismatch, cancellation, truncated output or invalid checks cannot yield a favorable comparison. This report neither grants task authority nor marks a profile promoted. It does not require a conversation/task schema migration merely to persist experiment metadata.

## Required regressions and acceptance

1. Fake HTTP inspection: malformed/oversized/slow metadata, invalid identity/numeric fields, duplicate selected digests, absent runtime context, wrong running digest, and before/after drift. Pure construction/discovery never queues inference or writes coordinator state.
2. Wire capture: unchanged baseline omission semantics; exact candidate role context; profile clones preserve options; structured/plain settings remain distinct; reported canonical identity matches the bytes actually sent. Changing any wire-affecting setting invalidates identity.
3. Admission lifecycle: queued cancellation and stale planner/worker state send no generation HTTP even with qualification enabled; capacity still releases on probe/request/error/drop, and queue time remains inside the existing deadline.
4. Context/prefix: changed source bytes/order, system text, format or profile invalidates reuse; required sources cannot disappear silently; equal prefixes do not imply server cache hits; templated requests never claim the raw byte-to-token proof.
5. Governed runner: real planner/worker/check/review transitions, exact repair lineage, independent source-dependent checks, no automatic approval outside isolated fixtures, bounded attempts, cancellation and restart without replay. Deterministic malformed/policy/EOF cases remain failures rather than fast successful samples.
6. Report validation: tampered profile/request/fixture/runtime digests, missing/duplicate repetitions, cross-run review references, invalid or non-finite timings, missing metadata and incomplete runs cannot produce complete favorable results. ReviewReady cannot count as Accepted; all attempted scenarios remain represented.
7. Run focused native provider/planner/worker/report tests, full native tests and strict Clippy under the root's single coordinated build. Add one installed-binary fixture smoke proving explicit profile transmission and artifact identity. Only an opt-in live paired cohort with fresh metadata can support model-specific findings.

Acceptance for this slice is a bounded reproducible native diagnostic harness plus honest reports for the selected scenarios, with production defaults unchanged. Full #70 parity remains open: the complete eleven-kind governed family, broad role qualification, reliable token-headroom admission, verified runtime withdrawal/pinning, profile selection/promotion and tested rollback require later slices. No live performance claim is made by this plan.

## Initial implementation questions (resolved below)

- Confirm the experimental model/digest, host/runtime evidence and exact controller-role mapping; select applicable initial legacy context bounds without treating advertised context as usable runtime proof.
- Fix metadata identity limits and cohort attempt/deadline/artifact caps in a versioned manifest contract; keep the proposed three-pair count visibly diagnostic.
- Choose the smallest explicit installed runner entrypoint and report location independent of mission authority, plus a reproducible source for binary/runtime/configuration hashes. Do not require production profile switching or promotion to run this experiment.


## Current implementation decisions (2026-09-27)

The explicit standalone CLI is `--qualify-inference REPORT`, with 1–3 paired
repetitions (default3), and read-only `--inspect-qualification REPORT`. Controlled
experiments use one Alfredo client slot, preserving production default2. Four
scenarios yield at most24 executions; recorded generation dispatch is capped at128,
the cohort stops new work after1800 seconds, and the runner cancels and joins active
work before returning. Each scenario has an internal240-second deadline. Retained
artifacts live beside the new report; output never overwrites or resumes an earlier
report. Report JSON is bounded to4MiB and has a canonical manifest/report digest.

Runtime metadata bounds are model200 bytes, origin2048, version128, quantization64,
context1..2^24 and model bytes0..2^60. Each endpoint has a10-second/1MiB limit; JSON
has depth8,8192nodes,256array entries and64keys perobject limits. Selected digests
use64lowercase hexadecimal characters. Snapshot observations never establish an
atomic upstream binary pin. Root inspected Ollama0.34.0 and the unchanged local
qwen3:14b digest on September26; no model was resident at that observation.

Baseline omits context as before; the candidate explicitly requests8192 for
foreground and16384 for background. The initial intended live diagnostic is one
paired repetition across four cases, followed by further repetitions only after
fixture correctness and runtime behavior are inspected. No favorable quality or
latency result is presumed. Per-request runtime snapshots are being added before
releasing the permit so one final worker observation cannot falsely establish an
earlier planner's context.

The fixture oracle compares typed results in a separate fixed parent process;
generated modules execute in a bounded child. An early exit or a printed success
marker cannot bypass the parent's assertions. Required reference hashes are checked
before and after execution. This is independent behavioral checking, not arbitrary
code attestation. Reports bind every accepted governed phase to its recorded
generation; queue evidence must overlap the exact pending background generation.
The oracle, provider and four runner scenario regressions passed their focused
gates; the final full/installed and live gates remain in progress.

## Verification checkpoint (2026-09-27)

[Implementation and verification report](../Reports/2026-09-27-native-inference-qualification.json)
records the 357-test full native pass (7 opt-in ignored), followed by 48 final focused
library/runner/oracle checks, strict Clippy, formatting and all 5 installed tests.
The final fixture-v2 archive source and payload hashes match the checkout.

The original fixture-v1 diagnostic
stopped after 4 cases, with no accepted work and one 240-second scenario cancellation.
Its worker criteria omitted exact behavior checked by the oracle. Actual worker
transcripts reproduced the omission; fixture-v2 criteria now carry the complete
contract independently of planner task titles, verified through captured HTTP.
The corrected fixture-v2 diagnostic
completed all 8 cases and 16 dispatched generations. One required-source candidate
reached canonical acceptance in 19,241 ms. Port parsing still admitted Unicode digits;
baseline reference work modified pinned source; repair plans did not retain the seed
marker and stopped before worker execution. Both queued discussions passed their
independent response check, while their coding tasks failed.

These outcomes preserve failures and refusals without attributing them all to model
capacity. No default/profile promotion or general performance claim follows. A scenario
limit currently cancels the cohort token and leaves later cases pending, as the original
report demonstrates. Matching compiled fixture definitions are required for inspection;
the retained original binary inspects the historical v1 report. Runtime binary pinning,
exact token headroom, the full fixture family and reliable reviewed quality remain open.

The next proposed user-visible slice is the [native Mission Work tree](native-mission-work-tree.md).
