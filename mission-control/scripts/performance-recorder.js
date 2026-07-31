import { appendFileSync, lstatSync, readFileSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, resolve } from "node:path";

const STAGES = new Set([
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
  "R0",
  "R1",
  "R2",
  "R3",
  "R4",
  "R5",
  "R6",
]);
const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const ENVIRONMENT_FIELDS = Object.freeze({
  jsonl_path: "ALFREDO_MEASUREMENT_JSONL",
  run_id: "ALFREDO_MEASUREMENT_RUN_ID",
  sample_id: "ALFREDO_MEASUREMENT_SAMPLE_ID",
  cohort_id: "ALFREDO_MEASUREMENT_COHORT_ID",
  correlation_id: "ALFREDO_MEASUREMENT_CORRELATION_ID",
  fixture_id: "ALFREDO_MEASUREMENT_FIXTURE_ID",
  fixture_sha256: "ALFREDO_MEASUREMENT_FIXTURE_SHA256",
  source_sha256: "ALFREDO_MEASUREMENT_SOURCE_SHA256",
  artifact_sha256: "ALFREDO_MEASUREMENT_ARTIFACT_SHA256",
  variant: "ALFREDO_MEASUREMENT_VARIANT",
  workflow: "ALFREDO_MEASUREMENT_WORKFLOW",
  mode: "ALFREDO_MEASUREMENT_MODE",
});
const CONTROL_PATH_VARIABLE = "ALFREDO_MEASUREMENT_CONTROL_PATH";

function requiredString(value, label) {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function validateIdentity(metadata) {
  for (const field of Object.keys(ENVIRONMENT_FIELDS)) {
    requiredString(metadata[field], field);
  }
  if (!isAbsolute(metadata.jsonl_path)) {
    throw new Error("jsonl_path must be absolute");
  }
  for (const field of ["fixture_sha256", "source_sha256", "artifact_sha256"]) {
    if (!SHA256_PATTERN.test(metadata[field])) {
      throw new Error(`${field} must be a lowercase SHA-256`);
    }
  }
  if (!["process-cold", "process-warm"].includes(metadata.mode)) {
    throw new Error("mode must be process-cold or process-warm");
  }
  if (metadata.desktop_pid !== undefined || metadata.desktop_session_id !== undefined) {
    if (!Number.isSafeInteger(metadata.desktop_pid) || metadata.desktop_pid <= 0) {
      throw new Error("desktop_pid must be a positive integer");
    }
    requiredString(metadata.desktop_session_id, "desktop_session_id");
  }
}

function checkedOutputPath(path) {
  const absolute = resolve(path);
  realpathSync(dirname(absolute));
  try {
    const entry = lstatSync(absolute);
    if (!entry.isFile() || entry.isSymbolicLink()) {
      throw new Error(`measurement output must be a regular non-symlink file: ${absolute}`);
    }
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  return absolute;
}

export function performanceEnvironment(environment = process.env) {
  const controlPath = environment[CONTROL_PATH_VARIABLE];
  const legacyPresent = Object.values(ENVIRONMENT_FIELDS).filter(
    (variable) => typeof environment[variable] === "string" && environment[variable].trim(),
  );
  if (typeof controlPath === "string" && controlPath.trim()) {
    if (legacyPresent.length > 0) {
      throw new Error(`${CONTROL_PATH_VARIABLE} must not be combined with legacy measurement identity`);
    }
    if (!isAbsolute(controlPath)) throw new Error(`${CONTROL_PATH_VARIABLE} must be absolute`);
    realpathSync(dirname(controlPath));
    let entry;
    try {
      entry = lstatSync(controlPath);
    } catch (error) {
      if (error?.code === "ENOENT") return null;
      throw error;
    }
    if (!entry.isFile() || entry.isSymbolicLink()) {
      throw new Error(`${CONTROL_PATH_VARIABLE} must be a regular non-symlink file`);
    }
    let metadata;
    try {
      metadata = JSON.parse(readFileSync(controlPath, "utf8"));
    } catch (error) {
      throw new Error(`measurement control file is invalid: ${error.message}`);
    }
    if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) {
      throw new Error("measurement control file must contain one JSON object");
    }
    const allowed = new Set([
      ...Object.keys(ENVIRONMENT_FIELDS),
      "desktop_pid",
      "desktop_session_id",
    ]);
    const unknown = Object.keys(metadata).filter((field) => !allowed.has(field));
    if (unknown.length > 0) {
      throw new Error(`measurement control file has unknown fields: ${unknown.join(", ")}`);
    }
    validateIdentity(metadata);
    return metadata;
  }
  const metadata = {};
  let present = 0;
  const missing = [];
  for (const [field, variable] of Object.entries(ENVIRONMENT_FIELDS)) {
    const value = environment[variable];
    if (typeof value !== "string" || !value.trim()) {
      missing.push(variable);
      continue;
    }
    present += 1;
    metadata[field] = value;
  }
  if (present === 0) return null;
  if (missing.length > 0) {
    throw new Error(`measurement environment is incomplete: missing ${missing.join(", ")}`);
  }
  validateIdentity(metadata);
  return metadata;
}

export function createPerformanceRecorder(options) {
  const {
    source,
    clock_id: clockId,
    monotonic_now_ns: monotonicNowNs = process.hrtime.bigint,
    ...metadata
  } = options;
  validateIdentity(metadata);
  requiredString(source, "source");
  requiredString(clockId, "clock_id");
  const outputPath = checkedOutputPath(metadata.jsonl_path);
  let lastTick = null;

  return {
    mark(stage, boundary, detail = {}) {
      if (!STAGES.has(stage)) throw new Error(`unknown measurement stage: ${stage}`);
      if (boundary !== "start" && boundary !== "end") {
        throw new Error(`unknown measurement boundary: ${boundary}`);
      }
      if (!detail || typeof detail !== "object" || Array.isArray(detail)) {
        throw new Error("measurement detail must be an object");
      }
      const tick = monotonicNowNs();
      if (typeof tick !== "bigint" || tick < 0n) {
        throw new Error("monotonic clock must return a non-negative bigint");
      }
      if (lastTick !== null && tick < lastTick) {
        throw new Error("monotonic clock moved backwards");
      }
      lastTick = tick;
      const { jsonl_path: _jsonlPath, ...identity } = metadata;
      const record = {
        schema_version: 1,
        record_type: "stage-mark",
        ...identity,
        source,
        clock_id: clockId,
        stage,
        boundary,
        monotonic_ns: tick.toString(),
        detail,
      };
      const line = `${JSON.stringify(record)}\n`;
      if (Buffer.byteLength(line) > 16_384) {
        throw new Error("measurement stage mark exceeds 16 KiB");
      }
      appendFileSync(outputPath, line, { encoding: "utf8", mode: 0o600, flag: "a" });
      return record;
    },
  };
}
