# Native inference admission across terminals

Date: 2026-09-26
Status: implemented and verified as a bounded native scheduling slice; full Issue #69 parity remains open.
Authority: active Rust rewrite goal, original disconnect/response-latency complaint, and regression inventory Issue #69.

## User scenario and current evidence

Two Alfredo terminals each configure two model slots. Before this slice, `Ollama::new` created a process-local semaphore, so both terminals can send four simultaneous requests to the same Ollama endpoint. Foreground discussion then competes with background worker traffic at the server while the UI can only describe its own local queue. Clones share capacity inside one process; separate provider instances and processes do not share a lease or scheduling priority.

Existing deterministic admission tests cover queueing/cancellation only within one provider's clones. The structured-thinking EOF workaround has small live-worker evidence, but that does not qualify sustained concurrent inference, cold/warm latency or model quality. The current ten-minute total deadline intentionally includes queue time; do not change it based on a guessed timeout defect.

## Proposed integrated behavior

Add an endpoint-keyed local admission coordinator shared by Alfredo processes. Establish the coordinator's storage and ownership scope explicitly even when terminals use different mission state directories. Normalize endpoint identity, define a safe policy for conflicting configured capacities, and keep file ownership, bounded state and owner-death recovery clear. External non-Alfredo clients remain outside this coordinator's guarantees.

Foreground conversation requests should receive bounded priority over queued background workers, with FIFO ordering within a class and protection against starvation. Cancellation while queued must remove eligibility before any HTTP dispatch; process exit must release capacity. A restored queue record must never authorize inference replay. Keep post-admission task/planner validation immediately before HTTP and keep inference admission separate from task approval.

Expose distinct application queue and upstream response phases. Preserve explicit retry, session/attempt isolation, shared capacity across discussion/planning/workers, and bounded deadlines. Do not claim latency improvements from scheduling tests alone.

## Required verification

Use deterministic two-process HTTP fixtures to prove aggregate capacity, foreground ordering with bounded background progress, queued cancellation with zero HTTP, scheduler/worker owner exit, conflicting configuration and restart without replay. Extend installed terminal coverage for concurrent foreground chat and workers. After correctness is established, use paired live cold/warm workloads with unchanged recorded endpoint/model/runtime settings before making performance claims.

This slice does not complete model/profile qualification, dynamic context, model-quality evaluation, full mission formation, retirement or production launch acceptance. Read the detailed Issue #69 contract and current provider/test seams before implementation; the selected coordinator design is recorded below.


## Implemented contract

Same-user Alfredo processes share capacity by normalized HTTP(S) URL origin under
`/tmp/alfredo-inference-<euid>/<sha256-origin>`, independent of mission state and
TMPDIR. Host aliases are deliberately distinct. Pure provider construction and
model discovery do not create scheduler state. The default remains two and the
configured range remains 1–8; differing live capacities refuse until all tickets
drain. Foreground chat/planning has at most three grants before an already-waiting
background worker, FIFO within each class, without preempting active requests.

A bounded private ledger and explicit owner locks coordinate eligibility. Queued
cancellation removes eligibility; dead owners are reaped without replay. Missing,
malformed or changed owner proof refuses new work. Removal publishes before dead
owner files are unlinked. The transaction lock never spans HTTP or an async wait.
The scheduler is for cooperating same-user clients; manual directory/lock
replacement while clients run is outside its guarantees. Namespace retirement
remains open, and releasing client capacity does not prove upstream abort.

Observed class, position, active slots and capacity are transient. Preparing,
Queued for Alfredo and Waiting for model server remain separate from model
thinking/answer streaming; stale attempts cannot revive queue observations.
Canonical task/conversation schemas and task authority are unchanged.

Verification: [shared admission evidence](../Reports/2026-09-26-shared-inference-admission.json). Final327 native tests pass / 7 ignored, worker cancel/grant regression RED/GREEN, strict Clippy and installed suites pass. Next: [native inference qualification](native-inference-qualification.md).
