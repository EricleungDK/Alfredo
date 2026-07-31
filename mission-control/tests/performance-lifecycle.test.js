import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import test from "node:test";

import {
  canonicalTreeSha256,
  createProductionLifecycle,
  materializeCanonicalFixture,
} from "../scripts/performance-lifecycle.js";
import { validateWarmSessionConfig } from "../scripts/performance-driver.js";

test("warm-session preflight binds all lifecycle paths to scratch and the installed launcher", () => {
  const scratchRoot = mkdtempSync(resolve(tmpdir(), "alfredo-warm-preflight-"));
  const checked = validateWarmSessionConfig(
    {
      launch_command: [process.execPath, "--version"],
      fixture_install_root: resolve(scratchRoot, "fixture"),
      ready_marker_path: resolve(scratchRoot, "ready.json"),
      control_path: resolve(scratchRoot, "control.json"),
    },
    {
      variantName: "python",
      installedLauncherPath: process.execPath,
      scratchRoot,
    },
  );

  assert.equal(checked.launch_command[0], process.execPath);
  assert.equal(checked.timeout_ms, 120_000);
  assert.throws(
    () => validateWarmSessionConfig(
      {
        ...checked,
        fixture_install_root: resolve(scratchRoot, "../outside"),
      },
      {
        variantName: "python",
        installedLauncherPath: process.execPath,
        scratchRoot,
      },
    ),
    /must stay inside scratch_root/,
  );
});

test("canonical fixture materialization restores fresh byte-identical state", () => {
  const scratchRoot = mkdtempSync(resolve(tmpdir(), "alfredo-fixture-lifecycle-"));
  const installRoot = resolve(scratchRoot, "installed-fixture");
  const canonicalFiles = [
    { path: "tracker/PRD.md", mode: 0o644, content: "# Performance Mission\n" },
    {
      path: "runtime/workspace-fixed/workspace-queue.json",
      mode: 0o600,
      content: '{"schema_version":1,"revision":7,"items":[]}\n',
    },
  ];
  const fixture = {
    fixture_id: "pending-ad-hoc-v1",
    canonical_files: canonicalFiles,
    canonical_tree_sha256:
      "33420f0f17f9a71c833f4180e938501e306686c1aea178a5754cdb74ba5961f6",
  };

  assert.equal(canonicalTreeSha256(canonicalFiles), fixture.canonical_tree_sha256);

  const first = materializeCanonicalFixture({ fixture, scratchRoot, installRoot });
  writeFileSync(
    resolve(installRoot, "runtime/workspace-fixed/workspace-queue.json"),
    "mutated\n",
    "utf8",
  );
  const second = materializeCanonicalFixture({ fixture, scratchRoot, installRoot });

  assert.deepEqual(second, first);
  assert.equal(second.file_count, 2);
  assert.equal(
    readFileSync(resolve(installRoot, "runtime/workspace-fixed/workspace-queue.json"), "utf8"),
    canonicalFiles[1].content,
  );
  assert.equal(second.canonical_tree_sha256, fixture.canonical_tree_sha256);
});

test("canonical fixture materialization rejects traversal and targets outside scratch", () => {
  const scratchRoot = mkdtempSync(resolve(tmpdir(), "alfredo-fixture-boundary-"));
  const fixture = {
    fixture_id: "bad-v1",
    canonical_files: [{ path: "../escape", mode: 0o600, content: "no" }],
    canonical_tree_sha256: "a".repeat(64),
  };

  assert.throws(
    () =>
      materializeCanonicalFixture({
        fixture,
        scratchRoot,
        installRoot: resolve(scratchRoot, "fixture"),
      }),
    /canonical file path escapes the fixture root/,
  );
  assert.throws(
    () =>
      materializeCanonicalFixture({
        fixture: { ...fixture, canonical_files: [] },
        scratchRoot,
        installRoot: resolve(scratchRoot, "../outside"),
      }),
    /fixture install root must stay inside scratch_root/,
  );
});

test("process-cold preparation uses a run-owned fixture root inside scratch", async () => {
  const scratchRoot = mkdtempSync(resolve(tmpdir(), "alfredo-cold-lifecycle-"));
  const fixture = {
    fixture_id: "minimal-ready-v1",
    installation_root: "/tmp/shared-path-that-must-not-be-used",
    canonical_files: [
      { path: "workspace/README.md", mode: 0o644, content: "cold fixture\n" },
    ],
    canonical_tree_sha256:
      "a9d805f9d4d5b0da5535d14a62359566081fe2ad64b111a8b14f53a99414c123",
  };
  const lifecycle = createProductionLifecycle({ scratch_root: scratchRoot });
  const prepared = await lifecycle.prepareSample({
    fixture,
    variant: { name: "python" },
  });

  assert.equal(
    prepared.fixture_proof.install_root,
    resolve(scratchRoot, "process-cold", "minimal-ready-v1"),
  );
  assert.equal(
    readFileSync(resolve(prepared.fixture_proof.install_root, "workspace/README.md"), "utf8"),
    "cold fixture\n",
  );
});

test("process-warm lifecycle keeps one verified desktop while restoring and arming each sample", async () => {
  const scratchRoot = mkdtempSync(resolve(tmpdir(), "alfredo-warm-lifecycle-"));
  const installRoot = resolve(scratchRoot, "fixture");
  const readyMarker = resolve(scratchRoot, "ready.json");
  const controlPath = resolve(scratchRoot, "measurement-control.json");
  const rawPath = resolve(scratchRoot, "raw.jsonl");
  const fixture = {
    fixture_id: "pending-ad-hoc-v1",
    canonical_files: [
      { path: "workspace/README.md", mode: 0o644, content: "fixture\n" },
      { path: "runtime/state.json", mode: 0o600, content: '{"revision":7}\n' },
    ],
    canonical_tree_sha256:
      "c7315fab8d44020b8af6a70b4c21085c4640f1a39c13e81708c8cc01ffab8be0",
  };
  const launcher = resolve(scratchRoot, "fake-installed-launcher.js");
  writeFileSync(
    launcher,
    "#!/usr/bin/env node\n" +
      "const fs=require('node:fs');" +
      "fs.writeFileSync(process.env.ALFREDO_WARM_READY_MARKER,JSON.stringify({" +
      "schema_version:1,status:'ready',process_id:process.pid," +
      "desktop_session_id:process.env.ALFREDO_MEASUREMENT_DESKTOP_SESSION_ID}));" +
      "setInterval(()=>{},1000);\n",
    "utf8",
  );
  chmodSync(launcher, 0o755);
  const lifecycle = createProductionLifecycle({ scratch_root: scratchRoot });
  const variant = {
    name: "python",
    installed_launcher_path: launcher,
    warm_session: {
      launch_command: [launcher],
      ready_marker_path: readyMarker,
      control_path: controlPath,
      fixture_install_root: installRoot,
      timeout_ms: 5_000,
    },
  };
  const session = await lifecycle.startWarmSession({
    run_id: "warm-run",
    cohort_id: "warm-cohort",
    variant,
    fixture,
  });
  try {
    const firstRequest = {
      run_id: "warm-run", raw_jsonl: rawPath, cohort_id: "warm-cohort",
      sample_id: "sample-1", pair_id: "pair-1", correlation_id: "correlation-1",
      fixture, variant, workflow: "queue-defer", mode: "process-warm", metric: "R0->R5",
    };
    const first = await lifecycle.prepareSample(firstRequest, session);
    writeFileSync(resolve(installRoot, "runtime/state.json"), "mutated\n", "utf8");
    const second = await lifecycle.prepareSample(
      { ...firstRequest, sample_id: "sample-2", correlation_id: "correlation-2" },
      session,
    );

    assert.equal(first.desktop_pid, second.desktop_pid);
    assert.equal(first.desktop_session_id, second.desktop_session_id);
    assert.equal(readFileSync(resolve(installRoot, "runtime/state.json"), "utf8"), '{"revision":7}\n');
    const armed = JSON.parse(readFileSync(controlPath, "utf8"));
    assert.equal(armed.sample_id, "sample-2");
    assert.equal(armed.desktop_pid, session.desktop_pid);
  } finally {
    await lifecycle.stopWarmSession(session);
  }
});
