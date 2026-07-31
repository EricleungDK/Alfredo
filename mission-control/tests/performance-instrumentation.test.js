import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import test from "node:test";

import {
  createPerformanceRecorder,
  performanceEnvironment,
} from "../scripts/performance-recorder.js";
import {
  compileStageMarks,
  readJsonLines,
} from "../scripts/performance-cohorts.js";
import { GATE_STEPS, verifyGateInputs } from "../scripts/performance-gates.js";

function metadata(path) {
  return {
    jsonl_path: path,
    run_id: "run-001",
    sample_id: "sample-001",
    cohort_id: "startup-process-cold",
    correlation_id: "startup-001",
    fixture_id: "minimal-ready-v1",
    fixture_sha256:
      "c4cef5ccc043bb6476e6e07195f979ea722e20abf3890c10d06de8ad1628839b",
    source_sha256:
      "a57b5956d8222b2ba001365fbff13e74350051ff13d5ba9dbe7fbeede203e721",
    artifact_sha256:
      "6e68b39f69f605a9218e74dd48f091460ccf15768b922d5b7dfda9ec1e84c2ac",
    variant: "python",
    workflow: "startup",
    mode: "process-cold",
  };
}

test("recorder appends process-local monotonic marks with immutable sample identity", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-performance-marks-"));
  const path = resolve(root, "raw.jsonl");
  const ticks = [1_000_000n, 2_500_000n];
  const recorder = createPerformanceRecorder({
    ...metadata(path),
    source: "launcher",
    clock_id: "launcher:123",
    monotonic_now_ns: () => ticks.shift(),
  });

  recorder.mark("S1", "start");
  recorder.mark("S1", "end", { desktop_kind: "native" });

  assert.deepEqual(readJsonLines(path), [
    {
      schema_version: 1,
      record_type: "stage-mark",
      run_id: "run-001",
      sample_id: "sample-001",
      cohort_id: "startup-process-cold",
      correlation_id: "startup-001",
      fixture_id: "minimal-ready-v1",
      fixture_sha256:
        "c4cef5ccc043bb6476e6e07195f979ea722e20abf3890c10d06de8ad1628839b",
      source_sha256:
        "a57b5956d8222b2ba001365fbff13e74350051ff13d5ba9dbe7fbeede203e721",
      artifact_sha256:
        "6e68b39f69f605a9218e74dd48f091460ccf15768b922d5b7dfda9ec1e84c2ac",
      variant: "python",
      workflow: "startup",
      mode: "process-cold",
      source: "launcher",
      clock_id: "launcher:123",
      stage: "S1",
      boundary: "start",
      monotonic_ns: "1000000",
      detail: {},
    },
    {
      schema_version: 1,
      record_type: "stage-mark",
      run_id: "run-001",
      sample_id: "sample-001",
      cohort_id: "startup-process-cold",
      correlation_id: "startup-001",
      fixture_id: "minimal-ready-v1",
      fixture_sha256:
        "c4cef5ccc043bb6476e6e07195f979ea722e20abf3890c10d06de8ad1628839b",
      source_sha256:
        "a57b5956d8222b2ba001365fbff13e74350051ff13d5ba9dbe7fbeede203e721",
      artifact_sha256:
        "6e68b39f69f605a9218e74dd48f091460ccf15768b922d5b7dfda9ec1e84c2ac",
      variant: "python",
      workflow: "startup",
      mode: "process-cold",
      source: "launcher",
      clock_id: "launcher:123",
      stage: "S1",
      boundary: "end",
      monotonic_ns: "2500000",
      detail: { desktop_kind: "native" },
    },
  ]);
});

test("repository correctness runner owns exactly the five fixed gates", () => {
  assert.deepEqual(Object.keys(GATE_STEPS), [
    "contract",
    "replay",
    "crash_cut",
    "packaging",
    "rollback",
  ]);
  assert.deepEqual(
    GATE_STEPS.packaging.map((step) => step.argv.join(" ")),
    [
      "npm run release:verify -- --artifact",
      "npm run release:verify",
      "npm run release:check",
    ],
  );
  assert.equal(GATE_STEPS.rollback.length, 2);
  assert.equal(GATE_STEPS.crash_cut[0].argv.slice(3).length, 4);
  assert.equal(GATE_STEPS.rollback[0].use_artifact_executable, true);
  assert.equal(GATE_STEPS.rollback[0].environment.ALFREDO_RUST_CANDIDATE_ENABLED, "0");
  assert.match(GATE_STEPS.rollback[1].fail_closed_reason, /prototype is not eligible/);
});

test("repository gate inputs must match the exact artifact and fixture bytes", async () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-gate-inputs-"));
  const artifact = resolve(root, "Alfredo.AppImage");
  const fixture = resolve(root, "fixture.json");
  writeFileSync(artifact, "artifact\n");
  chmodSync(artifact, 0o755);
  writeFileSync(fixture, "fixture\n");
  const digest = (value) =>
    createHash("sha256").update(value).digest("hex");
  const environment = {
    ALFREDO_GATE_ARTIFACT_PATH: artifact,
    ALFREDO_GATE_ARTIFACT_SHA256: digest("artifact\n"),
    ALFREDO_GATE_FIXTURE_PATH: fixture,
    ALFREDO_GATE_FIXTURE_SHA256: digest("fixture\n"),
  };

  const previous = Object.fromEntries(
    Object.keys(environment).map((key) => [key, process.env[key]]),
  );
  Object.assign(process.env, environment);
  try {
    assert.deepEqual(await verifyGateInputs(), {
      artifact_path: artifact,
      fixture_path: fixture,
    });
    process.env.ALFREDO_GATE_ARTIFACT_SHA256 = "0".repeat(64);
    await assert.rejects(() => verifyGateInputs(), /does not match/);
  } finally {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
});

test("environment contract is disabled only when absent and rejects partial identity", () => {
  assert.equal(performanceEnvironment({}), null);
  assert.throws(
    () =>
      performanceEnvironment({
        ALFREDO_MEASUREMENT_JSONL: "/tmp/raw.jsonl",
        ALFREDO_MEASUREMENT_RUN_ID: "run-only",
      }),
    /measurement environment is incomplete/,
  );
  assert.deepEqual(
    performanceEnvironment({
      ALFREDO_MEASUREMENT_JSONL: "/tmp/raw.jsonl",
      ALFREDO_MEASUREMENT_RUN_ID: "run-001",
      ALFREDO_MEASUREMENT_SAMPLE_ID: "sample-001",
      ALFREDO_MEASUREMENT_COHORT_ID: "startup-process-cold",
      ALFREDO_MEASUREMENT_CORRELATION_ID: "startup-001",
      ALFREDO_MEASUREMENT_FIXTURE_ID: "minimal-ready-v1",
      ALFREDO_MEASUREMENT_FIXTURE_SHA256:
        "c4cef5ccc043bb6476e6e07195f979ea722e20abf3890c10d06de8ad1628839b",
      ALFREDO_MEASUREMENT_SOURCE_SHA256:
        "a57b5956d8222b2ba001365fbff13e74350051ff13d5ba9dbe7fbeede203e721",
      ALFREDO_MEASUREMENT_ARTIFACT_SHA256:
        "6e68b39f69f605a9218e74dd48f091460ccf15768b922d5b7dfda9ec1e84c2ac",
      ALFREDO_MEASUREMENT_VARIANT: "python",
      ALFREDO_MEASUREMENT_WORKFLOW: "startup",
      ALFREDO_MEASUREMENT_MODE: "process-cold",
    }),
    metadata("/tmp/raw.jsonl"),
  );
});

test("environment contract accepts one trusted dynamic control file", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-performance-control-"));
  const controlPath = resolve(root, "control.json");
  writeFileSync(controlPath, JSON.stringify({
    ...metadata(resolve(root, "raw.jsonl")),
    desktop_pid: 4101,
    desktop_session_id: "desktop-one",
  }));

  assert.deepEqual(
    performanceEnvironment({ ALFREDO_MEASUREMENT_CONTROL_PATH: controlPath }),
    {
      ...metadata(resolve(root, "raw.jsonl")),
      desktop_pid: 4101,
      desktop_session_id: "desktop-one",
    },
  );
  assert.throws(
    () => performanceEnvironment({
      ALFREDO_MEASUREMENT_CONTROL_PATH: controlPath,
      ALFREDO_MEASUREMENT_RUN_ID: "ambiguous",
    }),
    /must not be combined/,
  );
});

test("compiler derives a stage duration only from one monotonic clock", () => {
  const base = {
    ...metadata("/tmp/unused.jsonl"),
    schema_version: 1,
    record_type: "stage-mark",
    source: "react",
    clock_id: "react:window-1",
    stage: "S8",
    detail: {},
  };
  delete base.jsonl_path;
  const compiled = compileStageMarks(
    [
      { ...base, boundary: "start", monotonic_ns: "1000000" },
      { ...base, boundary: "end", monotonic_ns: "4750000" },
    ],
    {
      metric: "S0->S8",
      duration_ms: 20,
      correctness: {
        contract: "pass",
        replay: "pass",
        crash_cut: "pass",
        packaging: "pass",
        rollback: "pass",
      },
    },
  );

  assert.equal(compiled.length, 1);
  assert.equal(compiled[0].stage_durations_ms.S8, 3.75);

  assert.throws(
    () =>
      compileStageMarks(
        [
          { ...base, boundary: "start", monotonic_ns: "1000000" },
          {
            ...base,
            boundary: "end",
            monotonic_ns: "4750000",
            clock_id: "react:window-2",
          },
        ],
        {
          metric: "S0->S8",
          duration_ms: 20,
          correctness: {
            contract: "pass",
            replay: "pass",
            crash_cut: "pass",
            packaging: "pass",
            rollback: "pass",
          },
        },
      ),
    /S8 start and end must share one monotonic clock/,
  );
});

test("recorder rejects unknown stages and backwards clocks", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-performance-invalid-mark-"));
  const ticks = [2n, 1n];
  const recorder = createPerformanceRecorder({
    ...metadata(resolve(root, "raw.jsonl")),
    source: "launcher",
    clock_id: "launcher:123",
    monotonic_now_ns: () => ticks.shift(),
  });

  assert.throws(() => recorder.mark("S10", "start"), /unknown measurement stage/);
  recorder.mark("S1", "start");
  assert.throws(() => recorder.mark("S1", "end"), /monotonic clock moved backwards/);
});

test("production launcher records S1 around its launch plan", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-performance-launcher-"));
  const rawPath = resolve(root, "raw.jsonl");
  const runtimeRoot = resolve(root, "runtime");
  const result = spawnSync(
    process.execPath,
    [resolve(import.meta.dirname, "../bin/alfredo.js"), "workstation", "--agent", "qwen3.6-27b"],
    {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        ALFREDO_DESKTOP_DRY_RUN: "launch",
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
        ALFREDO_STARTING_LOCATION: root,
        ALFREDO_MEASUREMENT_JSONL: rawPath,
        ALFREDO_MEASUREMENT_RUN_ID: "run-launcher",
        ALFREDO_MEASUREMENT_SAMPLE_ID: "sample-launcher",
        ALFREDO_MEASUREMENT_COHORT_ID: "startup-process-cold",
        ALFREDO_MEASUREMENT_CORRELATION_ID: "startup-launcher",
        ALFREDO_MEASUREMENT_FIXTURE_ID: "minimal-ready-v1",
        ALFREDO_MEASUREMENT_FIXTURE_SHA256:
          "c4cef5ccc043bb6476e6e07195f979ea722e20abf3890c10d06de8ad1628839b",
        ALFREDO_MEASUREMENT_SOURCE_SHA256:
          "a57b5956d8222b2ba001365fbff13e74350051ff13d5ba9dbe7fbeede203e721",
        ALFREDO_MEASUREMENT_ARTIFACT_SHA256:
          "6e68b39f69f605a9218e74dd48f091460ccf15768b922d5b7dfda9ec1e84c2ac",
        ALFREDO_MEASUREMENT_VARIANT: "python",
        ALFREDO_MEASUREMENT_WORKFLOW: "startup",
        ALFREDO_MEASUREMENT_MODE: "process-cold",
      },
    },
  );

  assert.equal(
    result.status,
    0,
    JSON.stringify({ stdout: result.stdout, stderr: result.stderr, error: result.error?.message }),
  );
  assert.deepEqual(
    readJsonLines(rawPath).map((record) => [record.stage, record.boundary, record.source]),
    [
      ["S1", "start", "launcher"],
      ["S1", "end", "launcher"],
    ],
  );
});
