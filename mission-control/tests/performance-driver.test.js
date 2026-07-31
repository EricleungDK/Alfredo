import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import test from "node:test";

import {
  createRandomizedPairSchedule,
  executeProductionSample,
  prepareProductionPlan,
  recordCorrectnessGate,
  runCohortPlan,
} from "../scripts/performance-driver.js";

const SHA_A = "a".repeat(64);
const SHA_B = "b".repeat(64);
const SHA_C = "c".repeat(64);

function correctnessEvidence(variant, cohortId) {
  return {
    variant,
    cohort_id: cohortId,
    gates: {
      contract: { status: "pass", evidence_path: "/evidence/contract.json", sha256: SHA_A },
      replay: { status: "pass", evidence_path: "/evidence/replay.json", sha256: SHA_A },
      crash_cut: { status: "pass", evidence_path: "/evidence/crash-cut.json", sha256: SHA_A },
      packaging: { status: "pass", evidence_path: "/evidence/packaging.json", sha256: SHA_A },
      rollback: { status: "pass", evidence_path: "/evidence/rollback.json", sha256: SHA_A },
    },
  };
}

function sampleFor(request) {
  const stageNames =
    request.workflow === "startup"
      ? ["S0", "S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9"]
      : request.workflow === "queue-defer"
        ? ["R0", "R1", "R2", "R3", "R4", "R5"]
        : ["R0", "R1", "R2", "R3", "R4", "R5", "R6"];
  return {
    schema_version: 1,
    record_type: "sample",
    cohort_id: request.cohort_id,
    sample_id: request.sample_id,
    pair_id: request.pair_id,
    correlation_id: request.correlation_id,
    fixture_id: request.fixture.fixture_id,
    fixture_sha256: request.fixture.sha256,
    fixture_byte_count: request.fixture.byte_count,
    source_revision: request.variant.source_revision,
    source_sha256: request.variant.source_sha256,
    artifact_sha256: request.variant.artifact_sha256,
    variant: request.variant.name,
    workflow: request.workflow,
    mode: request.mode,
    metric: request.metric,
    duration_ms: 10,
    stage_durations_ms: Object.fromEntries(stageNames.map((stage) => [stage, 1])),
    ...(request.desktop_pid
      ? {
          desktop_pid: request.desktop_pid,
          desktop_session_id: request.desktop_session_id,
        }
      : {}),
  };
}

function plan() {
  const cohorts = [
    {
      cohort_id: "startup-process-cold",
      fixture_id: "minimal-ready-v1",
      workflow: "startup",
      mode: "process-cold",
      metric: "S0->S8",
      sample_count: 30,
    },
    {
      cohort_id: "startup-hydrated-process-cold",
      fixture_id: "representative-warm-v1",
      workflow: "startup",
      mode: "process-cold",
      metric: "S0->S9",
      sample_count: 30,
    },
    {
      cohort_id: "queue-defer-process-warm",
      fixture_id: "pending-ad-hoc-v1",
      workflow: "queue-defer",
      mode: "process-warm",
      metric: "R0->R5",
      sample_count: 100,
    },
    {
      cohort_id: "queue-approve-visible-process-warm",
      fixture_id: "pending-ad-hoc-v1",
      workflow: "queue-approve",
      mode: "process-warm",
      metric: "R0->R5",
      sample_count: 100,
    },
    {
      cohort_id: "queue-approve-claim-process-warm",
      fixture_id: "pending-ad-hoc-v1",
      workflow: "queue-approve",
      mode: "process-warm",
      metric: "R0->R6",
      sample_count: 100,
    },
  ];
  return {
    schema_version: 1,
    run_id: "production-cohort-001",
    minimum_product_improvement_ms: 1,
    baseline_variant: "python",
    candidate_variant: "candidate",
    raw_jsonl: resolve("/tmp/alfredo-performance-raw.jsonl"),
    fixture_family: {
      fixtures: [
        {
          fixture_id: "minimal-ready-v1",
          kind: "minimal-ready",
          path: "/fixtures/minimal-ready.json",
          byte_count: 500,
          sha256: SHA_C,
        },
        {
          fixture_id: "representative-warm-v1",
          kind: "representative-warm",
          path: "/fixtures/representative-warm.json",
          byte_count: 650,
          sha256: SHA_A,
        },
        {
          fixture_id: "pending-ad-hoc-v1",
          kind: "pending-ad-hoc",
          path: "/fixtures/pending-ad-hoc.json",
          byte_count: 800,
          sha256: SHA_B,
        },
      ],
    },
    variants: [
      {
        name: "python",
        source_revision: "1".repeat(40),
        source_sha256: SHA_A,
        artifact_sha256: SHA_B,
      },
      {
        name: "candidate",
        source_revision: "1".repeat(40),
        source_sha256: SHA_A,
        artifact_sha256: SHA_C,
      },
    ],
    cohorts,
    correctness_evidence: cohorts.flatMap((cohort) => [
      correctnessEvidence("python", cohort.cohort_id),
      correctnessEvidence("candidate", cohort.cohort_id),
    ]),
  };
}

test("cohort schedule enforces accepted sample counts and balances randomized AB/BA pairs", () => {
  assert.throws(
    () =>
      createRandomizedPairSchedule({
        mode: "process-cold",
        sample_count: 29,
        baseline_variant: "python",
        candidate_variant: "candidate",
      }),
    /process-cold requires at least 30 pairs/,
  );

  const randomValues = [0.9, 0.1, 0.7, 0.2, 0.8, 0.3];
  const schedule = createRandomizedPairSchedule({
    mode: "process-cold",
    sample_count: 30,
    baseline_variant: "python",
    candidate_variant: "candidate",
    random: () => randomValues.shift() ?? 0.5,
  });
  const orientations = schedule.map((pair) => pair.variants.join(""));

  assert.equal(schedule.length, 30);
  assert.equal(orientations.filter((value) => value === "pythoncandidate").length, 15);
  assert.equal(orientations.filter((value) => value === "candidatepython").length, 15);
  assert.ok(schedule.every((pair) => new Set(pair.variants).size === 2));
});

test("production cohort plan rejects an incomplete product metric matrix", async () => {
  const incomplete = plan();
  incomplete.cohorts = incomplete.cohorts.slice(0, 2);
  incomplete.correctness_evidence = incomplete.correctness_evidence.filter((record) =>
    incomplete.cohorts.some((cohort) => cohort.cohort_id === record.cohort_id),
  );
  await assert.rejects(
    () => runCohortPlan(incomplete, { executeSample: async (request) => sampleFor(request) }),
    /production-equivalent S0->S8/,
  );
});

test("outer cohort runner preserves exact identity, raw order, evidence, and paired execution", async () => {
  const requests = [];
  const lifecycleEvents = [];
  const result = await runCohortPlan(plan(), {
    random: () => 0.25,
    lifecycle: {
      async startWarmSession({ variant }) {
        lifecycleEvents.push(`start:${variant.name}`);
        return {
          desktop_pid: variant.name === "python" ? 4101 : 4102,
          desktop_session_id: `${variant.name}-warm-desktop`,
        };
      },
      async prepareSample(request, session) {
        lifecycleEvents.push(`prepare:${request.sample_id}`);
        return {
          ...(session ?? {}),
          fixture_proof: {
            fixture_id: request.fixture.fixture_id,
            install_root: `/scratch/${request.variant.name}`,
            file_count: 3,
            byte_count: request.fixture.byte_count,
            canonical_tree_sha256: request.fixture.sha256,
          },
        };
      },
      async activateWarmSession(session) {
        lifecycleEvents.push(`activate:${session.desktop_session_id}`);
      },
      async deactivateWarmSession(session) {
        lifecycleEvents.push(`deactivate:${session.desktop_session_id}`);
      },
      async stopWarmSession(session) {
        lifecycleEvents.push(`stop:${session.desktop_session_id}`);
      },
    },
    executeSample: async (request) => {
      requests.push(request);
      return sampleFor(request);
    },
  });

  assert.equal(requests.length, 720);
  assert.equal(result.samples.length, 720);
  assert.equal(result.fixture_proofs.length, 10);
  assert.equal(result.sample_fixture_proofs.length, 720);
  assert.equal(result.sample_fixture_proofs[0].sample_id, result.samples[0].sample_id);
  assert.deepEqual(
    requests.slice(0, 2).map((request) => request.variant.name),
    ["python", "candidate"],
  );
  assert.equal(result.samples[0].fixture_byte_count, 500);
  assert.equal(result.samples[0].source_revision, "1".repeat(40));
  assert.equal(result.samples[0].correctness.packaging, "pass");
  assert.equal(result.samples[0].correctness_evidence.packaging.sha256, SHA_A);
  assert.equal(result.samples[0].pair_id, result.samples[1].pair_id);
  assert.equal(lifecycleEvents.filter((event) => event.startsWith("start:")).length, 6);
  assert.equal(lifecycleEvents.filter((event) => event.startsWith("prepare:")).length, 720);
  assert.equal(lifecycleEvents.filter((event) => event.startsWith("stop:")).length, 6);
  assert.equal(lifecycleEvents.filter((event) => event.startsWith("activate:")).length, 600);
  assert.equal(lifecycleEvents.filter((event) => event.startsWith("deactivate:")).length, 606);
  const warmRequests = requests.filter((request) => request.mode === "process-warm");
  assert.ok(warmRequests.every((request) => request.desktop_session_id));
  assert.equal(new Set(warmRequests.filter((request) => request.variant.name === "python").map((request) => request.desktop_pid)).size, 1);
});

test("failed correctness evidence is retained and makes the associated sample invalid", async () => {
  const invalid = plan();
  invalid.correctness_evidence = invalid.correctness_evidence
    .map((record) =>
      record.variant === "candidate" && record.cohort_id === "startup-process-cold"
        ? {
            ...record,
            gates: {
              ...record.gates,
              rollback: { ...record.gates.rollback, status: "fail" },
            },
          }
        : record,
    );

  const result = await runCohortPlan(invalid, {
    executeSample: async (request) => sampleFor(request),
    random: () => 0.75,
  });
  const candidate = result.samples.find((sample) => sample.variant === "candidate");

  assert.equal(candidate.correctness.rollback, "fail");
  assert.equal(candidate.valid, false);
  assert.match(candidate.invalid_reason, /rollback evidence failed/);
});

test("production preflight binds a clean committed source, exact artifact, and hashed gate evidence", async () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-performance-driver-"));
  const sourceRoot = resolve(root, "source");
  const artifactPath = resolve(root, "Alfredo.AppImage");
  const fixtureManifest = resolve(
    import.meta.dirname,
    "../performance/fixtures/v1/manifest.json",
  );
  spawnSync("mkdir", ["-p", sourceRoot], { encoding: "utf8" });
  writeFileSync(resolve(sourceRoot, "source.txt"), "exact source\n", "utf8");
  const gateRunnerPath = resolve(sourceRoot, "mission-control/scripts/performance-gates.js");
  spawnSync("mkdir", ["-p", resolve(sourceRoot, "mission-control/scripts")], {
    encoding: "utf8",
  });
  writeFileSync(gateRunnerPath, "// fixed gate runner\n", "utf8");
  for (const args of [
    ["init", "-q"],
    ["config", "user.email", "performance@example.invalid"],
    ["config", "user.name", "Performance Test"],
    ["add", "source.txt", "mission-control/scripts/performance-gates.js"],
    ["commit", "-qm", "fixture source"],
  ]) {
    const result = spawnSync("git", args, { cwd: sourceRoot, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
  }
  const revision = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: sourceRoot,
    encoding: "utf8",
  }).stdout.trim();
  writeFileSync(artifactPath, "exact packaged desktop\n", "utf8");
  chmodSync(artifactPath, 0o755);
  const sourceArchive = spawnSync("git", ["archive", "--format=tar", revision], {
    cwd: sourceRoot,
  }).stdout;
  const sourceSha = createHash("sha256").update(sourceArchive).digest("hex");
  const artifactSha = createHash("sha256")
    .update("exact packaged desktop\n")
    .digest("hex");
  const gateRunnerSha = createHash("sha256")
    .update("// fixed gate runner\n")
    .digest("hex");
  const cohorts = [
    { cohort_id: "startup-process-cold", fixture_id: "minimal-ready-v1", workflow: "startup", mode: "process-cold", metric: "S0->S8", sample_count: 30 },
    { cohort_id: "startup-hydrated-process-cold", fixture_id: "representative-warm-v1", workflow: "startup", mode: "process-cold", metric: "S0->S9", sample_count: 30 },
    { cohort_id: "queue-defer-process-warm", fixture_id: "pending-ad-hoc-v1", workflow: "queue-defer", mode: "process-warm", metric: "R0->R5", sample_count: 100 },
    { cohort_id: "queue-approve-visible-process-warm", fixture_id: "pending-ad-hoc-v1", workflow: "queue-approve", mode: "process-warm", metric: "R0->R5", sample_count: 100 },
    { cohort_id: "queue-approve-claim-process-warm", fixture_id: "pending-ad-hoc-v1", workflow: "queue-approve", mode: "process-warm", metric: "R0->R6", sample_count: 100 },
  ];
  const gateRecord = (variant, cohort) => {
    const fixtureSha = {
      "minimal-ready-v1": "ef1aaced12f47ffcce312e7228ce76f48be04214c7c2ceba8cecc8fdc966b616",
      "representative-warm-v1": "8b39b89256eb61c769cd18d6c7d112bbcee8bddfbd760e527c8437e673dab179",
      "pending-ad-hoc-v1": "8ab2c8b20fb1542551d80c6d6a691ebfd99e87ab183ec6e7e18f10cdab152822",
    }[cohort.fixture_id];
    const gates = {};
    for (const gate of ["contract", "replay", "crash_cut", "packaging", "rollback"]) {
      const evidencePath = resolve(root, `${variant}-${cohort.cohort_id}-${gate}.json`);
      const payload = {
        schema_version: 1,
        record_type: "correctness-gate",
        run_id: "exact-production-run",
        variant,
        cohort_id: cohort.cohort_id,
        fixture_id: cohort.fixture_id,
        fixture_sha256: fixtureSha,
        source_revision: revision,
        source_sha256: sourceSha,
        artifact_sha256: artifactSha,
        gate_runner_path: gateRunnerPath,
        gate_runner_sha256: gateRunnerSha,
        gate,
        status: "pass",
      };
      const encoded = `${JSON.stringify(payload)}\n`;
      writeFileSync(evidencePath, encoded, "utf8");
      gates[gate] = {
        status: "pass",
        evidence_path: evidencePath,
        sha256: createHash("sha256").update(encoded).digest("hex"),
      };
    }
    return { variant, cohort_id: cohort.cohort_id, gates };
  };
  const variant = (name, suffix) => ({
    name,
    artifact_path: artifactPath,
    installed_launcher_path: process.execPath,
    commands: {
      startup: [process.execPath, "--version"],
      "queue-defer": [process.execPath, "--version"],
      "queue-approve": [process.execPath, "--version"],
    },
    warm_session: {
      launch_command: [process.execPath, "--version"],
      fixture_install_root: resolve(root, `${suffix}-fixture`),
      ready_marker_path: resolve(root, `${suffix}-ready.json`),
      control_path: resolve(root, `${suffix}-control.json`),
    },
  });

  const prepared = await prepareProductionPlan({
    schema_version: 1,
    run_id: "exact-production-run",
    minimum_product_improvement_ms: 1,
    baseline_variant: "python",
    candidate_variant: "candidate",
    raw_jsonl: resolve(root, "raw.jsonl"),
    scratch_root: root,
    fixture_manifest: fixtureManifest,
    source_root: sourceRoot,
    source_revision: revision,
    variants: [variant("python", "python"), variant("candidate", "candidate")],
    cohorts,
    correctness_evidence: cohorts.flatMap((cohort) => [
      gateRecord("python", cohort),
      gateRecord("candidate", cohort),
    ]),
  });

  assert.equal(prepared.variants[0].source_revision, revision);
  assert.match(prepared.variants[0].source_sha256, /^[a-f0-9]{64}$/);
  assert.equal(prepared.variants[0].artifact_sha256, prepared.variants[1].artifact_sha256);
  assert.equal(prepared.fixture_family.fixtures[0].fixture_id, "minimal-ready-v1");

  writeFileSync(resolve(sourceRoot, "dirty.txt"), "not committed\n", "utf8");
  await assert.rejects(
    () =>
      prepareProductionPlan({
        ...prepared,
        fixture_manifest: fixtureManifest,
        source_root: sourceRoot,
        source_revision: revision,
        variants: [
          {
            name: "python",
            artifact_path: artifactPath,
            installed_launcher_path: process.execPath,
            commands: { startup: [process.execPath, "--version"] },
          },
          {
            name: "candidate",
            artifact_path: artifactPath,
            installed_launcher_path: process.execPath,
            commands: { startup: [process.execPath, "--version"] },
          },
        ],
      }),
    /source worktree must be clean/,
  );
});

test("production sample executor owns S0 and waits through distinct usable and hydrated marks", async () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-performance-sample-"));
  const rawPath = resolve(root, "raw.jsonl");
  const harness = `
    const { appendFileSync } = require("node:fs");
    const stages = ["S1","S2","S3","S4","S5","S6","S7","S8","S9"];
    let tick = 1000000n;
    for (const stage of stages) {
      for (const boundary of ["start", "end"]) {
        const record = {
          schema_version: 1,
          record_type: "stage-mark",
          run_id: process.env.ALFREDO_MEASUREMENT_RUN_ID,
          sample_id: process.env.ALFREDO_MEASUREMENT_SAMPLE_ID,
          cohort_id: process.env.ALFREDO_MEASUREMENT_COHORT_ID,
          correlation_id: process.env.ALFREDO_MEASUREMENT_CORRELATION_ID,
          fixture_id: process.env.ALFREDO_MEASUREMENT_FIXTURE_ID,
          fixture_sha256: process.env.ALFREDO_MEASUREMENT_FIXTURE_SHA256,
          source_sha256: process.env.ALFREDO_MEASUREMENT_SOURCE_SHA256,
          artifact_sha256: process.env.ALFREDO_MEASUREMENT_ARTIFACT_SHA256,
          variant: process.env.ALFREDO_MEASUREMENT_VARIANT,
          workflow: process.env.ALFREDO_MEASUREMENT_WORKFLOW,
          mode: process.env.ALFREDO_MEASUREMENT_MODE,
          source: ({S1:"launcher",S2:"native-shell",S3:"react",S4:"native-shell",S5:"native-shell",S6:"python-authority",S7:"native-shell",S8:"react",S9:"react"})[stage],
          clock_id: "fake-production-desktop:1",
          stage,
          boundary,
          monotonic_ns: String(tick),
          detail: { outcome: "pass" },
        };
        appendFileSync(process.env.ALFREDO_MEASUREMENT_JSONL, JSON.stringify(record) + "\\n");
        tick += 1000000n;
      }
    }
  `;
  const request = {
    run_id: "production-executor-run",
    raw_jsonl: rawPath,
    cohort_id: "startup-process-cold",
    sample_id: "startup-process-cold-pair-0001-python",
    pair_id: "startup-process-cold-pair-0001",
    correlation_id: "production-executor-run-sample-1",
    fixture: {
      fixture_id: "minimal-ready-v1",
      kind: "minimal-ready",
      path: "/fixtures/minimal-ready.json",
      byte_count: 500,
      sha256: SHA_C,
    },
    variant: {
      name: "python",
      source_revision: "1".repeat(40),
      source_sha256: SHA_A,
      artifact_sha256: SHA_B,
      cwd: root,
      commands: { startup: [process.execPath, "-e", harness] },
    },
    workflow: "startup",
    mode: "process-cold",
    metric: "S0->S8",
  };

  const sample = await executeProductionSample(request);

  assert.equal(sample.execution_failure, "");
  assert.equal(sample.raw_mark_count, 20);
  assert.ok(sample.duration_ms >= 0);
  assert.ok(sample.stage_durations_ms.S0 >= 0);
  assert.equal(sample.stage_durations_ms.S8, 1);
  assert.equal(sample.stage_durations_ms.S9, 1);
});

test("rendered action helper cannot receive credentials or forge desktop marks", async () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-rendered-sample-"));
  const rawPath = resolve(root, "raw.jsonl");
  const environmentReportPath = resolve(root, "action-environment.json");
  const harness = `
    const { writeFileSync } = require("node:fs");
    const credentials = Object.keys(process.env).filter((name) =>
      name.startsWith("ALFREDO_MEASUREMENT_")
    );
    writeFileSync(process.env.ACTION_ENVIRONMENT_REPORT, JSON.stringify({credentials}));
  `;
  const sample = await executeProductionSample({
    run_id: "rendered-run", raw_jsonl: rawPath,
    cohort_id: "queue-defer-process-warm", sample_id: "rendered-sample",
    pair_id: "rendered-pair", correlation_id: "rendered-correlation",
    desktop_pid: 4101, desktop_session_id: "desktop-one",
    fixture: { fixture_id: "pending-ad-hoc-v1", kind: "pending-ad-hoc", path: "/fixture", byte_count: 1, sha256: SHA_C },
    variant: { name: "python", source_revision: "1".repeat(40), source_sha256: SHA_A, artifact_sha256: SHA_B, cwd: root, environment: { ACTION_ENVIRONMENT_REPORT: environmentReportPath }, commands: { "queue-defer": [process.execPath, "-e", harness] } },
    workflow: "queue-defer", mode: "process-warm", metric: "R0->R5",
  });

  assert.match(sample.execution_failure, /exited before R5/);
  assert.equal(sample.raw_mark_count, 0);
  assert.deepEqual(JSON.parse(readFileSync(environmentReportPath, "utf8")), {
    credentials: [],
  });
});

test("correctness gate recorder binds command outcome to exact production provenance", async () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-performance-gate-"));
  const sourceRoot = resolve(root, "source");
  const artifactPath = resolve(root, "Alfredo.AppImage");
  const outputPath = resolve(root, "contract.json");
  spawnSync("mkdir", ["-p", sourceRoot], { encoding: "utf8" });
  writeFileSync(resolve(sourceRoot, "source.txt"), "gate source\n", "utf8");
  spawnSync("mkdir", ["-p", resolve(sourceRoot, "mission-control/scripts")], {
    encoding: "utf8",
  });
  const gateRunnerPath = resolve(
    sourceRoot,
    "mission-control/scripts/performance-gates.js",
  );
  writeFileSync(
    gateRunnerPath,
    "require('node:fs').writeFileSync(process.env.ALFREDO_GATE_RECEIPT_PATH, JSON.stringify({" +
      "gate:process.env.ALFREDO_GATE_NAME," +
      "status:'pass'," +
      "source_sha256:process.env.ALFREDO_GATE_SOURCE_SHA256," +
      "artifact_sha256:process.env.ALFREDO_GATE_ARTIFACT_SHA256," +
      "fixture_sha256:process.env.ALFREDO_GATE_FIXTURE_SHA256" +
      "}) + '\\n', {flag:'wx'})\n",
    "utf8",
  );
  chmodSync(gateRunnerPath, 0o755);
  for (const args of [
    ["init", "-q"],
    ["config", "user.email", "performance@example.invalid"],
    ["config", "user.name", "Performance Test"],
    ["add", "source.txt", "mission-control/scripts/performance-gates.js"],
    ["commit", "-qm", "gate source"],
  ]) {
    const result = spawnSync("git", args, { cwd: sourceRoot, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
  }
  const revision = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: sourceRoot,
    encoding: "utf8",
  }).stdout.trim();
  writeFileSync(artifactPath, "gate artifact\n", "utf8");
  chmodSync(artifactPath, 0o755);
  const gateSourceSha = createHash("sha256")
    .update(
      spawnSync("git", ["archive", "--format=tar", revision], {
        cwd: sourceRoot,
      }).stdout,
    )
    .digest("hex");
  const gateArtifactSha = createHash("sha256").update("gate artifact\n").digest("hex");
  const record = await recordCorrectnessGate({
    output_path: outputPath,
    run_id: "gate-run-001",
    variant: "python",
    cohort_id: "startup-process-cold",
    gate: "contract",
    source_root: sourceRoot,
    source_revision: revision,
    artifact_path: artifactPath,
    fixture_manifest: resolve(
      import.meta.dirname,
      "../performance/fixtures/v1/manifest.json",
    ),
    fixture_id: "minimal-ready-v1",
  });

  assert.equal(record.status, "pass", JSON.stringify(record));
  assert.equal(record.gate, "contract");
  assert.equal(record.command_exit_code, 0);
  assert.match(record.source_sha256, /^[a-f0-9]{64}$/);
  assert.match(record.artifact_sha256, /^[a-f0-9]{64}$/);
  assert.equal(record.gate_runner_path, gateRunnerPath);
  assert.match(record.gate_runner_sha256, /^[a-f0-9]{64}$/);
  assert.equal(JSON.parse(readFileSync(outputPath, "utf8")).status, "pass");
  await assert.rejects(
    () =>
      recordCorrectnessGate({
        output_path: resolve(root, "arbitrary-command.json"),
        run_id: "gate-run-001",
        variant: "python",
        cohort_id: "startup-process-cold",
        gate: "contract",
        source_root: sourceRoot,
        source_revision: revision,
        artifact_path: artifactPath,
        fixture_manifest: resolve(
          import.meta.dirname,
          "../performance/fixtures/v1/manifest.json",
        ),
        fixture_id: "minimal-ready-v1",
        command: ["/usr/bin/printf", "fake pass"],
      }),
    /does not accept an arbitrary correctness command/,
  );
});
