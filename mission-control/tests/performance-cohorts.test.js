import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import test from "node:test";

import {
  compileStageMarks,
  loadFixtureFamily,
  PERFORMANCE_PHASE_FAMILIES,
  readJsonLines,
  summarizeCohorts,
  writeJsonLines,
} from "../scripts/performance-cohorts.js";

const fixtureManifest = resolve(
  import.meta.dirname,
  "../performance/fixtures/v1/manifest.json",
);

test("release evidence keeps workstation and model phase families separately attributable", () => {
  assert.deepEqual(PERFORMANCE_PHASE_FAMILIES, {
    launcher: ["S1"],
    desktop: ["S2", "S4"],
    react: ["S3", "S8", "S9", "R0", "R4", "R5", "R6"],
    backend: ["S5", "S6", "R2"],
    persistence: ["R2"],
    transport: ["S5", "S7", "R1", "R3"],
    rendered: ["S8", "S9", "R0", "R5", "R6"],
    model: ["load_ms", "prompt_evaluation_ms", "first_token_ms", "decoding_ms"],
  });
});

test("rendered metrics use one frontend clock from trusted R0 entry to the visible endpoint", () => {
  const identity = {
    schema_version: 1,
    record_type: "stage-mark",
    run_id: "rendered-clock-run",
    sample_id: "rendered-clock-sample",
    cohort_id: "queue-defer-process-warm",
    correlation_id: "rendered-clock-correlation",
    fixture_id: "pending-ad-hoc-v1",
    fixture_sha256: "a".repeat(64),
    source_sha256: "b".repeat(64),
    artifact_sha256: "c".repeat(64),
    variant: "python",
    workflow: "queue-defer",
    mode: "process-warm",
    source: "react",
    clock_id: "react:fixed-origin",
    detail: { outcome: "pass" },
  };
  const marks = [
    { ...identity, stage: "R0", boundary: "start", monotonic_ns: "1000000" },
    { ...identity, stage: "R0", boundary: "end", monotonic_ns: "2000000" },
    { ...identity, stage: "R5", boundary: "start", monotonic_ns: "7000000" },
    { ...identity, stage: "R5", boundary: "end", monotonic_ns: "11000000" },
  ];

  const [sample] = compileStageMarks(marks, { metric: "R0->R5" });

  assert.equal(sample.duration_ms, 10);
  assert.equal(sample.metric_clock_id, "react:fixed-origin");
  assert.throws(
    () =>
      compileStageMarks(
        marks.map((mark) =>
          mark.stage === "R5" ? { ...mark, clock_id: "react:different-origin" } : mark,
        ),
        { metric: "R0->R5" },
      ),
    /metric endpoints must share one monotonic clock/,
  );
});

test("compiler rejects a stage attributed to a process that does not own it", () => {
  const record = {
    schema_version: 1,
    record_type: "stage-mark",
    run_id: "forged-run",
    sample_id: "forged-sample",
    cohort_id: "queue-defer-process-warm",
    correlation_id: "forged-correlation",
    fixture_id: "pending-ad-hoc-v1",
    fixture_sha256: "a".repeat(64),
    source_sha256: "b".repeat(64),
    artifact_sha256: "c".repeat(64),
    variant: "python",
    workflow: "queue-defer",
    mode: "process-warm",
    source: "react",
    clock_id: "react:forged",
    stage: "R2",
    detail: { outcome: "pass" },
  };

  assert.throws(
    () => compileStageMarks([
      { ...record, boundary: "start", monotonic_ns: "1" },
      { ...record, boundary: "end", monotonic_ns: "2" },
    ], { metric: "R0->R5" }),
    /R2 must be recorded by python-authority/,
  );
});

function correctness(overrides = {}) {
  return {
    contract: "pass",
    replay: "pass",
    crash_cut: "pass",
    packaging: "pass",
    rollback: "pass",
    ...overrides,
  };
}

function sample({
  sampleId,
  mode,
  variant,
  durationMs,
  pairId,
  evidence = correctness(),
}) {
  return {
    schema_version: 1,
    record_type: "sample",
    cohort_id: `startup-${mode}`,
    sample_id: sampleId,
    pair_id: pairId,
    correlation_id: `measure-${sampleId}`,
    fixture_id: "minimal-ready-v1",
    fixture_byte_count: 485,
    fixture_sha256:
      "c0b82fbde62f89b687cd0b47d31ba644ad57d399813082bd1ad9e6c2da1cbbcf",
    source_revision: "1".repeat(40),
    source_sha256:
      "a57b5956d8222b2ba001365fbff13e74350051ff13d5ba9dbe7fbeede203e721",
    artifact_sha256:
      "6e68b39f69f605a9218e74dd48f091460ccf15768b922d5b7dfda9ec1e84c2ac",
    variant,
    workflow: "startup",
    mode,
    metric: "S0->S8",
    duration_ms: durationMs,
    stage_durations_ms: {
      S0: durationMs,
      S1: 2,
      S2: 4,
      S3: 3,
      S4: 1,
      S5: 8,
      S6: 6,
      S7: 2,
      S8: 5,
      S9: 7,
    },
    correctness: evidence,
    correctness_evidence: Object.fromEntries(
      ["contract", "replay", "crash_cut", "packaging", "rollback"].map((gate) => [
        gate,
        {
          evidence_path: `/evidence/${gate}.json`,
          sha256:
            "06f7aa7de78c2be55b924980b33417db16bec32435c2d8dbbd4563143fdc423a",
        },
      ]),
    ),
    valid: Object.values(evidence).every((value) => value === "pass"),
    invalid_reason: Object.values(evidence).every((value) => value === "pass")
      ? ""
      : "correctness evidence failed",
    execution_failure: "",
  };
}

test("fixture templates pin workload identities and bytes without stale build provenance", () => {
  const family = loadFixtureFamily(fixtureManifest);

  assert.equal(family.schema_version, 1);
  assert.deepEqual(
    family.fixtures.map((fixture) => fixture.kind),
    ["minimal-ready", "representative-warm", "pending-ad-hoc"],
  );
  for (const fixture of family.fixtures) {
    assert.match(fixture.fixture_id, /^[a-z0-9-]+-v1$/);
    assert.ok(fixture.byte_count > 0);
    assert.match(fixture.sha256, /^[a-f0-9]{64}$/);
    assert.equal("correctness" in fixture, false);
    const payload = JSON.parse(readFileSync(fixture.path, "utf8"));
    assert.equal("source_sha256" in payload, false);
    assert.equal("artifact_sha256" in payload, false);
    assert.equal("correctness" in payload, false);
  }
});

test("JSON Lines evidence round-trips without changing raw sample order", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-performance-jsonl-"));
  const path = resolve(root, "raw.jsonl");
  const records = [
    sample({
      sampleId: "cold-a-001",
      mode: "process-cold",
      variant: "python",
      durationMs: 100,
      pairId: "cold-001",
    }),
    sample({
      sampleId: "cold-b-001",
      mode: "process-cold",
      variant: "candidate",
      durationMs: 80,
      pairId: "cold-001",
    }),
  ];

  writeJsonLines(path, records);

  assert.deepEqual(readJsonLines(path), records);
  assert.equal(readFileSync(path, "utf8").split("\n").filter(Boolean).length, 2);
});

test("cohort report uses nearest-rank tails and paired deltas after required repeats", () => {
  const records = [];
  for (let index = 1; index <= 30; index += 1) {
    const pairId = `cold-${String(index).padStart(3, "0")}`;
    records.push(
      sample({
        sampleId: `${pairId}-python`,
        mode: "process-cold",
        variant: "python",
        durationMs: index + 100,
        pairId,
      }),
      sample({
        sampleId: `${pairId}-candidate`,
        mode: "process-cold",
        variant: "candidate",
        durationMs: index + 95,
        pairId,
      }),
    );
  }
  for (let index = 1; index <= 100; index += 1) {
    const pairId = `warm-${String(index).padStart(3, "0")}`;
    records.push(
      sample({
        sampleId: `${pairId}-python`,
        mode: "process-warm",
        variant: "python",
        durationMs: index + 100,
        pairId,
      }),
      sample({
        sampleId: `${pairId}-candidate`,
        mode: "process-warm",
        variant: "candidate",
        durationMs: index + 90,
        pairId,
      }),
    );
  }

  const report = summarizeCohorts(records, {
    baseline_variant: "python",
    candidate_variant: "candidate",
    minimum_improvement_ms: 1,
  });
  const coldBaseline = report.cohorts.find(
    (cohort) => cohort.mode === "process-cold" && cohort.variant === "python",
  );
  const warmCandidate = report.cohorts.find(
    (cohort) => cohort.mode === "process-warm" && cohort.variant === "candidate",
  );

  assert.deepEqual(
    {
      sample_count: coldBaseline.sample_count,
      p50_ms: coldBaseline.p50_ms,
      p95_ms: coldBaseline.p95_ms,
      min_ms: coldBaseline.min_ms,
      max_ms: coldBaseline.max_ms,
      failures: coldBaseline.failures,
    },
    {
      sample_count: 30,
      p50_ms: 115,
      p95_ms: 129,
      min_ms: 101,
      max_ms: 130,
      failures: 0,
    },
  );
  assert.equal(warmCandidate.sample_count, 100);
  assert.equal(warmCandidate.p95_ms, 185);
  assert.deepEqual(Object.keys(coldBaseline.phase_statistics), [
    "launcher",
    "desktop",
    "react",
    "backend",
    "persistence",
    "transport",
    "rendered",
    "model",
  ]);
  assert.deepEqual(coldBaseline.phase_statistics.launcher.S1, {
    sample_count: 30,
    p50_ms: 2,
    p95_ms: 2,
    min_ms: 2,
    max_ms: 2,
  });
  assert.deepEqual(coldBaseline.phase_statistics.backend.S6, {
    sample_count: 30,
    p50_ms: 6,
    p95_ms: 6,
    min_ms: 6,
    max_ms: 6,
  });
  assert.deepEqual(coldBaseline.phase_statistics.model.load_ms, {
    sample_count: 0,
    p50_ms: null,
    p95_ms: null,
    min_ms: null,
    max_ms: null,
  });
  assert.deepEqual(
    report.paired_deltas.map((item) => ({
      mode: item.mode,
      sample_count: item.sample_count,
      p50_ms: item.p50_ms,
      p95_ms: item.p95_ms,
    })),
    [
      { mode: "process-cold", sample_count: 30, p50_ms: -5, p95_ms: -5 },
      { mode: "process-warm", sample_count: 100, p50_ms: -10, p95_ms: -10 },
    ],
  );
  assert.equal(report.speed_claim_eligible, true);
});

test("failed correctness invalidates latency and blocks a speed claim", () => {
  const records = [
    sample({
      sampleId: "cold-python",
      mode: "process-cold",
      variant: "python",
      durationMs: 100,
      pairId: "cold-001",
    }),
    sample({
      sampleId: "cold-candidate",
      mode: "process-cold",
      variant: "candidate",
      durationMs: 1,
      pairId: "cold-001",
      evidence: correctness({ rollback: "fail" }),
    }),
  ];

  const report = summarizeCohorts(records, {
    baseline_variant: "python",
    candidate_variant: "candidate",
    minimum_improvement_ms: 1,
  });
  const candidate = report.cohorts.find((cohort) => cohort.variant === "candidate");

  assert.equal(candidate.sample_count, 0);
  assert.equal(candidate.failures, 1);
  assert.equal(candidate.p50_ms, null);
  assert.equal(report.paired_deltas.length, 0);
  assert.equal(report.speed_claim_eligible, false);
  assert.ok(
    report.ineligibility_reasons.includes(
      "candidate/process-cold/startup/S0->S8 has invalid correctness evidence",
    ),
  );
});

test("missing provenance or an execution failure invalidates an otherwise fast sample", () => {
  const missingEvidence = sample({
    sampleId: "cold-missing-evidence",
    mode: "process-cold",
    variant: "candidate",
    durationMs: 1,
    pairId: "cold-001",
  });
  delete missingEvidence.correctness_evidence.packaging;
  const failedExecution = {
    ...sample({
      sampleId: "cold-failed-execution",
      mode: "process-cold",
      variant: "python",
      durationMs: 1,
      pairId: "cold-001",
    }),
    execution_failure: "desktop exited before S9",
    valid: false,
    invalid_reason: "desktop exited before S9",
  };

  const report = summarizeCohorts([missingEvidence, failedExecution], {
    baseline_variant: "python",
    candidate_variant: "candidate",
    minimum_improvement_ms: 1,
  });

  assert.ok(report.cohorts.every((cohort) => cohort.sample_count === 0));
  assert.equal(report.speed_claim_eligible, false);
  const failureReasons = report.cohorts.flatMap((cohort) =>
    cohort.failure_records.flatMap((failure) => failure.reasons),
  );
  assert.ok(failureReasons.includes("correctness_evidence.packaging is missing"));
  assert.ok(failureReasons.includes("execution failed: desktop exited before S9"));
});

test("malformed raw evidence is rejected instead of silently repaired", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-performance-invalid-"));
  const path = resolve(root, "raw.jsonl");
  writeFileSync(path, '{"schema_version":1}\nnot-json\n', "utf8");

  assert.throws(() => readJsonLines(path), /raw JSON Lines record 2 is invalid/);
});
