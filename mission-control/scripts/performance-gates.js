#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  createReadStream,
  lstatSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, resolve } from "node:path";

import {
  canonicalTreeSha256,
  materializeCanonicalFixture,
} from "./performance-lifecycle.js";

const sourceRoot = process.env.ALFREDO_GATE_SOURCE_ROOT
  ? resolve(process.env.ALFREDO_GATE_SOURCE_ROOT)
  : resolve(import.meta.dirname, "../..");
const missionControl = resolve(sourceRoot, "mission-control");

export const GATE_STEPS = Object.freeze({
  contract: Object.freeze([
    Object.freeze({
      cwd: missionControl,
      argv: Object.freeze([
        "npm",
        "test",
        "--",
        "--run",
        "src/alfredo-release-seam.test.tsx",
        "src/App.test.tsx",
        "src/workspace-client.test.ts",
      ]),
    }),
    Object.freeze({
      cwd: sourceRoot,
      argv: Object.freeze([
        "python3",
        "-m",
        "unittest",
        "tests.test_orchestrator_server.PersistentOrchestratorServerTests",
        "tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_approved_ad_hoc_delegation_launches_bounded_session_without_issue_slice",
        "tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_concurrent_workspace_queue_decisions_accept_only_one_same_revision",
      ]),
    }),
  ]),
  replay: Object.freeze([
    Object.freeze({
      cwd: sourceRoot,
      argv: Object.freeze([
        "python3",
        "-m",
        "unittest",
        "tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_ad_hoc_proposal_and_approval_replay_after_lost_queue_write_response",
        "tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_queue_replay_rejects_forged_inner_acknowledgement_boundaries",
      ]),
    }),
  ]),
  crash_cut: Object.freeze([
    Object.freeze({
      cwd: sourceRoot,
      argv: Object.freeze([
        "python3",
        "-m",
        "unittest",
        "tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_workspace_queue_rejects_and_stale_decisions_preserve_authoritative_state",
        "tests.test_performance_correctness.PerformanceCorrectnessTests.test_ad_hoc_decision_crash_before_first_durable_write_preserves_old_state",
        "tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_ad_hoc_proposal_and_approval_replay_after_lost_queue_write_response",
        "tests.test_workspace_snapshot.WorkspaceSnapshotTest.test_queue_replay_repairs_only_the_missing_orchestrator_audit_phase",
      ]),
    }),
  ]),
  packaging: Object.freeze([
    Object.freeze({
      cwd: missionControl,
      argv: Object.freeze([
        "cargo",
        "build",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--release",
        "--no-default-features",
        "--bin",
        "alfredo-execution-provider",
      ]),
      environment: Object.freeze({
        CARGO_TARGET_DIR: resolve(missionControl, "src-tauri", "target"),
      }),
    }),
    Object.freeze({
      cwd: missionControl,
      argv: Object.freeze(["npm", "run", "release:verify", "--", "--artifact"]),
      append_artifact_path: true,
      append_shadow_provider_path: true,
    }),
    Object.freeze({ cwd: missionControl, argv: Object.freeze(["npm", "run", "release:verify"]) }),
    Object.freeze({ cwd: missionControl, argv: Object.freeze(["npm", "run", "release:check"]) }),
  ]),
  rollback: Object.freeze([
    Object.freeze({
      argv: Object.freeze(["--version"]),
      cwd: sourceRoot,
      use_artifact_executable: true,
      environment: Object.freeze({ ALFREDO_RUST_CANDIDATE_ENABLED: "0" }),
    }),
    Object.freeze({
      fail_closed_reason:
        "production Rust-candidate rollback is unavailable; the prototype is not eligible evidence",
    }),
  ]),
});

function requiredEnvironment(name) {
  const value = process.env[name];
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function sha256File(path) {
  const hash = createHash("sha256");
  return new Promise((resolveHash, reject) => {
    const stream = createReadStream(path);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("error", reject);
    stream.on("end", () => resolveHash(hash.digest("hex")));
  });
}

async function verifiedInputPath(pathVariable, shaVariable) {
  const value = requiredEnvironment(pathVariable);
  if (!isAbsolute(value)) throw new Error(`${pathVariable} must be absolute`);
  const entry = lstatSync(value);
  if (!entry.isFile() || entry.isSymbolicLink()) {
    throw new Error(`${pathVariable} must be a regular non-symlink file`);
  }
  const path = realpathSync(value);
  const actual = await sha256File(path);
  if (actual !== requiredEnvironment(shaVariable)) {
    throw new Error(`${pathVariable} does not match ${shaVariable}`);
  }
  return path;
}

export async function verifyGateInputs() {
  return {
    artifact_path: await verifiedInputPath(
      "ALFREDO_GATE_ARTIFACT_PATH",
      "ALFREDO_GATE_ARTIFACT_SHA256",
    ),
    fixture_path: await verifiedInputPath(
      "ALFREDO_GATE_FIXTURE_PATH",
      "ALFREDO_GATE_FIXTURE_SHA256",
    ),
  };
}

function runStep(step, inputs) {
  if (step.fail_closed_reason) throw new Error(step.fail_closed_reason);
  const [declaredExecutable, ...declaredArgs] = step.argv;
  const executable = step.use_artifact_executable
    ? inputs.artifact_path
    : declaredExecutable;
  const baseArgs = step.use_artifact_executable ? step.argv : declaredArgs;
  let args = step.append_artifact_path
    ? [...baseArgs, inputs.artifact_path]
    : baseArgs;
  if (step.append_shadow_provider_path) {
    args = [
      ...args,
      "--provider",
      resolve(missionControl, "src-tauri", "target", "release", "alfredo-execution-provider"),
    ];
  }
  const result = spawnSync(executable, args, {
    cwd: step.cwd,
    env: { ...process.env, ...(step.environment ?? {}) },
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    timeout: 20 * 60 * 1000,
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error || result.status !== 0 || result.signal) {
    const output = `${result.error?.message ?? ""}\n${result.stdout ?? ""}\n${result.stderr ?? ""}`
      .trim()
      .slice(-16_384);
    throw new Error(
      `${[executable, ...args].join(" ")} failed ` +
        `(code=${result.status ?? "null"}, signal=${result.signal ?? "none"}): ${output}`,
    );
  }
  if (step.require_rollback_pass) {
    const report = JSON.parse(result.stdout);
    if (report?.rollback?.passed !== true) {
      throw new Error("rollback runner did not prove rollback.passed=true");
    }
  }
}

function canonicalFilesFromDisk(fixture, installRoot) {
  return fixture.canonical_files.map((entry) => ({
    ...entry,
    content: readFileSync(resolve(installRoot, entry.path), "utf8"),
  }));
}

function initializeFixtureWorkspace(installRoot) {
  const git = spawnSync("git", ["init", "--quiet", resolve(installRoot, "workspace")], {
    encoding: "utf8",
  });
  if (git.status !== 0) throw new Error(`fixture Git initialization failed: ${git.stderr}`);
}

function fixtureCommandArgs(fixture, installRoot, command) {
  const args = [
    "-m",
    "albert_mvp",
    command,
    "--target-repo",
    resolve(installRoot, "workspace"),
    "--tracker-dir",
    resolve(installRoot, "tracker"),
    "--runtime-root",
    resolve(installRoot, "runtime"),
  ];
  if (fixture.identities?.mission_id) {
    args.push("--mission-id", fixture.identities.mission_id);
  }
  const agentConfig = resolve(installRoot, "workspace/.albert/agents.json");
  try {
    if (lstatSync(agentConfig).isFile()) args.push("--agent-config", agentConfig);
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  return args;
}

function runFixtureCommand(fixture, installRoot, command, extraArgs = []) {
  const result = spawnSync(
    "python3",
    [...fixtureCommandArgs(fixture, installRoot, command), ...extraArgs],
    { cwd: sourceRoot, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
  );
  if (result.status !== 0) {
    throw new Error(`canonical fixture public ${command} failed: ${result.stderr}`);
  }
  return JSON.parse(result.stdout);
}

function bindFixtureRuntimeIdentity(fixture, installRoot) {
  const oldKey = fixture.identities?.workspace_session_id;
  const missionId = fixture.identities?.mission_id;
  if (!oldKey || !missionId) return;
  const identity = [
    resolve(installRoot, "workspace"),
    resolve(installRoot, "tracker"),
    resolve(installRoot, "tracker/issues"),
    missionId,
  ].join("\n");
  const newKey = `workspace-${createHash("sha1").update(identity).digest("hex").slice(0, 8)}`;
  if (newKey === oldKey) return;
  const oldRuntime = resolve(installRoot, "runtime", oldKey);
  const newRuntime = resolve(installRoot, "runtime", newKey);
  renameSync(oldRuntime, newRuntime);
  const statePath = resolve(newRuntime, "runtime.json");
  const state = JSON.parse(readFileSync(statePath, "utf8"));
  state.project_key = newKey;
  writeFileSync(statePath, `${JSON.stringify(state, null, 2)}\n`, "utf8");
}

function runPendingQueueFixtureContract(fixture, scratchRoot) {
  for (const decision of ["defer", "approve"]) {
    const installRoot = resolve(scratchRoot, `queue-${decision}`);
    materializeCanonicalFixture({ fixture, scratchRoot, installRoot });
    bindFixtureRuntimeIdentity(fixture, installRoot);
    initializeFixtureWorkspace(installRoot);
    const acknowledgement = runFixtureCommand(
      fixture,
      installRoot,
      "workspace-queue-decision",
      [
        "--correlation-id",
        `gate-${decision}-001`,
        "--expected-queue-revision",
        String(fixture.identities.expected_revision),
        "--item-id",
        fixture.identities.queue_item_id,
        "--decision",
        decision,
        "--reason",
        `Fixed correctness gate ${decision}.`,
      ],
    );
    const expectedStatus = decision === "approve" ? "approved" : "deferred";
    if (acknowledgement.item_status !== expectedStatus) {
      throw new Error(`canonical fixture Queue ${decision} returned the wrong status`);
    }
    if (decision === "approve" && !acknowledgement.session_id) {
      throw new Error("canonical fixture Queue approval did not create one queued session");
    }
  }
}

function runFixtureContract(inputs) {
  const fixture = JSON.parse(readFileSync(inputs.fixture_path, "utf8"));
  const scratchRoot = mkdtempSync(resolve(tmpdir(), "alfredo-gate-fixture-"));
  const installRoot = resolve(scratchRoot, "installed");
  materializeCanonicalFixture({ fixture, scratchRoot, installRoot });
  initializeFixtureWorkspace(installRoot);
  const projection = runFixtureCommand(fixture, installRoot, "workspace-snapshot");
  if (projection.schema_version !== 1) {
    rmSync(scratchRoot, { recursive: true, force: true });
    throw new Error("canonical fixture public snapshot returned the wrong schema");
  }
  if (fixture.kind === "pending-ad-hoc") {
    runPendingQueueFixtureContract(fixture, scratchRoot);
  }
  return { fixture, installRoot, scratchRoot };
}

function verifyFixtureUnchanged(context) {
  const actual = canonicalTreeSha256(
    canonicalFilesFromDisk(context.fixture, context.installRoot),
  );
  if (actual !== context.fixture.canonical_tree_sha256) {
    throw new Error("correctness gate changed the canonical fixture bytes");
  }
}

export async function runRepositoryGate(gate) {
  const steps = GATE_STEPS[gate];
  if (!steps) throw new Error(`unknown repository correctness gate: ${gate}`);
  const inputs = await verifyGateInputs();
  const fixtureContext = runFixtureContract(inputs);
  try {
    for (const step of steps) runStep(step, inputs);
    verifyFixtureUnchanged(fixtureContext);
  } finally {
    rmSync(fixtureContext.scratchRoot, { recursive: true, force: true });
  }

  const receipt = {
    gate,
    status: "pass",
    source_sha256: requiredEnvironment("ALFREDO_GATE_SOURCE_SHA256"),
    artifact_sha256: requiredEnvironment("ALFREDO_GATE_ARTIFACT_SHA256"),
    fixture_sha256: requiredEnvironment("ALFREDO_GATE_FIXTURE_SHA256"),
  };
  writeFileSync(
    requiredEnvironment("ALFREDO_GATE_RECEIPT_PATH"),
    `${JSON.stringify(receipt)}\n`,
    { encoding: "utf8", mode: 0o600, flag: "wx" },
  );
  return receipt;
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(import.meta.filename)) {
  runRepositoryGate(process.argv[2] ?? "")
    .then((receipt) => process.stdout.write(`${JSON.stringify(receipt)}\n`))
    .catch((error) => {
      process.stderr.write(`Error: ${error.message}\n`);
      process.exitCode = 1;
    });
}
