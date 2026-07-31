#!/usr/bin/env node
import { createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import {
  appendFileSync,
  createReadStream,
  lstatSync,
  readFileSync,
  realpathSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  compileStageMarks,
  CORRECTNESS_GATES,
  loadFixtureFamily,
  RENDERED_ACTION_STAGES,
  STARTUP_STAGES,
  summarizeCohorts,
} from "./performance-cohorts.js";
import { createPerformanceRecorder } from "./performance-recorder.js";
import { createProductionLifecycle } from "./performance-lifecycle.js";

export const MINIMUM_SAMPLE_PAIRS = Object.freeze({
  "process-cold": 30,
  "process-warm": 100,
});
const REQUIRED_COHORT_MATRIX = Object.freeze([
  Object.freeze({ workflow: "startup", mode: "process-cold", metric: "S0->S8", fixture_kind: "minimal-ready" }),
  Object.freeze({ workflow: "startup", mode: "process-cold", metric: "S0->S9", fixture_kind: "representative-warm" }),
  Object.freeze({ workflow: "queue-defer", mode: "process-warm", metric: "R0->R5", fixture_kind: "pending-ad-hoc" }),
  Object.freeze({ workflow: "queue-approve", mode: "process-warm", metric: "R0->R5", fixture_kind: "pending-ad-hoc" }),
  Object.freeze({ workflow: "queue-approve", mode: "process-warm", metric: "R0->R6", fixture_kind: "pending-ad-hoc" }),
]);

const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const SOURCE_REVISION_PATTERN = /^[a-f0-9]{40}$/;

function fail(message) {
  throw new Error(message);
}

function requiredString(value, label) {
  if (typeof value !== "string" || !value.trim()) fail(`${label} must be a non-empty string`);
  return value;
}

function requiredSha256(value, label) {
  requiredString(value, label);
  if (!SHA256_PATTERN.test(value)) fail(`${label} must be a lowercase SHA-256`);
  return value;
}

function requiredSourceRevision(value, label) {
  requiredString(value, label);
  if (!SOURCE_REVISION_PATTERN.test(value)) {
    fail(`${label} must be a full lowercase Git revision`);
  }
  return value;
}

function requiredPositiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) fail(`${label} must be a positive integer`);
  return value;
}

function checkedRegularFile(path, label, { executable = false } = {}) {
  requiredString(path, label);
  if (!isAbsolute(path)) fail(`${label} must be absolute`);
  const entry = lstatSync(path);
  if (!entry.isFile() || entry.isSymbolicLink()) {
    fail(`${label} must be a regular non-symlink file`);
  }
  if (executable && (entry.mode & 0o111) === 0) fail(`${label} must be executable`);
  return realpathSync(path);
}

function requiredScratchPath(scratchRoot, path, label) {
  requiredString(path, label);
  if (!isAbsolute(path)) fail(`${label} must be absolute`);
  const absolute = resolve(path);
  const suffix = relative(scratchRoot, absolute);
  if (!suffix || suffix.startsWith("..") || isAbsolute(suffix)) {
    fail(`${label} must stay inside scratch_root`);
  }
  return absolute;
}

export function validateWarmSessionConfig(rawConfig, {
  variantName,
  installedLauncherPath,
  scratchRoot,
}) {
  if (!rawConfig || typeof rawConfig !== "object" || Array.isArray(rawConfig)) {
    fail(`variant ${variantName}.warm_session is required for process-warm cohorts`);
  }
  if (!Array.isArray(rawConfig.launch_command) || rawConfig.launch_command.length === 0) {
    fail(`variant ${variantName}.warm_session.launch_command must be a non-empty argv array`);
  }
  const launcher = checkedRegularFile(
    rawConfig.launch_command[0],
    `variant ${variantName}.warm_session.launch_command[0]`,
    { executable: true },
  );
  if (launcher !== installedLauncherPath) {
    fail(`variant ${variantName} warm desktop must use the exact installed launcher`);
  }
  return {
    ...rawConfig,
    launch_command: [
      launcher,
      ...rawConfig.launch_command.slice(1).map((argument, index) =>
        requiredString(
          argument,
          `variant ${variantName}.warm_session.launch_command[${index + 1}]`,
        ),
      ),
    ],
    fixture_install_root: requiredScratchPath(
      scratchRoot,
      rawConfig.fixture_install_root,
      `variant ${variantName}.warm_session.fixture_install_root`,
    ),
    ready_marker_path: requiredScratchPath(
      scratchRoot,
      rawConfig.ready_marker_path,
      `variant ${variantName}.warm_session.ready_marker_path`,
    ),
    control_path: requiredScratchPath(
      scratchRoot,
      rawConfig.control_path,
      `variant ${variantName}.warm_session.control_path`,
    ),
    timeout_ms:
      rawConfig.timeout_ms === undefined
        ? 120_000
        : requiredPositiveInteger(rawConfig.timeout_ms, `variant ${variantName}.warm_session.timeout_ms`),
  };
}

function sha256File(path) {
  return new Promise((resolveHash, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(path);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("error", reject);
    stream.on("end", () => resolveHash(hash.digest("hex")));
  });
}

function gitOutput(sourceRoot, args, label) {
  const result = spawnSync("git", args, {
    cwd: sourceRoot,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.status !== 0) {
    fail(`${label} failed: ${(result.stderr || result.stdout).trim()}`);
  }
  return result.stdout.trim();
}

function sha256GitArchive(sourceRoot, revision) {
  return new Promise((resolveHash, reject) => {
    const child = spawn("git", ["archive", "--format=tar", revision], {
      cwd: sourceRoot,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const hash = createHash("sha256");
    let stderr = "";
    child.stdout.on("data", (chunk) => hash.update(chunk));
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
      if (stderr.length > 16_384) stderr = stderr.slice(-16_384);
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code !== 0) {
        reject(new Error(`source archive failed: ${stderr.trim()}`));
        return;
      }
      resolveHash(hash.digest("hex"));
    });
  });
}

function runCorrectnessCommand(command, options) {
  const result = spawnSync(command[0], command.slice(1), {
    cwd: options.cwd,
    env: options.env,
    encoding: "utf8",
    timeout: options.timeout_ms,
    maxBuffer: 16 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
  return Promise.resolve({
    status: result.status,
    signal: result.signal,
    error: result.error?.message ?? "",
    stdout: result.stdout ?? "",
    stderr: result.stderr ?? "",
  });
}

function requiredStages(workflow) {
  if (workflow === "startup") return STARTUP_STAGES;
  if (workflow === "queue-defer") return RENDERED_ACTION_STAGES.slice(0, 6);
  if (workflow === "queue-approve") return RENDERED_ACTION_STAGES;
  fail(`unsupported cohort workflow: ${workflow}`);
}

function shuffled(values, random) {
  const result = [...values];
  for (let index = result.length - 1; index > 0; index -= 1) {
    const selected = Math.floor(random() * (index + 1));
    if (!Number.isSafeInteger(selected) || selected < 0 || selected > index) {
      fail("random source must return a number from zero up to but not including one");
    }
    [result[index], result[selected]] = [result[selected], result[index]];
  }
  return result;
}

export function createRandomizedPairSchedule({
  mode,
  sample_count: sampleCount,
  baseline_variant: baselineVariant,
  candidate_variant: candidateVariant,
  random = Math.random,
}) {
  const minimum = MINIMUM_SAMPLE_PAIRS[mode];
  if (!minimum) fail("cohort mode must be process-cold or process-warm");
  if (!Number.isSafeInteger(sampleCount) || sampleCount < minimum) {
    fail(`${mode} requires at least ${minimum} pairs`);
  }
  requiredString(baselineVariant, "baseline_variant");
  requiredString(candidateVariant, "candidate_variant");
  if (baselineVariant === candidateVariant) fail("candidate_variant must differ from baseline_variant");
  if (typeof random !== "function") fail("random must be a function");

  const forwardCount = Math.floor(sampleCount / 2);
  const orientations = [
    ...Array.from({ length: forwardCount }, () => [baselineVariant, candidateVariant]),
    ...Array.from(
      { length: sampleCount - forwardCount },
      () => [candidateVariant, baselineVariant],
    ),
  ];
  return shuffled(orientations, random).map((variants, index) => ({
    pair_index: index + 1,
    variants,
  }));
}

function indexBy(items, key, label) {
  const result = new Map();
  for (const item of items) {
    const value = item?.[key];
    requiredString(value, `${label}.${key}`);
    if (result.has(value)) fail(`${label} repeats ${key} ${value}`);
    result.set(value, item);
  }
  return result;
}

function validatedVariant(variant) {
  const name = requiredString(variant?.name, "variant.name");
  return {
    ...variant,
    name,
    source_revision: requiredSourceRevision(
      variant.source_revision,
      `variant ${name}.source_revision`,
    ),
    source_sha256: requiredSha256(variant.source_sha256, `variant ${name}.source_sha256`),
    artifact_sha256: requiredSha256(
      variant.artifact_sha256,
      `variant ${name}.artifact_sha256`,
    ),
  };
}

function validatedFixture(fixture) {
  const fixtureId = requiredString(fixture?.fixture_id, "fixture.fixture_id");
  return {
    ...fixture,
    fixture_id: fixtureId,
    kind: requiredString(fixture.kind, `fixture ${fixtureId}.kind`),
    path: requiredString(fixture.path, `fixture ${fixtureId}.path`),
    byte_count: requiredPositiveInteger(
      fixture.byte_count,
      `fixture ${fixtureId}.byte_count`,
    ),
    sha256: requiredSha256(fixture.sha256, `fixture ${fixtureId}.sha256`),
  };
}

function validatedEvidence(record, variants, cohorts) {
  const variant = requiredString(record?.variant, "correctness evidence variant");
  const cohortId = requiredString(record?.cohort_id, "correctness evidence cohort_id");
  if (!variants.has(variant)) fail(`correctness evidence references unknown variant ${variant}`);
  if (!cohorts.has(cohortId)) fail(`correctness evidence references unknown cohort ${cohortId}`);
  if (!record.gates || typeof record.gates !== "object" || Array.isArray(record.gates)) {
    fail(`correctness evidence ${variant}/${cohortId} gates must be an object`);
  }
  const gates = {};
  for (const gate of CORRECTNESS_GATES) {
    const evidence = record.gates[gate];
    if (!evidence || typeof evidence !== "object" || Array.isArray(evidence)) {
      fail(`correctness evidence ${variant}/${cohortId}.${gate} must be an object`);
    }
    if (evidence.status !== "pass" && evidence.status !== "fail") {
      fail(`correctness evidence ${variant}/${cohortId}.${gate}.status must be pass or fail`);
    }
    gates[gate] = {
      status: evidence.status,
      evidence_path: requiredString(
        evidence.evidence_path,
        `correctness evidence ${variant}/${cohortId}.${gate}.evidence_path`,
      ),
      sha256: requiredSha256(
        evidence.sha256,
        `correctness evidence ${variant}/${cohortId}.${gate}.sha256`,
      ),
    };
  }
  return { variant, cohort_id: cohortId, gates };
}

function validatePlan(plan) {
  if (plan?.schema_version !== 1) fail("cohort plan schema_version must be 1");
  const runId = requiredString(plan.run_id, "run_id");
  const baselineVariant = requiredString(plan.baseline_variant, "baseline_variant");
  const candidateVariant = requiredString(plan.candidate_variant, "candidate_variant");
  if (baselineVariant === candidateVariant) fail("candidate_variant must differ from baseline_variant");
  requiredString(plan.raw_jsonl, "raw_jsonl");
  if (!Array.isArray(plan.variants)) fail("cohort plan variants must be an array");
  if (!Array.isArray(plan.cohorts) || plan.cohorts.length === 0) {
    fail("cohort plan cohorts must be a non-empty array");
  }
  if (!Array.isArray(plan.fixture_family?.fixtures)) {
    fail("cohort plan fixture_family.fixtures must be an array");
  }
  if (!Array.isArray(plan.correctness_evidence)) {
    fail("cohort plan correctness_evidence must be an array");
  }

  const variants = plan.variants.map(validatedVariant);
  const variantByName = indexBy(variants, "name", "variants");
  if (!variantByName.has(baselineVariant) || !variantByName.has(candidateVariant)) {
    fail("cohort plan must define both baseline_variant and candidate_variant");
  }
  const fixtures = plan.fixture_family.fixtures.map(validatedFixture);
  const fixtureById = indexBy(fixtures, "fixture_id", "fixtures");
  const cohorts = plan.cohorts.map((cohort) => {
    const cohortId = requiredString(cohort?.cohort_id, "cohort.cohort_id");
    const mode = requiredString(cohort.mode, `cohort ${cohortId}.mode`);
    const sampleCount = requiredPositiveInteger(
      cohort.sample_count,
      `cohort ${cohortId}.sample_count`,
    );
    createRandomizedPairSchedule({
      mode,
      sample_count: sampleCount,
      baseline_variant: baselineVariant,
      candidate_variant: candidateVariant,
      random: () => 0,
    });
    const fixtureId = requiredString(cohort.fixture_id, `cohort ${cohortId}.fixture_id`);
    if (!fixtureById.has(fixtureId)) fail(`cohort ${cohortId} references unknown fixture ${fixtureId}`);
    const workflow = requiredString(cohort.workflow, `cohort ${cohortId}.workflow`);
    requiredStages(workflow);
    return {
      ...cohort,
      cohort_id: cohortId,
      fixture_id: fixtureId,
      workflow,
      mode,
      metric: requiredString(cohort.metric, `cohort ${cohortId}.metric`),
      sample_count: sampleCount,
    };
  });
  const cohortSignatures = cohorts.map((cohort) => {
    const fixture = fixtureById.get(cohort.fixture_id);
    return `${cohort.workflow}\u0000${cohort.mode}\u0000${cohort.metric}\u0000${fixture.kind}`;
  });
  const requiredSignatures = REQUIRED_COHORT_MATRIX.map(
    (cohort) => `${cohort.workflow}\u0000${cohort.mode}\u0000${cohort.metric}\u0000${cohort.fixture_kind}`,
  );
  if (
    cohortSignatures.length !== requiredSignatures.length ||
    requiredSignatures.some(
      (signature) => cohortSignatures.filter((candidate) => candidate === signature).length !== 1,
    )
  ) {
    fail("cohort plan must contain exactly the production-equivalent S0->S8, S0->S9, Queue defer R0->R5, Queue approve R0->R5, and Queue approve R0->R6 matrix");
  }
  const cohortById = indexBy(cohorts, "cohort_id", "cohorts");
  const evidence = plan.correctness_evidence.map((record) =>
    validatedEvidence(record, variantByName, cohortById),
  );
  const evidenceByIdentity = new Map();
  for (const record of evidence) {
    const identity = `${record.variant}\u0000${record.cohort_id}`;
    if (evidenceByIdentity.has(identity)) {
      fail(`correctness evidence repeats ${record.variant}/${record.cohort_id}`);
    }
    evidenceByIdentity.set(identity, record);
  }
  for (const cohort of cohorts) {
    for (const variant of [baselineVariant, candidateVariant]) {
      if (!evidenceByIdentity.has(`${variant}\u0000${cohort.cohort_id}`)) {
        fail(`correctness evidence is missing for ${variant}/${cohort.cohort_id}`);
      }
    }
  }
  const sourceRevisions = new Set(variants.map((variant) => variant.source_revision));
  const sourceHashes = new Set(variants.map((variant) => variant.source_sha256));
  if (sourceRevisions.size !== 1 || sourceHashes.size !== 1) {
    fail("all compared variants must use the exact same committed source");
  }
  return {
    ...plan,
    run_id: runId,
    baseline_variant: baselineVariant,
    candidate_variant: candidateVariant,
    raw_jsonl: resolve(plan.raw_jsonl),
    variants,
    variantByName,
    fixtures,
    fixtureById,
    cohorts,
    evidence,
    evidenceByIdentity,
  };
}

export async function recordCorrectnessGate(options) {
  const gate = requiredString(options?.gate, "gate");
  if (!CORRECTNESS_GATES.includes(gate)) fail(`unknown correctness gate: ${gate}`);
  const outputPath = resolve(requiredString(options.output_path, "output_path"));
  ensureNewOutput(outputPath, "correctness gate output");
  const sourceRoot = realpathSync(requiredString(options.source_root, "source_root"));
  const sourceRevision = requiredSourceRevision(
    options.source_revision,
    "source_revision",
  );
  const headRevision = gitOutput(sourceRoot, ["rev-parse", "HEAD"], "source revision");
  if (headRevision !== sourceRevision) {
    fail(`source revision changed: expected ${sourceRevision}, got ${headRevision}`);
  }
  if (
    gitOutput(
      sourceRoot,
      ["status", "--porcelain=v1", "--untracked-files=all"],
      "source status",
    )
  ) {
    fail("source worktree must be clean before recording correctness evidence");
  }
  const sourceSha256 = await sha256GitArchive(sourceRoot, sourceRevision);
  const artifactPath = checkedRegularFile(
    options.artifact_path,
    "artifact_path",
    { executable: true },
  );
  const artifactSha256 = await sha256File(artifactPath);
  const fixtureFamily = loadFixtureFamily(
    checkedRegularFile(options.fixture_manifest, "fixture_manifest"),
  );
  const fixtureId = requiredString(options.fixture_id, "fixture_id");
  const fixture = fixtureFamily.fixtures.find(
    (candidate) => candidate.fixture_id === fixtureId,
  );
  if (!fixture) fail(`unknown fixture_id: ${fixtureId}`);
  if (options.command !== undefined) {
    fail("record-gate does not accept an arbitrary correctness command");
  }
  const gateRunnerPath = checkedRegularFile(
    resolve(sourceRoot, "mission-control/scripts/performance-gates.js"),
    "repository correctness gate runner",
  );
  const gateRunnerSha256 = await sha256File(gateRunnerPath);
  const command = [process.execPath, gateRunnerPath, gate];
  const runnerReceiptPath = `${outputPath}.runner-receipt-${process.pid}`;
  ensureNewOutput(runnerReceiptPath, "correctness runner receipt");
  const gateIdentity = {
    gate,
    source_sha256: sourceSha256,
    artifact_sha256: artifactSha256,
    fixture_sha256: fixture.sha256,
  };
  const commandResult = await runCorrectnessCommand(command, {
    cwd: options.cwd ? realpathSync(options.cwd) : sourceRoot,
    env: {
      ...process.env,
      ...(options.environment ?? {}),
      ALFREDO_GATE_NAME: gate,
      ALFREDO_GATE_SOURCE_ROOT: sourceRoot,
      ALFREDO_GATE_SOURCE_SHA256: sourceSha256,
      ALFREDO_GATE_ARTIFACT_SHA256: artifactSha256,
      ALFREDO_GATE_FIXTURE_SHA256: fixture.sha256,
      ALFREDO_GATE_FIXTURE_PATH: fixture.path,
      ALFREDO_GATE_ARTIFACT_PATH: artifactPath,
      ALFREDO_GATE_RECEIPT_PATH: runnerReceiptPath,
    },
    timeout_ms:
      options.timeout_ms === undefined
        ? 120_000
        : requiredPositiveInteger(options.timeout_ms, "timeout_ms"),
  });
  const stdout = commandResult.stdout ?? "";
  const stderr = commandResult.stderr ?? "";
  let receipt = null;
  let receiptFailure = "";
  let receiptBytes = "";
  if (!commandResult.error && commandResult.status === 0 && !commandResult.signal) {
    try {
      const checkedReceiptPath = checkedRegularFile(
        runnerReceiptPath,
        "correctness runner receipt",
      );
      receiptBytes = readFileSync(checkedReceiptPath, "utf8");
      receipt = JSON.parse(receiptBytes);
      for (const [field, value] of Object.entries({ ...gateIdentity, status: "pass" })) {
        if (receipt?.[field] !== value) {
          throw new Error(`receipt changes ${field}`);
        }
      }
    } catch (error) {
      receiptFailure = `correctness runner did not write an exact pass receipt: ${error.message}`;
    }
  }
  try {
    unlinkSync(runnerReceiptPath);
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  const status =
    !commandResult.error &&
    commandResult.status === 0 &&
    !commandResult.signal &&
    !receiptFailure
      ? "pass"
      : "fail";
  const record = {
    schema_version: 1,
    record_type: "correctness-gate",
    run_id: requiredString(options.run_id, "run_id"),
    variant: requiredString(options.variant, "variant"),
    cohort_id: requiredString(options.cohort_id, "cohort_id"),
    fixture_id: fixture.fixture_id,
    fixture_byte_count: fixture.byte_count,
    fixture_sha256: fixture.sha256,
    source_revision: sourceRevision,
    source_sha256: sourceSha256,
    artifact_path: artifactPath,
    artifact_sha256: artifactSha256,
    gate,
    status,
    command,
    gate_runner_path: gateRunnerPath,
    gate_runner_sha256: gateRunnerSha256,
    command_exit_code: commandResult.status,
    command_signal: commandResult.signal ?? "",
    command_error: commandResult.error ?? "",
    receipt_error: receiptFailure,
    receipt,
    receipt_sha256: receiptBytes
      ? createHash("sha256").update(receiptBytes).digest("hex")
      : "",
    stdout_sha256: createHash("sha256").update(stdout).digest("hex"),
    stderr_sha256: createHash("sha256").update(stderr).digest("hex"),
    stdout_tail: stdout.slice(-16_384),
    stderr_tail: stderr.slice(-16_384),
  };
  writeFileSync(outputPath, `${JSON.stringify(record, null, 2)}\n`, {
    encoding: "utf8",
    mode: 0o600,
    flag: "wx",
  });
  return record;
}

export async function prepareProductionPlan(rawPlan) {
  if (rawPlan?.schema_version !== 1) fail("cohort plan schema_version must be 1");
  if (
    typeof rawPlan.minimum_product_improvement_ms !== "number" ||
    !Number.isFinite(rawPlan.minimum_product_improvement_ms) ||
    rawPlan.minimum_product_improvement_ms <= 0
  ) {
    fail("minimum_product_improvement_ms must be a positive predeclared number");
  }
  requiredString(rawPlan.fixture_manifest, "fixture_manifest");
  if (!isAbsolute(rawPlan.fixture_manifest)) fail("fixture_manifest must be absolute");
  requiredString(rawPlan.source_root, "source_root");
  if (!isAbsolute(rawPlan.source_root)) fail("source_root must be absolute");
  const sourceRoot = realpathSync(rawPlan.source_root);
  requiredString(rawPlan.scratch_root, "scratch_root");
  if (!isAbsolute(rawPlan.scratch_root)) fail("scratch_root must be absolute");
  const scratchRoot = realpathSync(rawPlan.scratch_root);
  const expectedRevision = requiredSourceRevision(
    rawPlan.source_revision,
    "source_revision",
  );
  const headRevision = gitOutput(sourceRoot, ["rev-parse", "HEAD"], "source revision");
  if (headRevision !== expectedRevision) {
    fail(`source revision changed: expected ${expectedRevision}, got ${headRevision}`);
  }
  const dirty = gitOutput(
    sourceRoot,
    ["status", "--porcelain=v1", "--untracked-files=all"],
    "source status",
  );
  if (dirty) fail("source worktree must be clean before a production cohort");
  const sourceSha256 = await sha256GitArchive(sourceRoot, expectedRevision);
  const gateRunnerPath = checkedRegularFile(
    resolve(sourceRoot, "mission-control/scripts/performance-gates.js"),
    "repository correctness gate runner",
  );
  const gateRunnerSha256 = await sha256File(gateRunnerPath);
  const fixtureFamily = loadFixtureFamily(rawPlan.fixture_manifest);
  if (!Array.isArray(rawPlan.cohorts) || rawPlan.cohorts.length === 0) {
    fail("cohort plan cohorts must be a non-empty array");
  }

  const hasWarmCohort = rawPlan.cohorts.some((cohort) => cohort?.mode === "process-warm");
  const variants = [];
  for (const rawVariant of rawPlan.variants ?? []) {
    const name = requiredString(rawVariant?.name, "variant.name");
    const installedLauncherPath = checkedRegularFile(
      rawVariant.installed_launcher_path,
      `variant ${name}.installed_launcher_path`,
      { executable: true },
    );
    const artifactPath = checkedRegularFile(
      rawVariant.artifact_path,
      `variant ${name}.artifact_path`,
      { executable: true },
    );
    if (
      !rawVariant.commands ||
      typeof rawVariant.commands !== "object" ||
      Array.isArray(rawVariant.commands)
    ) {
      fail(`variant ${name}.commands must be an object`);
    }
    const commands = {};
    for (const cohort of rawPlan.cohorts) {
      const workflow = requiredString(cohort.workflow, "cohort.workflow");
      const command = rawVariant.commands[workflow];
      if (!Array.isArray(command) || command.length === 0) {
        fail(`variant ${name}.commands.${workflow} must be a non-empty argv array`);
      }
      const executable = checkedRegularFile(
        command[0],
        `variant ${name}.commands.${workflow}[0]`,
        { executable: true },
      );
      commands[workflow] = [
        executable,
        ...command.slice(1).map((argument, index) =>
          requiredString(argument, `variant ${name}.commands.${workflow}[${index + 1}]`),
        ),
      ];
      if (workflow === "startup" && executable !== installedLauncherPath) {
        fail(
          `variant ${name}.commands.startup must launch the exact installed launcher`,
        );
      }
    }
    const warmSession = hasWarmCohort
      ? validateWarmSessionConfig(rawVariant.warm_session, {
          variantName: name,
          installedLauncherPath,
          scratchRoot,
        })
      : undefined;
    variants.push({
      ...rawVariant,
      name,
      installed_launcher_path: installedLauncherPath,
      artifact_path: artifactPath,
      artifact_sha256: await sha256File(artifactPath),
      installed_launcher_sha256: await sha256File(installedLauncherPath),
      source_revision: expectedRevision,
      source_sha256: sourceSha256,
      commands,
      ...(warmSession ? { warm_session: warmSession } : {}),
      cwd: rawVariant.cwd ? realpathSync(rawVariant.cwd) : dirname(commands.startup?.[0] ?? artifactPath),
    });
  }
  if (hasWarmCohort) {
    for (const field of ["fixture_install_root", "ready_marker_path", "control_path"]) {
      const values = variants.map((variant) => variant.warm_session[field]);
      if (new Set(values).size !== values.length) {
        fail(`process-warm variants must use distinct warm_session.${field} paths`);
      }
    }
  }

  const correctnessEvidence = [];
  for (const record of rawPlan.correctness_evidence ?? []) {
    const variant = variants.find((candidate) => candidate.name === record?.variant);
    const cohort = rawPlan.cohorts.find(
      (candidate) => candidate.cohort_id === record?.cohort_id,
    );
    const fixture = fixtureFamily.fixtures.find(
      (candidate) => candidate.fixture_id === cohort?.fixture_id,
    );
    if (!variant || !cohort || !fixture) {
      fail(
        `correctness evidence references unknown measurement identity ` +
          `${record?.variant ?? ""}/${record?.cohort_id ?? ""}`,
      );
    }
    const gates = {};
    for (const gate of CORRECTNESS_GATES) {
      const evidence = record?.gates?.[gate];
      if (!evidence) fail(`correctness evidence ${record?.variant ?? ""}.${gate} is missing`);
      const evidencePath = checkedRegularFile(
        evidence.evidence_path,
        `correctness evidence ${record.variant}/${record.cohort_id}.${gate}.evidence_path`,
      );
      const expectedSha256 = requiredSha256(
        evidence.sha256,
        `correctness evidence ${record.variant}/${record.cohort_id}.${gate}.sha256`,
      );
      const actualSha256 = await sha256File(evidencePath);
      if (actualSha256 !== expectedSha256) {
        fail(`correctness evidence ${record.variant}/${record.cohort_id}.${gate} changed`);
      }
      let payload;
      try {
        payload = JSON.parse(readFileSync(evidencePath, "utf8"));
      } catch (error) {
        fail(
          `correctness evidence ${record.variant}/${record.cohort_id}.${gate} ` +
            `is invalid JSON: ${error.message}`,
        );
      }
      const expectedIdentity = {
        schema_version: 1,
        record_type: "correctness-gate",
        run_id: rawPlan.run_id,
        variant: record.variant,
        cohort_id: record.cohort_id,
        fixture_id: fixture.fixture_id,
        fixture_sha256: fixture.sha256,
        source_revision: expectedRevision,
        source_sha256: sourceSha256,
        artifact_sha256: variant.artifact_sha256,
        gate_runner_path: gateRunnerPath,
        gate_runner_sha256: gateRunnerSha256,
        gate,
        status: evidence.status,
      };
      for (const [field, value] of Object.entries(expectedIdentity)) {
        if (payload?.[field] !== value) {
          fail(
            `correctness evidence ${record.variant}/${record.cohort_id}.${gate} ` +
              `changes ${field}`,
          );
        }
      }
      gates[gate] = {
        status: evidence.status,
        evidence_path: evidencePath,
        sha256: actualSha256,
      };
    }
    correctnessEvidence.push({ ...record, gates });
  }

  const prepared = {
    ...rawPlan,
    source_root: sourceRoot,
    scratch_root: scratchRoot,
    source_revision: expectedRevision,
    source_sha256: sourceSha256,
    fixture_manifest: realpathSync(rawPlan.fixture_manifest),
    fixture_family: fixtureFamily,
    variants,
    correctness_evidence: correctnessEvidence,
  };
  validatePlan(prepared);
  return prepared;
}

function fixtureProof(plan, cohort, variant, fixture, evidence) {
  return {
    schema_version: 1,
    record_type: "fixture-proof",
    run_id: plan.run_id,
    cohort_id: cohort.cohort_id,
    fixture_id: fixture.fixture_id,
    fixture_kind: fixture.kind,
    fixture_path: fixture.path,
    fixture_byte_count: fixture.byte_count,
    fixture_sha256: fixture.sha256,
    source_revision: variant.source_revision,
    source_sha256: variant.source_sha256,
    artifact_sha256: variant.artifact_sha256,
    installed_launcher_sha256: variant.installed_launcher_sha256,
    variant: variant.name,
    workflow: cohort.workflow,
    mode: cohort.mode,
    correctness: Object.fromEntries(
      CORRECTNESS_GATES.map((gate) => [gate, evidence.gates[gate].status]),
    ),
    correctness_evidence: Object.fromEntries(
      CORRECTNESS_GATES.map((gate) => [
        gate,
        {
          evidence_path: evidence.gates[gate].evidence_path,
          sha256: evidence.gates[gate].sha256,
        },
      ]),
    ),
  };
}

function exactIdentityFailures(sample, request) {
  const expected = {
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
    installed_launcher_sha256: request.variant.installed_launcher_sha256,
    variant: request.variant.name,
    workflow: request.workflow,
    mode: request.mode,
    metric: request.metric,
  };
  if (request.desktop_pid) {
    expected.desktop_pid = request.desktop_pid;
    expected.desktop_session_id = request.desktop_session_id;
  }
  const failures = [];
  for (const [field, value] of Object.entries(expected)) {
    if (sample?.[field] !== value) failures.push(`${field} changed`);
  }
  const stages = sample?.stage_durations_ms;
  for (const stage of requiredStages(request.workflow)) {
    if (
      typeof stages?.[stage] !== "number" ||
      !Number.isFinite(stages[stage]) ||
      stages[stage] < 0
    ) {
      failures.push(`${stage} duration is invalid`);
    }
  }
  if (
    typeof sample?.duration_ms !== "number" ||
    !Number.isFinite(sample.duration_ms) ||
    sample.duration_ms < 0
  ) {
    failures.push("duration_ms is invalid");
  }
  if (typeof sample?.execution_failure === "string" && sample.execution_failure) {
    failures.push(sample.execution_failure);
  }
  return failures;
}

function measurementEnvironment(request) {
  return {
    ALFREDO_MEASUREMENT_JSONL: request.raw_jsonl,
    ALFREDO_MEASUREMENT_RUN_ID: request.run_id,
    ALFREDO_MEASUREMENT_SAMPLE_ID: request.sample_id,
    ALFREDO_MEASUREMENT_COHORT_ID: request.cohort_id,
    ALFREDO_MEASUREMENT_CORRELATION_ID: request.correlation_id,
    ALFREDO_MEASUREMENT_FIXTURE_ID: request.fixture.fixture_id,
    ALFREDO_MEASUREMENT_FIXTURE_SHA256: request.fixture.sha256,
    ALFREDO_MEASUREMENT_SOURCE_SHA256: request.variant.source_sha256,
    ALFREDO_MEASUREMENT_ARTIFACT_SHA256: request.variant.artifact_sha256,
    ALFREDO_MEASUREMENT_VARIANT: request.variant.name,
    ALFREDO_MEASUREMENT_WORKFLOW: request.workflow,
    ALFREDO_MEASUREMENT_MODE: request.mode,
  };
}

function completedJsonLines(path) {
  let content;
  try {
    content = readFileSync(path, "utf8");
  } catch (error) {
    if (error?.code === "ENOENT") return [];
    throw error;
  }
  const lastNewline = content.lastIndexOf("\n");
  if (lastNewline < 0) return [];
  return content
    .slice(0, lastNewline)
    .split(/\r?\n/)
    .filter((line) => line.trim())
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch (error) {
        fail(`measurement output contains invalid completed JSON at line ${index + 1}: ${error.message}`);
      }
    });
}

function sampleMarks(path, sampleId) {
  return completedJsonLines(path).filter(
    (record) => record.record_type === "stage-mark" && record.sample_id === sampleId,
  );
}

function hasEndMark(records, stage) {
  return records.some((record) => record.stage === stage && record.boundary === "end");
}

function boundedOutput(current, chunk) {
  const next = `${current}${chunk}`;
  return next.length <= 65_536 ? next : next.slice(-65_536);
}

function wait(milliseconds) {
  return new Promise((resolveWait) => setTimeout(resolveWait, milliseconds));
}

function stopOwnedProcessGroup(child) {
  if (!child.pid) return;
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch (error) {
    if (error?.code !== "ESRCH") child.kill("SIGTERM");
  }
}

function sampleIdentity(request) {
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
    installed_launcher_sha256: request.variant.installed_launcher_sha256,
    variant: request.variant.name,
    workflow: request.workflow,
    mode: request.mode,
    metric: request.metric,
  };
}

function measurementCommandEnvironment(request) {
  const environment = {
    ...process.env,
    ...(request.variant.environment ?? {}),
  };
  if (request.workflow === "startup") {
    Object.assign(environment, measurementEnvironment(request));
    environment.ALFREDO_MEASUREMENT_FIXTURE_PATH = request.fixture.path;
    return environment;
  }
  for (const name of Object.keys(environment)) {
    if (name.startsWith("ALFREDO_MEASUREMENT_")) delete environment[name];
  }
  environment.ALFREDO_PERFORMANCE_FIXTURE_PATH = request.fixture.path;
  return environment;
}

export async function executeProductionSample(request) {
  const command = request.variant.commands?.[request.workflow];
  if (!Array.isArray(command) || command.length === 0) {
    fail(`variant ${request.variant.name} has no ${request.workflow} command`);
  }
  const metadata = {
    jsonl_path: request.raw_jsonl,
    run_id: request.run_id,
    sample_id: request.sample_id,
    cohort_id: request.cohort_id,
    correlation_id: request.correlation_id,
    fixture_id: request.fixture.fixture_id,
    fixture_sha256: request.fixture.sha256,
    source_sha256: request.variant.source_sha256,
    artifact_sha256: request.variant.artifact_sha256,
    variant: request.variant.name,
    workflow: request.workflow,
    mode: request.mode,
  };
  const driverRecorder =
    request.workflow === "startup"
      ? createPerformanceRecorder({
          ...metadata,
          source: "cohort-driver",
          clock_id: `cohort-driver:${process.pid}:${request.sample_id}`,
        })
      : null;
  const startedAt = process.hrtime.bigint();
  driverRecorder?.mark("S0", "start", { outcome: "pass" });
  let measuredAt = null;
  let executionFailure = "";
  let stdout = "";
  let stderr = "";
  const child = spawn(command[0], command.slice(1), {
    cwd: request.variant.cwd,
    env: measurementCommandEnvironment(request),
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let exit = null;
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout = boundedOutput(stdout, chunk);
  });
  child.stderr.on("data", (chunk) => {
    stderr = boundedOutput(stderr, chunk);
  });
  child.on("error", (error) => {
    executionFailure = `measurement command failed to start: ${error.message}`;
  });
  child.on("close", (code, signal) => {
    exit = { code, signal };
  });

  const measurementStage =
    request.workflow === "startup"
      ? request.metric.endsWith("S9")
        ? "S9"
        : "S8"
      : request.metric.endsWith("R6")
        ? "R6"
        : "R5";
  const completionStage =
    request.workflow === "startup"
      ? "S9"
      : request.workflow === "queue-approve"
        ? "R6"
        : "R5";
  const timeoutMs = request.variant.timeout_ms ?? 120_000;
  const deadline = Date.now() + timeoutMs;
  while (!executionFailure && Date.now() <= deadline) {
    const marks = sampleMarks(request.raw_jsonl, request.sample_id);
    if (measuredAt === null && hasEndMark(marks, measurementStage)) {
      measuredAt = process.hrtime.bigint();
      driverRecorder?.mark("S0", "end", {
        outcome: "pass",
        endpoint_stage: measurementStage,
      });
    }
    if (hasEndMark(marks, completionStage)) {
      if (exit === null && request.variant.terminate_after_endpoint !== false) {
        stopOwnedProcessGroup(child);
      }
      break;
    }
    if (exit !== null) {
      executionFailure =
        `measurement command exited before ${completionStage} ` +
        `(code=${exit.code ?? "null"}, signal=${exit.signal ?? "none"})`;
      break;
    }
    await wait(5);
  }
  if (!executionFailure && Date.now() > deadline) {
    executionFailure = `measurement command timed out before ${completionStage}`;
    stopOwnedProcessGroup(child);
  }
  if (exit === null) {
    for (let attempt = 0; attempt < 100 && exit === null; attempt += 1) {
      await wait(5);
    }
  }
  const marks = sampleMarks(request.raw_jsonl, request.sample_id);
  const outerDurationMs =
    measuredAt === null ? Number.NaN : Number(measuredAt - startedAt) / 1_000_000;
  const base = sampleIdentity(request);
  try {
    const compileOptions = {
      pair_id: request.pair_id,
      metric: request.metric,
    };
    if (request.workflow === "startup") {
      compileOptions.duration_ms = outerDurationMs;
      compileOptions.metric_clock_id = driverRecorder
        ? `cohort-driver:${process.pid}:${request.sample_id}`
        : "";
    }
    const [compiled] = compileStageMarks(marks, compileOptions);
    return {
      ...compiled,
      fixture_byte_count: request.fixture.byte_count,
      source_revision: request.variant.source_revision,
      installed_launcher_sha256: request.variant.installed_launcher_sha256,
      raw_mark_count: marks.length,
      execution_failure: executionFailure,
      command_exit_code: exit?.code ?? null,
      command_signal: exit?.signal ?? "",
      command_stdout_tail: stdout,
      command_stderr_tail: stderr,
    };
  } catch (error) {
    return {
      ...base,
      duration_ms: request.workflow === "startup" ? outerDurationMs : Number.NaN,
      stage_durations_ms: {},
      raw_mark_count: marks.length,
      execution_failure: executionFailure || error.message,
      command_exit_code: exit?.code ?? null,
      command_signal: exit?.signal ?? "",
      command_stdout_tail: stdout,
      command_stderr_tail: stderr,
    };
  }
}

export async function runCohortPlan(
  rawPlan,
  { executeSample, lifecycle = null, random = Math.random } = {},
) {
  if (typeof executeSample !== "function") fail("executeSample must be a function");
  const plan = validatePlan(rawPlan);
  const fixtureProofs = [];
  const sampleFixtureProofs = [];
  const samples = [];

  for (const cohort of plan.cohorts) {
    const fixture = plan.fixtureById.get(cohort.fixture_id);
    for (const variantName of [plan.baseline_variant, plan.candidate_variant]) {
      const variant = plan.variantByName.get(variantName);
      const evidence = plan.evidenceByIdentity.get(
        `${variantName}\u0000${cohort.cohort_id}`,
      );
      fixtureProofs.push(fixtureProof(plan, cohort, variant, fixture, evidence));
    }
    const schedule = createRandomizedPairSchedule({
      mode: cohort.mode,
      sample_count: cohort.sample_count,
      baseline_variant: plan.baseline_variant,
      candidate_variant: plan.candidate_variant,
      random,
    });
    const warmSessions = new Map();
    try {
      if (cohort.mode === "process-warm" && lifecycle) {
        if (
          typeof lifecycle.activateWarmSession !== "function" ||
          typeof lifecycle.deactivateWarmSession !== "function"
        ) {
          fail("process-warm lifecycle must support exclusive activate/deactivate");
        }
        for (const variantName of [plan.baseline_variant, plan.candidate_variant]) {
          const variant = plan.variantByName.get(variantName);
          const session = await lifecycle.startWarmSession({
            run_id: plan.run_id,
            cohort_id: cohort.cohort_id,
            variant,
            fixture,
          });
          await lifecycle.deactivateWarmSession(session);
          warmSessions.set(variantName, session);
        }
      }
      for (const pair of schedule) {
        const pairId = `${cohort.cohort_id}-pair-${String(pair.pair_index).padStart(4, "0")}`;
        for (const variantName of pair.variants) {
          const variant = plan.variantByName.get(variantName);
          const sampleId = `${pairId}-${variantName}`;
          const warmSession = warmSessions.get(variantName) ?? null;
          let warmSessionActive = false;
          try {
          let request = {
          run_id: plan.run_id,
          raw_jsonl: plan.raw_jsonl,
          cohort_id: cohort.cohort_id,
          sample_id: sampleId,
          pair_id: pairId,
          correlation_id: `${plan.run_id}-${sampleId}`,
          fixture,
          variant,
          workflow: cohort.workflow,
          mode: cohort.mode,
          metric: cohort.metric,
          };
          if (lifecycle) {
            const prepared = await lifecycle.prepareSample(
              request,
              warmSession,
            );
            if (!prepared?.fixture_proof) {
              fail(`lifecycle did not return a fresh fixture proof for ${sampleId}`);
            }
            sampleFixtureProofs.push({
              schema_version: 1,
              record_type: "sample-fixture-proof",
              run_id: plan.run_id,
              cohort_id: cohort.cohort_id,
              sample_id: sampleId,
              pair_id: pairId,
              variant: variant.name,
              ...prepared.fixture_proof,
            });
            request = { ...request, ...prepared };
          }
          if (warmSession) {
            await lifecycle.activateWarmSession(warmSession);
            warmSessionActive = true;
          }
          const measured = await executeSample(request);
        const evidence = plan.evidenceByIdentity.get(
          `${variantName}\u0000${cohort.cohort_id}`,
        );
        const correctness = Object.fromEntries(
          CORRECTNESS_GATES.map((gate) => [gate, evidence.gates[gate].status]),
        );
        const correctnessEvidence = Object.fromEntries(
          CORRECTNESS_GATES.map((gate) => [
            gate,
            {
              evidence_path: evidence.gates[gate].evidence_path,
              sha256: evidence.gates[gate].sha256,
            },
          ]),
        );
        const identityFailures = exactIdentityFailures(measured, request);
        const correctnessFailures = CORRECTNESS_GATES.filter(
          (gate) => correctness[gate] !== "pass",
        ).map((gate) => `${gate} evidence failed`);
        const failures = [...identityFailures, ...correctnessFailures];
          samples.push({
          ...measured,
          correctness,
          correctness_evidence: correctnessEvidence,
          valid: failures.length === 0,
          invalid_reason: failures.join("; "),
          });
          } finally {
            if (lifecycle && warmSession && warmSessionActive) {
              await lifecycle.deactivateWarmSession(warmSession);
            }
          }
        }
      }
    } finally {
      if (lifecycle) {
        for (const session of warmSessions.values()) {
          await lifecycle.stopWarmSession(session);
        }
      }
    }
  }
  return { fixture_proofs: fixtureProofs, sample_fixture_proofs: sampleFixtureProofs, samples };
}

function parseOption(args, name) {
  const index = args.indexOf(name);
  if (index === -1 || index + 1 >= args.length) fail(`${name} is required`);
  return args[index + 1];
}

function optionalOption(args, name) {
  const index = args.indexOf(name);
  return index === -1 ? undefined : args[index + 1];
}

function readPlan(path) {
  const planPath = checkedRegularFile(path, "plan");
  try {
    return JSON.parse(readFileSync(planPath, "utf8"));
  } catch (error) {
    fail(`measurement plan is invalid JSON: ${error.message}`);
  }
}

function ensureNewOutput(path, label) {
  if (!isAbsolute(path)) fail(`${label} must be absolute`);
  realpathSync(dirname(path));
  try {
    const entry = lstatSync(path);
    if (entry.isSymbolicLink() || !entry.isFile()) {
      fail(`${label} must be a new regular non-symlink file`);
    }
    fail(`${label} already exists; measurement evidence is append-only per run`);
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
}

function appendRecords(path, records) {
  for (const record of records) {
    appendFileSync(path, `${JSON.stringify(record)}\n`, {
      encoding: "utf8",
      mode: 0o600,
      flag: "a",
    });
  }
}

async function runCli(args) {
  const command = args[0] ?? "";
  if (command === "record-gate") {
    const timeout = optionalOption(args, "--timeout-ms");
    const record = await recordCorrectnessGate({
      output_path: resolve(parseOption(args, "--output")),
      run_id: parseOption(args, "--run-id"),
      variant: parseOption(args, "--variant"),
      cohort_id: parseOption(args, "--cohort"),
      gate: parseOption(args, "--gate"),
      source_root: resolve(parseOption(args, "--source-root")),
      source_revision: parseOption(args, "--source-revision"),
      artifact_path: resolve(parseOption(args, "--artifact")),
      fixture_manifest: resolve(parseOption(args, "--fixture-manifest")),
      fixture_id: parseOption(args, "--fixture"),
      cwd: optionalOption(args, "--cwd"),
      timeout_ms: timeout === undefined ? undefined : Number(timeout),
    });
    process.stdout.write(`${JSON.stringify(record)}\n`);
    if (record.status !== "pass") process.exitCode = 2;
    return;
  }
  if (command !== "check" && command !== "run") {
    fail(
      "Usage: performance-driver.js record-gate <identity options> | " +
        "check --plan <absolute-plan.json> | " +
        "run --plan <absolute-plan.json> --report <absolute-report.json>",
    );
  }
  const prepared = await prepareProductionPlan(readPlan(parseOption(args, "--plan")));
  if (command === "check") {
    process.stdout.write(
      `${JSON.stringify(
        {
          schema_version: 1,
          run_id: prepared.run_id,
          source_revision: prepared.source_revision,
          source_sha256: prepared.source_sha256,
          variants: prepared.variants.map((variant) => ({
            name: variant.name,
            artifact_path: variant.artifact_path,
            artifact_sha256: variant.artifact_sha256,
            installed_launcher_path: variant.installed_launcher_path,
            installed_launcher_sha256: variant.installed_launcher_sha256,
          })),
          fixtures: prepared.fixture_family.fixtures.map((fixture) => ({
            fixture_id: fixture.fixture_id,
            byte_count: fixture.byte_count,
            sha256: fixture.sha256,
          })),
          cohorts: prepared.cohorts.map((cohort) => ({
            cohort_id: cohort.cohort_id,
            mode: cohort.mode,
            sample_pairs: cohort.sample_count,
            invocations: cohort.sample_count * 2,
          })),
          correctness_evidence: prepared.correctness_evidence,
        },
        null,
        2,
      )}\n`,
    );
    return;
  }

  const reportPath = resolve(parseOption(args, "--report"));
  ensureNewOutput(prepared.raw_jsonl, "raw_jsonl");
  ensureNewOutput(reportPath, "report");
  const result = await runCohortPlan(prepared, {
    executeSample: executeProductionSample,
    lifecycle: createProductionLifecycle({ scratch_root: prepared.scratch_root }),
  });
  appendRecords(prepared.raw_jsonl, [
    ...result.fixture_proofs,
    ...result.sample_fixture_proofs,
    ...result.samples,
  ]);
  const report = summarizeCohorts(result.samples, {
    baseline_variant: prepared.baseline_variant,
    candidate_variant: prepared.candidate_variant,
    raw_jsonl: prepared.raw_jsonl,
    minimum_improvement_ms: prepared.minimum_product_improvement_ms,
  });
  writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, {
    encoding: "utf8",
    mode: 0o600,
    flag: "wx",
  });
  process.stdout.write(`${JSON.stringify(report)}\n`);
  if (!report.speed_claim_eligible) process.exitCode = 2;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await runCli(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`Error: ${error.message}\n`);
    process.exitCode = 1;
  }
}
