# Production performance cohorts

This directory defines Alfredo's production-equivalent startup and rendered-action
measurement contract. It does not contain an accepted performance result.

A valid run uses one clean committed source revision, exact packaged desktop
artifacts, byte-pinned workload fixtures, separately recorded correctness gates,
and sequential randomized AB/BA pairs. The driver refuses fewer than 30
process-cold pairs or 100 process-warm pairs. A failed contract, replay,
crash-cut, packaging, or rollback gate invalidates the associated samples and
blocks a speed claim.

The plan is a fixed five-cohort product matrix, not a menu: minimal startup
`S0→S8`, representative hydrated startup `S0→S9`, pending-Ad-Hoc defer
`R0→R5`, pending-Ad-Hoc approval `R0→R5`, and the same approval's separate
runner claim `R0→R6`. Missing, duplicate, substituted, or extra metrics fail
preflight.

Development Vite, jsdom, reducer microbenchmarks, the Rust shadow prototype, and
the historical backend-ready smoke marker are not production cohort substitutes.

## Recorded stages

Startup records S0 through S9:

- S0: outer installed-launch duration to the selected rendered endpoint.
- S1: npm launcher initialization and preflight.
- S2: native shell entry through frontend entry.
- S3: React entry through the two-frame boot paint.
- S4: snapshot invoke through Tauri handler entry.
- S5: Tauri handler through the correlated Python server acceptance handshake.
- S6: canonical Python snapshot authority.
- S7: native response decode and validation.
- S8: two-frame usable Prompt Workstation paint with exact workspace and Mission.
- S9: two-frame hydrated paint after capabilities, Console history, Working
  Context, Queue, and Shell each succeed or render an explicit failure.

Rendered Queue decisions record R0 through R6:

- R0: trusted intent through the rendered pending card.
- R1: invoke through Tauri handler entry.
- R2: locked authority transaction through its durable commit.
- R3: durable commit through typed React acknowledgement.
- R4: canonical snapshot reload.
- R5: two-frame canonical visible result.
- R6: a separate two-frame rendered claim backed by the durable
  `runner_started_at` field. General last activity is not runner-start evidence.

Queue defer ends at R5. Queue approval retains R5 as the visible-result metric
and waits separately for R6 before the sample is complete.

## Fixture templates

Validate the committed workload bytes:

```bash
cd /mnt/c/Users/ericl/Documents/AI-projects/local-coding-agent/mission-control
npm run performance:fixtures
```

`fixtures/v1` contains the complete canonical UTF-8 file bundle, file modes,
stable domain identities, declared byte count, payload SHA-256, and canonical
tree SHA-256 for each workload. Before every sample the lifecycle driver removes
only that variant's bounded fixture directory, recreates every file with
create-only writes, initializes the fixture workspace as a Git repository, and
rereads the declared canonical files to prove byte identity. Generated
`workspace/.git` metadata is outside the canonical store hash. A
sample-qualified materialization proof is retained in raw evidence for every
variant execution. The pending-Ad-Hoc fixture includes the
real versioned Queue store and receipt consumed by the public workspace API; it
is not a workload descriptor.

Source, artifact, and correctness fields are bound later to the exact run in
`fixture-proof` records. This prevents an old AppImage digest or descriptive
gate result from masquerading as current build proof.

## Record correctness evidence

Create one immutable correctness-gate file for every gate, variant, and cohort.
The recorder binds its result to the clean Git archive, fixture bytes, packaged
executable, and SHA-256 of the committed repository gate runner. It accepts no
caller-supplied command. `scripts/performance-gates.js` materializes and exercises
the exact named fixture through the public snapshot transport, verifies its
canonical bytes remain unchanged, and owns the exact contract, replay,
four-boundary crash-cut, packaging, and rollback commands. Packaging must pass
both release verification steps.

A zero exit code alone is not a pass. The fixed runner writes a create-only
receipt to the recorder-owned path containing `gate`, `status: "pass"`,
`source_sha256`, `artifact_sha256`, and `fixture_sha256`, exactly matching the
trusted `ALFREDO_GATE_*` values. The recorder reopens, validates, hashes, and
removes that transient receipt before it writes the immutable gate evidence.
Before running a gate, the runner itself opens the exact regular non-symlink
artifact and fixture paths and recomputes both declared digests. Packaging first
runs the installed-artifact GUI verification, then the full production AppImage
build/install and release-manifest check.
Rollback probes the exact artifact with the Rust-candidate feature disabled but
then deliberately fails closed: the repository has no production Rust authority
or production rollback flag yet, and the narrower prototype is not eligible
evidence. Consequently no candidate latency sample can become valid until that
production seam and its no-conversion/no-data-loss test exist.

```bash
npm run performance:gate -- \
  --output /absolute/evidence/python-startup-contract.json \
  --run-id run-2026-07-30 \
  --variant python \
  --cohort startup-process-cold \
  --gate contract \
  --source-root /absolute/clean/source \
  --source-revision FULL_40_CHARACTER_GIT_SHA \
  --artifact /absolute/installed/Alfredo.AppImage \
  --fixture-manifest /absolute/source/mission-control/performance/fixtures/v1/manifest.json \
  --fixture minimal-ready-v1
```

Repeat with gate names `replay`, `crash_cut`, `packaging`, and `rollback`. A gate
failure is still preserved as a file with `status: "fail"`; do not delete or
rewrite it. Hash each evidence file with `sha256sum` and put that digest and
absolute path in the cohort plan.

## Cohort plan

The plan is JSON with schema version 1. It names:

- `run_id`, absolute new `raw_jsonl`, absolute existing `scratch_root`, baseline,
  and candidate;
- a positive, predeclared `minimum_product_improvement_ms`; a statistically
  complete but smaller or negative paired p50 cannot authorize a speed claim;
- one absolute clean `source_root` and its full `source_revision`;
- the absolute fixture manifest;
- each variant's exact executable `artifact_path`, exact
  `installed_launcher_path`, workflow argv, working directory, and optional
  timeout/environment;
- for every process-warm variant, `warm_session.launch_command` using that exact
  installed launcher plus distinct absolute `fixture_install_root`,
  `ready_marker_path`, and `control_path` values inside `scratch_root`;
- cold or warm cohorts with fixture, workflow, metric, and required pair count;
- the five hashed correctness-gate records for every variant/cohort identity.

The five cohorts must be exactly the matrix listed above. `S0→S8` uses
`minimal-ready-v1`, `S0→S9` uses `representative-warm-v1`, and all three Queue
cohorts use `pending-ad-hoc-v1`.

Startup command argv zero must equal the exact `installed_launcher_path`; the
preflight rejects a source launcher or substitute harness.
Rendered-action commands must drive a trusted event in the already production
installed desktop and emit the same R-stage identity. A command that substitutes
jsdom, Vite, a reducer, or a different UI contract makes the contract gate fail.
The action subprocess receives no `ALFREDO_MEASUREMENT_*` identity, control path,
or raw JSONL path; only the persistent desktop and its authorities can emit marks.
The evidence compiler also enforces the fixed owner for every stage and
monotonic ordering within every process-local clock.

For a warm cohort, the lifecycle starts one persistent desktop per variant and
requires a create-only native readiness marker containing its real PID and
predeclared desktop session id. It suspends both owned process groups, restores
the next variant's canonical fixture and measurement control, resumes only that
variant for its action, then suspends it before resuming the paired variant.
Thus every variant keeps one warm PID/session while only one can execute. Tauri and the
persistent Python authority reread that file for the action and include the
desktop PID/session in every mark. The compiler rejects missing or changing
desktop identity. R0 and R5/R6 must also come from the same frontend
`performance.now()` clock; action-driver process startup is outside the rendered
duration.

Example warm-session shape (the two variants require distinct paths):

```json
{
  "warm_session": {
    "launch_command": ["/absolute/installed/bin/alfredo", "workstation"],
    "fixture_install_root": "/absolute/scratch/python-fixture",
    "ready_marker_path": "/absolute/scratch/python-ready.json",
    "control_path": "/absolute/scratch/python-control.json",
    "timeout_ms": 120000
  }
}
```

Check all source, artifact, fixture, command, and evidence identities before
opening any desktop:

```bash
npm run performance:check -- --plan /absolute/cohort-plan.json
```

The check fails if the Git worktree is dirty, HEAD differs from the declared
revision, either artifact changes, a fixture changes, an evidence file changes,
or an evidence record belongs to another source/artifact/fixture/gate identity.

## Run and report

Run pairs sequentially, one variant at a time:

```bash
npm run performance:run -- \
  --plan /absolute/cohort-plan.json \
  --report /absolute/cohort-report.json
```

The raw JSON Lines file is new and append-only for one run. It retains process
marks, per-cohort and per-sample fixture proofs, exact sample identities, command outcomes, correctness
evidence digests, and invalid samples. The report includes raw and valid sample
counts, nearest-rank p50/p95, min/max, failures, paired deltas, and explicit
ineligibility reasons. Exit status 2 means the evidence was recorded but no
speed claim is eligible.

Do not call a run “cold boot.” `process-cold` means new product processes and an
isolated runtime/fixture copy; OS page cache remains uncontrolled.
