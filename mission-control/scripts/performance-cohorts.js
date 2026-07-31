#!/usr/bin/env node
import { createHash } from "node:crypto";
import {
  lstatSync,
  readFileSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { canonicalTreeSha256 } from "./performance-lifecycle.js";

export const STARTUP_STAGES = Object.freeze([
  "S0",
  "S1",
  "S2",
  "S3",
  "S4",
  "S5",
  "S6",
  "S7",
  "S8",
  "S9",
]);

export const RENDERED_ACTION_STAGES = Object.freeze([
  "R0",
  "R1",
  "R2",
  "R3",
  "R4",
  "R5",
  "R6",
]);

export const CORRECTNESS_GATES = Object.freeze([
  "contract",
  "replay",
  "crash_cut",
  "packaging",
  "rollback",
]);
export const STAGE_SOURCES = Object.freeze({
  S0: "cohort-driver",
  S1: "launcher",
  S2: "native-shell",
  S3: "react",
  S4: "native-shell",
  S5: "native-shell",
  S6: "python-authority",
  S7: "native-shell",
  S8: "react",
  S9: "react",
  R0: "react",
  R1: "native-shell",
  R2: "python-authority",
  R3: "native-shell",
  R4: "react",
  R5: "react",
  R6: "react",
});

const FIXTURE_KINDS = Object.freeze([
  "minimal-ready",
  "representative-warm",
  "pending-ad-hoc",
]);
const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const SOURCE_REVISION_PATTERN = /^[a-f0-9]{40}$/;
const MINIMUM_SAMPLES = Object.freeze({
  "process-cold": 30,
  "process-warm": 100,
});

function fail(message) {
  throw new Error(message);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function isInside(root, candidate) {
  const fromRoot = relative(root, candidate);
  return fromRoot === "" || (!fromRoot.startsWith("..") && !isAbsolute(fromRoot));
}

function requireString(value, label) {
  if (typeof value !== "string" || !value.trim()) fail(`${label} must be a non-empty string`);
  return value;
}

function requireSha256(value, label) {
  requireString(value, label);
  if (!SHA256_PATTERN.test(value)) fail(`${label} must be a lowercase SHA-256`);
  return value;
}

function requireCorrectness(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
  for (const gate of CORRECTNESS_GATES) {
    if (value[gate] !== "pass" && value[gate] !== "fail") {
      fail(`${label}.${gate} must be pass or fail`);
    }
  }
  return value;
}

function checkedFixturePath(manifestPath, fixturePath) {
  requireString(fixturePath, "fixture.path");
  if (isAbsolute(fixturePath)) fail("fixture.path must be relative to its manifest");
  const root = realpathSync(dirname(manifestPath));
  const candidate = resolve(root, fixturePath);
  const entry = lstatSync(candidate);
  if (!entry.isFile() || entry.isSymbolicLink()) {
    fail(`fixture must be a regular non-symlink file: ${fixturePath}`);
  }
  const realCandidate = realpathSync(candidate);
  if (!isInside(root, realCandidate)) fail(`fixture escapes its manifest directory: ${fixturePath}`);
  return realCandidate;
}

export function loadFixtureFamily(manifestPath) {
  const absoluteManifest = resolve(manifestPath);
  const manifest = JSON.parse(readFileSync(absoluteManifest, "utf8"));
  if (manifest.schema_version !== 1) fail("fixture manifest schema_version must be 1");
  if (!Array.isArray(manifest.fixtures)) fail("fixture manifest fixtures must be an array");
  if (manifest.fixtures.length !== FIXTURE_KINDS.length) {
    fail(`fixture manifest must contain exactly ${FIXTURE_KINDS.length} fixtures`);
  }

  const fixtures = manifest.fixtures.map((fixture, index) => {
    const expectedKind = FIXTURE_KINDS[index];
    if (fixture.kind !== expectedKind) {
      fail(`fixture ${index + 1} kind must be ${expectedKind}`);
    }
    requireString(fixture.fixture_id, `fixture ${expectedKind}.fixture_id`);
    if (!/^[a-z0-9-]+-v1$/.test(fixture.fixture_id)) {
      fail(`fixture ${expectedKind}.fixture_id must be a stable v1 identity`);
    }
    requireSha256(fixture.sha256, `fixture ${expectedKind}.sha256`);
    if (!Number.isSafeInteger(fixture.byte_count) || fixture.byte_count <= 0) {
      fail(`fixture ${expectedKind}.byte_count must be a positive integer`);
    }

    const path = checkedFixturePath(absoluteManifest, fixture.path);
    const bytes = readFileSync(path);
    if (bytes.byteLength !== fixture.byte_count) {
      fail(`fixture ${expectedKind} byte count changed`);
    }
    if (sha256(bytes) !== fixture.sha256) fail(`fixture ${expectedKind} SHA-256 changed`);
    const payload = JSON.parse(bytes.toString("utf8"));
    if (
      payload.schema_version !== 1 ||
      payload.fixture_id !== fixture.fixture_id ||
      payload.kind !== fixture.kind
    ) {
      fail(`fixture ${expectedKind} payload identity does not match its manifest`);
    }
    if (!isAbsolute(payload.installation_root)) {
      fail(`fixture ${expectedKind} installation_root must be absolute`);
    }
    requireSha256(
      payload.canonical_tree_sha256,
      `fixture ${expectedKind}.canonical_tree_sha256`,
    );
    if (canonicalTreeSha256(payload.canonical_files) !== payload.canonical_tree_sha256) {
      fail(`fixture ${expectedKind} canonical tree changed`);
    }
    return {
      ...fixture,
      path,
      installation_root: payload.installation_root,
      canonical_tree_sha256: payload.canonical_tree_sha256,
      canonical_files: payload.canonical_files,
      identities: payload.identities,
      counts: payload.counts,
    };
  });

  return {
    ...manifest,
    manifest_path: absoluteManifest,
    fixtures,
  };
}

export function readJsonLines(path) {
  const records = [];
  const lines = readFileSync(path, "utf8").split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    if (!lines[index].trim()) continue;
    try {
      records.push(JSON.parse(lines[index]));
    } catch (error) {
      fail(`raw JSON Lines record ${index + 1} is invalid: ${error.message}`);
    }
  }
  return records;
}

export function writeJsonLines(path, records) {
  const encoded = records.map((record) => JSON.stringify(record)).join("\n");
  writeFileSync(path, encoded ? `${encoded}\n` : "", {
    encoding: "utf8",
    flag: "wx",
    mode: 0o600,
  });
}

function stageMarkIdentity(record) {
  return [
    record.run_id,
    record.sample_id,
    record.cohort_id,
    record.correlation_id,
    record.fixture_id,
    record.variant,
    record.workflow,
    record.mode,
  ].join("\u0000");
}

export function compileStageMarks(records, options = {}) {
  if (!Array.isArray(records)) fail("stage marks must be an array");
  const groups = new Map();
  for (const record of records) {
    if (record?.schema_version !== 1 || record?.record_type !== "stage-mark") {
      fail("stage marks must use schema version 1");
    }
    const key = stageMarkIdentity(record);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(record);
  }

  return [...groups.values()].map((marks) => {
    const first = marks[0];
    const byStage = new Map();
    const lastTickByClock = new Map();
    for (const mark of marks) {
      if (mark?.detail?.outcome === "fail") {
        fail(`${mark.stage} contains an explicit failed outcome`);
      }
      if (STAGE_SOURCES[mark.stage] !== mark.source) {
        fail(`${mark.stage} must be recorded by ${STAGE_SOURCES[mark.stage] ?? "a known owner"}`);
      }
      for (const field of [
        "run_id",
        "sample_id",
        "cohort_id",
        "correlation_id",
        "fixture_id",
        "fixture_sha256",
        "source_sha256",
        "artifact_sha256",
        "variant",
        "workflow",
        "mode",
        "desktop_pid",
        "desktop_session_id",
      ]) {
        if (mark[field] !== first[field]) {
          fail(`sample ${first.sample_id} changes immutable ${field}`);
        }
      }
      if (!byStage.has(mark.stage)) byStage.set(mark.stage, []);
      byStage.get(mark.stage).push(mark);
      let tick;
      try {
        tick = BigInt(mark.monotonic_ns);
      } catch {
        fail(`${mark.stage} monotonic marks must be integer nanoseconds`);
      }
      const previousTick = lastTickByClock.get(mark.clock_id);
      if (tick < 0n || (previousTick !== undefined && tick < previousTick)) {
        fail(`${mark.clock_id} emits out-of-order monotonic marks`);
      }
      lastTickByClock.set(mark.clock_id, tick);
    }
    const stageDurations = {};
    for (const [stage, stageMarks] of byStage) {
      const starts = stageMarks.filter((mark) => mark.boundary === "start");
      const ends = stageMarks.filter((mark) => mark.boundary === "end");
      if (starts.length !== 1 || ends.length !== 1) {
        fail(`${stage} must have exactly one start and one end mark`);
      }
      if (starts[0].clock_id !== ends[0].clock_id) {
        fail(`${stage} start and end must share one monotonic clock`);
      }
      let start;
      let end;
      try {
        start = BigInt(starts[0].monotonic_ns);
        end = BigInt(ends[0].monotonic_ns);
      } catch {
        fail(`${stage} monotonic marks must be integer nanoseconds`);
      }
      if (start < 0n || end < start) fail(`${stage} monotonic marks moved backwards`);
      stageDurations[stage] = Number(end - start) / 1_000_000;
    }
    const metric = requireString(options.metric ?? first.metric, "metric");
    let durationMs = options.duration_ms ?? first.duration_ms;
    let metricClockId = options.metric_clock_id ?? first.metric_clock_id;
    if (durationMs === undefined && metric.startsWith("R")) {
      const [startStage, endStage, extra] = metric.split("->");
      if (extra || !/^R[0-6]$/.test(startStage) || !/^R[0-6]$/.test(endStage)) {
        fail(`unsupported rendered metric: ${metric}`);
      }
      const starts = (byStage.get(startStage) ?? []).filter(
        (mark) => mark.boundary === "start",
      );
      const ends = (byStage.get(endStage) ?? []).filter(
        (mark) => mark.boundary === "end",
      );
      if (starts.length !== 1 || ends.length !== 1) {
        fail(`${metric} requires exactly one start and endpoint mark`);
      }
      if (starts[0].clock_id !== ends[0].clock_id) {
        fail(`${metric} metric endpoints must share one monotonic clock`);
      }
      const start = BigInt(starts[0].monotonic_ns);
      const end = BigInt(ends[0].monotonic_ns);
      if (start < 0n || end < start) fail(`${metric} metric clock moved backwards`);
      durationMs = Number(end - start) / 1_000_000;
      metricClockId = starts[0].clock_id;
    }
    const desktopIdentity =
      first.desktop_pid === undefined && first.desktop_session_id === undefined
        ? {}
        : {
            desktop_pid: first.desktop_pid,
            desktop_session_id: first.desktop_session_id,
          };
    if (
      Object.keys(desktopIdentity).length > 0 &&
      (!Number.isSafeInteger(first.desktop_pid) ||
        first.desktop_pid <= 0 ||
        typeof first.desktop_session_id !== "string" ||
        !first.desktop_session_id.trim())
    ) {
      fail(`sample ${first.sample_id} has invalid desktop identity`);
    }
    return {
      schema_version: 1,
      record_type: "sample",
      cohort_id: first.cohort_id,
      sample_id: first.sample_id,
      pair_id: options.pair_id ?? first.pair_id ?? first.sample_id,
      correlation_id: first.correlation_id,
      fixture_id: first.fixture_id,
      fixture_sha256: first.fixture_sha256,
      source_sha256: first.source_sha256,
      artifact_sha256: first.artifact_sha256,
      variant: first.variant,
      workflow: first.workflow,
      mode: first.mode,
      ...desktopIdentity,
      metric,
      duration_ms: durationMs,
      metric_clock_id: metricClockId,
      stage_durations_ms: stageDurations,
      correctness: options.correctness ?? first.correctness,
    };
  });
}

function requiredStages(record) {
  if (record.workflow === "startup") return STARTUP_STAGES;
  if (record.workflow === "queue-defer") return RENDERED_ACTION_STAGES.slice(0, 6);
  if (record.workflow === "queue-approve" || record.workflow === "session-claim") {
    return RENDERED_ACTION_STAGES;
  }
  return [];
}

function validateSample(record) {
  const failures = [];
  if (record.schema_version !== 1 || record.record_type !== "sample") {
    failures.push("unsupported sample schema");
  }
  for (const field of [
    "cohort_id",
    "sample_id",
    "pair_id",
    "correlation_id",
    "fixture_id",
    "variant",
    "workflow",
    "mode",
    "metric",
  ]) {
    if (typeof record[field] !== "string" || !record[field].trim()) {
      failures.push(`${field} is missing`);
    }
  }
  for (const field of ["fixture_sha256", "source_sha256", "artifact_sha256"]) {
    if (typeof record[field] !== "string" || !SHA256_PATTERN.test(record[field])) {
      failures.push(`${field} is invalid`);
    }
  }
  if (!Number.isSafeInteger(record.fixture_byte_count) || record.fixture_byte_count <= 0) {
    failures.push("fixture_byte_count is invalid");
  }
  if (
    typeof record.source_revision !== "string" ||
    !SOURCE_REVISION_PATTERN.test(record.source_revision)
  ) {
    failures.push("source_revision is invalid");
  }
  if (!(record.mode in MINIMUM_SAMPLES)) failures.push("mode is invalid");
  if (typeof record.duration_ms !== "number" || !Number.isFinite(record.duration_ms) || record.duration_ms < 0) {
    failures.push("duration_ms is invalid");
  }
  const stages = requiredStages(record);
  if (stages.length === 0) failures.push("workflow is invalid");
  if (!record.stage_durations_ms || typeof record.stage_durations_ms !== "object") {
    failures.push("stage_durations_ms is missing");
  } else {
    for (const stage of stages) {
      const duration = record.stage_durations_ms[stage];
      if (typeof duration !== "number" || !Number.isFinite(duration) || duration < 0) {
        failures.push(`${stage} duration is invalid`);
      }
    }
  }
  try {
    requireCorrectness(record.correctness, "correctness");
  } catch (error) {
    failures.push(error.message);
  }
  if (
    !record.correctness_evidence ||
    typeof record.correctness_evidence !== "object" ||
    Array.isArray(record.correctness_evidence)
  ) {
    failures.push("correctness_evidence is missing");
  } else {
    for (const gate of CORRECTNESS_GATES) {
      const evidence = record.correctness_evidence[gate];
      if (!evidence || typeof evidence !== "object" || Array.isArray(evidence)) {
        failures.push(`correctness_evidence.${gate} is missing`);
        continue;
      }
      if (
        typeof evidence.evidence_path !== "string" ||
        !isAbsolute(evidence.evidence_path)
      ) {
        failures.push(`correctness_evidence.${gate}.evidence_path is invalid`);
      }
      if (typeof evidence.sha256 !== "string" || !SHA256_PATTERN.test(evidence.sha256)) {
        failures.push(`correctness_evidence.${gate}.sha256 is invalid`);
      }
    }
  }
  const correctnessPassed =
    record.correctness &&
    CORRECTNESS_GATES.every((gate) => record.correctness[gate] === "pass");
  if (!correctnessPassed) failures.push("invalid correctness evidence");
  if (record.valid === false) {
    failures.push(
      typeof record.invalid_reason === "string" && record.invalid_reason
        ? `sample invalid: ${record.invalid_reason}`
        : "sample is explicitly invalid",
    );
  }
  if (typeof record.execution_failure === "string" && record.execution_failure) {
    failures.push(`execution failed: ${record.execution_failure}`);
  }
  return { valid: failures.length === 0, failures };
}

function nearestRank(values, percentile) {
  if (values.length === 0) return null;
  const ordered = [...values].sort((left, right) => left - right);
  const rank = Math.max(1, Math.ceil(percentile * ordered.length));
  return ordered[rank - 1];
}

function groupKey(record) {
  return [
    record.cohort_id,
    record.fixture_id,
    record.variant,
    record.workflow,
    record.mode,
    record.metric,
  ].join("\u0000");
}

function deltaGroupKey(record) {
  return [
    record.cohort_id,
    record.fixture_id,
    record.workflow,
    record.mode,
    record.metric,
  ].join("\u0000");
}

function lexical(left, right) {
  return left.localeCompare(right);
}

export function summarizeCohorts(records, options = {}) {
  if (!Array.isArray(records)) fail("raw cohort evidence must be an array");
  const baselineVariant = requireString(options.baseline_variant, "baseline_variant");
  const candidateVariant = requireString(options.candidate_variant, "candidate_variant");
  if (baselineVariant === candidateVariant) fail("candidate_variant must differ from baseline_variant");
  const minimumImprovement = options.minimum_improvement_ms;
  if (
    typeof minimumImprovement !== "number" ||
    !Number.isFinite(minimumImprovement) ||
    minimumImprovement <= 0
  ) {
    fail("minimum_improvement_ms must be a positive predeclared number");
  }

  const validated = records.map((record) => ({ record, result: validateSample(record) }));
  const groups = new Map();
  for (const item of validated) {
    const key = groupKey(item.record);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(item);
  }

  const ineligibilityReasons = [];
  const cohorts = [...groups.values()]
    .map((items) => {
      const first = items[0].record;
      const valid = items.filter((item) => item.result.valid);
      const invalid = items.filter((item) => !item.result.valid);
      const durations = valid.map((item) => item.record.duration_ms);
      const minimum = MINIMUM_SAMPLES[first.mode] ?? Number.POSITIVE_INFINITY;
      if (invalid.some((item) => item.result.failures.includes("invalid correctness evidence"))) {
        ineligibilityReasons.push(
          `${first.variant}/${first.mode}/${first.workflow}/${first.metric} has invalid correctness evidence`,
        );
      }
      if (durations.length < minimum) {
        ineligibilityReasons.push(
          `${first.variant}/${first.mode}/${first.workflow}/${first.metric} has ${durations.length}/${minimum} valid samples`,
        );
      }
      return {
        cohort_id: first.cohort_id,
        fixture_id: first.fixture_id,
        variant: first.variant,
        workflow: first.workflow,
        mode: first.mode,
        metric: first.metric,
        raw_sample_count: items.length,
        sample_count: durations.length,
        p50_ms: nearestRank(durations, 0.5),
        p95_ms: nearestRank(durations, 0.95),
        min_ms: durations.length > 0 ? Math.min(...durations) : null,
        max_ms: durations.length > 0 ? Math.max(...durations) : null,
        failures: invalid.length,
        failure_records: invalid.map((item) => ({
          sample_id: item.record.sample_id ?? "",
          reasons: item.result.failures,
        })),
      };
    })
    .sort((left, right) =>
      lexical(left.mode, right.mode) ||
      lexical(left.workflow, right.workflow) ||
      lexical(left.metric, right.metric) ||
      lexical(left.variant, right.variant),
    );

  const deltaGroups = new Map();
  for (const item of validated) {
    if (!item.result.valid) continue;
    const key = deltaGroupKey(item.record);
    if (!deltaGroups.has(key)) deltaGroups.set(key, []);
    deltaGroups.get(key).push(item.record);
  }
  const pairedDeltas = [...deltaGroups.values()]
    .map((items) => {
      const first = items[0];
      const byPair = new Map();
      for (const item of items) {
        if (!byPair.has(item.pair_id)) byPair.set(item.pair_id, new Map());
        const variants = byPair.get(item.pair_id);
        if (variants.has(item.variant)) {
          fail(`pair ${item.pair_id} repeats variant ${item.variant}`);
        }
        variants.set(item.variant, item.duration_ms);
      }
      const deltas = [];
      for (const variants of byPair.values()) {
        if (variants.has(baselineVariant) && variants.has(candidateVariant)) {
          deltas.push(variants.get(candidateVariant) - variants.get(baselineVariant));
        }
      }
      if (deltas.length === 0) return null;
      const minimum = MINIMUM_SAMPLES[first.mode] ?? Number.POSITIVE_INFINITY;
      if (deltas.length < minimum) {
        ineligibilityReasons.push(
          `${first.mode}/${first.workflow}/${first.metric} has ${deltas.length}/${minimum} valid pairs`,
        );
      }
      const p50 = nearestRank(deltas, 0.5);
      if (p50 > -minimumImprovement) {
        ineligibilityReasons.push(
          `${first.mode}/${first.workflow}/${first.metric} p50 improvement ` +
            `${-p50}ms is below the predeclared ${minimumImprovement}ms minimum`,
        );
      }
      return {
        cohort_id: first.cohort_id,
        fixture_id: first.fixture_id,
        workflow: first.workflow,
        mode: first.mode,
        metric: first.metric,
        baseline_variant: baselineVariant,
        candidate_variant: candidateVariant,
        sample_count: deltas.length,
        p50_ms: p50,
        p95_ms: nearestRank(deltas, 0.95),
        min_ms: Math.min(...deltas),
        max_ms: Math.max(...deltas),
      };
    })
    .filter(Boolean)
    .sort((left, right) => lexical(left.mode, right.mode));

  const requiredModes = Object.keys(MINIMUM_SAMPLES);
  for (const mode of requiredModes) {
    for (const variant of [baselineVariant, candidateVariant]) {
      if (!cohorts.some((cohort) => cohort.mode === mode && cohort.variant === variant)) {
        ineligibilityReasons.push(`${variant}/${mode} cohort is missing`);
      }
    }
  }

  return {
    schema_version: 1,
    baseline_variant: baselineVariant,
    candidate_variant: candidateVariant,
    minimum_improvement_ms: minimumImprovement,
    raw_jsonl: options.raw_jsonl ? resolve(options.raw_jsonl) : "",
    cohorts,
    paired_deltas: pairedDeltas,
    ineligibility_reasons: [...new Set(ineligibilityReasons)],
    speed_claim_eligible: ineligibilityReasons.length === 0,
  };
}

function parseOption(args, name) {
  const index = args.indexOf(name);
  if (index === -1 || index + 1 >= args.length) fail(`${name} is required`);
  return args[index + 1];
}

function runCli(args) {
  const command = args[0] ?? "";
  if (command === "fixture-check") {
    const family = loadFixtureFamily(parseOption(args, "--manifest"));
    process.stdout.write(`${JSON.stringify(family, null, 2)}\n`);
    return;
  }
  if (command === "report") {
    const rawPath = parseOption(args, "--raw");
    const outputPath = parseOption(args, "--output");
    const report = summarizeCohorts(
      readJsonLines(rawPath).filter((record) => record?.record_type === "sample"),
      {
      baseline_variant: parseOption(args, "--baseline"),
      candidate_variant: parseOption(args, "--candidate"),
      minimum_improvement_ms: Number(parseOption(args, "--minimum-improvement-ms")),
      raw_jsonl: rawPath,
      },
    );
    writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
    process.stdout.write(`${JSON.stringify(report)}\n`);
    if (!report.speed_claim_eligible) process.exitCode = 2;
    return;
  }
  fail(
    "Usage: performance-cohorts.js fixture-check --manifest <path> | " +
      "report --raw <path> --output <path> --baseline <name> --candidate <name> " +
      "--minimum-improvement-ms <positive-number>",
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    runCli(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`Error: ${error.message}\n`);
    process.exitCode = 1;
  }
}
