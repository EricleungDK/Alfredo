import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";

import { expect, test } from "vitest";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const expectedMetaPackFiles = [
  "README.md",
  "bin/alfredo.js",
  "bin/desktop-adapter.js",
  "bundled-backend/.albert/agents.json",
  "bundled-backend/albert_mvp/__init__.py",
  "bundled-backend/albert_mvp/__main__.py",
  "bundled-backend/albert_mvp/agents.py",
  "bundled-backend/albert_mvp/capabilities.py",
  "bundled-backend/albert_mvp/cli.py",
  "bundled-backend/albert_mvp/core.py",
  "bundled-backend/albert_mvp/execution.py",
  "bundled-backend/albert_mvp/execution_cutover.py",
  "bundled-backend/albert_mvp/local_agent_execution_cutover.py",
  "bundled-backend/albert_mvp/execution_shadow.py",
  "bundled-backend/albert_mvp/inference.py",
  "bundled-backend/albert_mvp/inference_qualification.py",
  "bundled-backend/albert_mvp/performance.py",
  "bundled-backend/albert_mvp/process_supervisor.py",
  "bundled-backend/albert_mvp/retirement.py",
  "bundled-backend/albert_mvp/server.py",
  "bundled-backend/albert_mvp/tui.py",
  "bundled-backend/albert_mvp/workspace.py",
  "bundled-backend/albert_mvp/workspace_selection.py",
  "package.json",
  "scripts/performance-recorder.js",
].sort();
const expectedPlatformPackFiles = [
  "README.md",
  "bin/alfredo-desktop.AppImage",
  "bin/alfredo-execution-provider",
  "desktop.json",
  "package.json",
].sort();

function writeBackendFixture(root, controllerId) {
  const backendRoot = resolve(root, "backend");
  mkdirSync(resolve(backendRoot, "albert_mvp"), { recursive: true });
  mkdirSync(resolve(backendRoot, ".albert"), { recursive: true });
  writeFileSync(resolve(backendRoot, "albert_mvp", "__init__.py"), "", "utf8");
  writeFileSync(
    resolve(backendRoot, ".albert", "agents.json"),
    `${JSON.stringify({
      agents: [
        {
          id: controllerId,
          role: "frontier",
          provider: "fake",
          runner: "fake",
          model: "fixture-controller",
          routing: "controller",
          availability: "available",
          assignable: false,
          delegate_only: false,
          requires_approval: false,
        },
      ],
    })}\n`,
    "utf8",
  );
  return backendRoot;
}

function withoutBackendOverrides(overrides) {
  const environment = { ...process.env, ...overrides };
  delete environment.ALBERT_BACKEND_ROOT;
  delete environment.ALFREDO_AGENT_CONFIG;
  return environment;
}

function withBackendOverride(backendRoot, overrides) {
  return {
    ...withoutBackendOverrides(overrides),
    ALBERT_BACKEND_ROOT: backendRoot,
  };
}

test("the npm-installed package exposes PATH launch backed by a shipped native desktop", () => {
  const root = mkdtempSync(join(tmpdir(), "alfredo-installed-package-"));
  const packDestination = resolve(root, "packed");
  const releaseRoot = resolve(projectRoot, "release", "out");
  const metaPackageRoot = resolve(releaseRoot, "alfredo-agent");
  const platformPackageRoot = resolve(releaseRoot, "alfredo-agent-linux-x64-gnu");
  const installPrefix = resolve(root, "prefix");
  const installedPackageRoot = resolve(
    installPrefix,
    "lib",
    "node_modules",
    "alfredo-agent",
  );
  const bundledBackendRoot = resolve(installedPackageRoot, "bundled-backend");
  const installedPlatformRoot = resolve(
    installPrefix,
    "lib",
    "node_modules",
    "alfredo-agent-linux-x64-gnu",
  );
  const installedBinRoot = resolve(installPrefix, "bin");
  const nativeExecutable = resolve(installedPlatformRoot, "bin", "alfredo-desktop.AppImage");
  const shadowProvider = resolve(installedPlatformRoot, "bin", "alfredo-execution-provider");
  const workspace = resolve(root, "workspace");
  const runtimeRoot = resolve(root, "runtime");
  const fixtureArtifact = resolve(root, "alfredo-desktop.AppImage");
  const fixtureProvider = resolve(root, "alfredo-execution-provider");
  mkdirSync(packDestination, { recursive: true });
  mkdirSync(workspace, { recursive: true });
  try {
    writeFileSync(
      fixtureArtifact,
      "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'Alfredo Desktop 0.1.0\\n'\nelse\n  printf 'fixture GUI\\n'\nfi\n",
      "utf8",
    );
    chmodSync(fixtureArtifact, 0o755);
    writeFileSync(
      fixtureProvider,
      "#!/bin/sh\nwhile IFS= read -r _line; do\n  printf '%s\\n' '{\"ok\":false,\"failure\":{\"code\":\"contract-failure\",\"message\":\"fixture rejected invalid request\",\"recoverable\":true}}'\ndone\n",
      "utf8",
    );
    chmodSync(fixtureProvider, 0o755);
    const built = spawnSync(
      process.execPath,
      [
        resolve(projectRoot, "scripts", "build-npm-release.js"),
        "build",
        "--artifact",
        fixtureArtifact,
        "--provider",
        fixtureProvider,
      ],
      {
        cwd: projectRoot,
        encoding: "utf8",
      },
    );
    expect(built.status, built.stderr || built.stdout).toBe(0);

    const packedMeta = spawnSync(
      npmCommand,
      ["pack", "--json", "--pack-destination", packDestination],
      {
        cwd: metaPackageRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          npm_config_cache: resolve(root, "npm-cache"),
        },
      },
    );
    expect(packedMeta.status, packedMeta.stderr || packedMeta.stdout).toBe(0);
    const [metaPackRecord] = JSON.parse(packedMeta.stdout);
    const { filename: metaFilename } = metaPackRecord;
    expect(metaPackRecord.files.map((entry) => entry.path).sort()).toEqual(expectedMetaPackFiles);
    const metaTarball = resolve(packDestination, metaFilename);

    const packedPlatform = spawnSync(
      npmCommand,
      ["pack", "--json", "--pack-destination", packDestination],
      {
        cwd: platformPackageRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          npm_config_cache: resolve(root, "npm-cache"),
        },
      },
    );
    expect(packedPlatform.status, packedPlatform.stderr || packedPlatform.stdout).toBe(0);
    const [platformPackRecord] = JSON.parse(packedPlatform.stdout);
    const { filename: platformFilename } = platformPackRecord;
    expect(platformPackRecord.files.map((entry) => entry.path).sort()).toEqual(
      expectedPlatformPackFiles,
    );
    const platformTarball = resolve(packDestination, platformFilename);

    const installed = spawnSync(
      npmCommand,
      [
        "install",
        "--global",
        "--ignore-scripts",
        "--prefix",
        installPrefix,
        metaTarball,
        platformTarball,
      ],
      {
        encoding: "utf8",
        env: {
          ...process.env,
          npm_config_cache: resolve(root, "npm-cache"),
        },
      },
    );
    expect(installed.status, installed.stderr || installed.stdout).toBe(0);

    expect(existsSync(nativeExecutable)).toBe(true);
    expect(existsSync(shadowProvider)).toBe(true);
    expect(existsSync(resolve(bundledBackendRoot, "albert_mvp", "__main__.py"))).toBe(true);
    expect(existsSync(resolve(bundledBackendRoot, ".albert", "agents.json"))).toBe(true);
    expect(existsSync(resolve(projectRoot, "bundled-backend"))).toBe(false);

    const siblingBackendRoot = resolve(installedPackageRoot, "..");
    mkdirSync(resolve(siblingBackendRoot, "albert_mvp"), { recursive: true });
    mkdirSync(resolve(siblingBackendRoot, ".albert"), { recursive: true });
    writeFileSync(resolve(siblingBackendRoot, "albert_mvp", "__init__.py"), "", "utf8");
    writeFileSync(
      resolve(siblingBackendRoot, ".albert", "agents.json"),
      `${JSON.stringify({ agents: [{ id: "confusing-sibling" }] })}\n`,
      "utf8",
    );

    const backendHelp = spawnSync(
      process.env.ALBERT_PYTHON ?? (process.platform === "win32" ? "python" : "python3"),
      ["-m", "albert_mvp", "--help"],
      {
        cwd: bundledBackendRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          PYTHONPATH: bundledBackendRoot,
        },
      },
    );
    expect(backendHelp.status, backendHelp.stderr || backendHelp.stdout).toBe(0);

    const installedPath = `${installedBinRoot}${delimiter}${process.env.PATH ?? ""}`;
    const installedVersion = spawnSync("alfredo", ["--version"], {
      cwd: workspace,
      encoding: "utf8",
      env: { ...process.env, PATH: installedPath },
    });
    expect(installedVersion.status, installedVersion.stderr || installedVersion.stdout).toBe(0);
    expect(installedVersion.stdout).toBe("Alfredo 0.1.0\n");
    const launched = spawnSync(
      "alfredo",
      [],
      {
        cwd: workspace,
        encoding: "utf8",
        env: withoutBackendOverrides({
          ALFREDO_DESKTOP_DRY_RUN: "1",
          ALFREDO_RUNTIME_ROOT: runtimeRoot,
          PATH: installedPath,
        }),
      },
    );
    expect(launched.status, launched.stderr || launched.stdout).toBe(0);
    expect(JSON.parse(launched.stdout)).toMatchObject({
      project_root: installedPackageRoot,
      backend_root: bundledBackendRoot,
      starting_location: workspace,
      workspace_selection: {
        schema_version: 1,
        phase: "selection-required",
        starting_location: workspace,
        coding_workspace: null,
        active_mission: null,
      },
      selected_agent: "qwen3-14b",
      selected_model: "qwen3:14b",
    });

    const desktopLaunch = spawnSync(
      "alfredo",
      [],
      {
        cwd: workspace,
        encoding: "utf8",
        env: withoutBackendOverrides({
          ALFREDO_DESKTOP_DRY_RUN: "launch",
          ALFREDO_RUNTIME_ROOT: resolve(root, "launch-runtime"),
          PATH: installedPath,
        }),
      },
    );
    expect(desktopLaunch.status, desktopLaunch.stderr || desktopLaunch.stdout).toBe(0);
    expect(JSON.parse(desktopLaunch.stdout)).toMatchObject({
      command: [nativeExecutable],
      cwd: workspace,
      env: {
        APPIMAGE_EXTRACT_AND_RUN: "1",
      },
    });

    const headlessProbeRoot = writeBackendFixture(root, "headless-probe");
    writeFileSync(
      resolve(headlessProbeRoot, "albert_mvp", "__main__.py"),
      [
        "import json",
        "import os",
        "print(json.dumps({key: os.environ.get(key, '') for key in (",
        "    'ALFREDO_RUST_CANDIDATE_ENABLED',",
        "    'ALFREDO_RUST_LOCAL_AGENT_ENABLED',",
        "    'ALFREDO_RUST_EXECUTION_PROVIDER',",
        "    'ALFREDO_RUST_EXECUTION_PROVIDER_SHA256',",
        ")}))",
        "",
      ].join("\n"),
      "utf8",
    );
    const headlessCutover = spawnSync(
      "alfredo",
      ["run", "--agent", "headless-probe", "inspect provider environment"],
      {
        cwd: workspace,
        encoding: "utf8",
        env: withBackendOverride(headlessProbeRoot, {
          PATH: installedPath,
        }),
      },
    );
    expect(headlessCutover.status, headlessCutover.stderr || headlessCutover.stdout).toBe(0);
    expect(JSON.parse(headlessCutover.stdout)).toEqual({
      ALFREDO_RUST_CANDIDATE_ENABLED: "1",
      ALFREDO_RUST_LOCAL_AGENT_ENABLED: "1",
      ALFREDO_RUST_EXECUTION_PROVIDER: shadowProvider,
      ALFREDO_RUST_EXECUTION_PROVIDER_SHA256: createHash("sha256")
        .update(readFileSync(shadowProvider))
        .digest("hex"),
    });

    chmodSync(shadowProvider, 0o644);
    const headlessFallback = spawnSync(
      "alfredo",
      ["run", "--agent", "headless-probe", "inspect fallback environment"],
      {
        cwd: workspace,
        encoding: "utf8",
        env: withBackendOverride(headlessProbeRoot, {
          ALFREDO_RUST_LOCAL_AGENT_ENABLED: "0",
          PATH: installedPath,
        }),
      },
    );
    expect(headlessFallback.status, headlessFallback.stderr || headlessFallback.stdout).toBe(0);
    expect(JSON.parse(headlessFallback.stdout)).toEqual({
      ALFREDO_RUST_CANDIDATE_ENABLED: "1",
      ALFREDO_RUST_LOCAL_AGENT_ENABLED: "0",
      ALFREDO_RUST_EXECUTION_PROVIDER: "",
      ALFREDO_RUST_EXECUTION_PROVIDER_SHA256: "",
    });
    chmodSync(shadowProvider, 0o755);

    const exactNoArgumentLaunch = spawnSync("alfredo", [], {
      cwd: workspace,
      encoding: "utf8",
      env: withoutBackendOverrides({
        ALFREDO_RUNTIME_ROOT: resolve(root, "native-preflight-runtime"),
        PATH: [installedBinRoot, dirname(process.execPath), "/usr/bin", "/bin"].join(delimiter),
      }),
      timeout: 30_000,
    });
    expect(exactNoArgumentLaunch.status, exactNoArgumentLaunch.stderr).toBe(0);
    expect(exactNoArgumentLaunch.stdout).toContain("fixture GUI");

    const mismatchedNative = "#!/bin/sh\nprintf 'Alfredo Desktop 9.9.9\\n'\n";
    writeFileSync(nativeExecutable, mismatchedNative, "utf8");
    chmodSync(nativeExecutable, 0o755);
    const adapterManifestPath = resolve(installedPlatformRoot, "desktop.json");
    const adapterManifest = JSON.parse(readFileSync(adapterManifestPath, "utf8"));
    expect(adapterManifest.shadow_provider).toBe("bin/alfredo-execution-provider");
    expect(adapterManifest.shadow_provider_sha256).toBe(
      createHash("sha256").update(readFileSync(shadowProvider)).digest("hex"),
    );
    writeFileSync(
      adapterManifestPath,
      `${JSON.stringify(
        {
          ...adapterManifest,
          executable_sha256: createHash("sha256").update(mismatchedNative).digest("hex"),
        },
        null,
        2,
      )}\n`,
      "utf8",
    );
    const nativePreflightFailure = spawnSync("alfredo", [], {
      cwd: workspace,
      encoding: "utf8",
      env: withoutBackendOverrides({
        ALFREDO_RUNTIME_ROOT: resolve(root, "native-mismatch-runtime"),
        PATH: [installedBinRoot, dirname(process.execPath), "/usr/bin", "/bin"].join(delimiter),
      }),
      timeout: 30_000,
    });
    expect(nativePreflightFailure.status).toBe(1);
    expect(nativePreflightFailure.stderr).toContain("desktop_shell");
    expect(nativePreflightFailure.stderr).toContain("Alfredo Desktop 9.9.9");
    expect(nativePreflightFailure.stderr).toContain("APPIMAGE_EXTRACT_AND_RUN=1");

    const overrideBackendRoot = writeBackendFixture(root, "installed-controller");
    const overridden = spawnSync(
      "alfredo",
      ["workstation", "--agent", "installed-controller"],
      {
        cwd: workspace,
        encoding: "utf8",
        env: withBackendOverride(overrideBackendRoot, {
          ALFREDO_DESKTOP_DRY_RUN: "1",
          ALFREDO_RUNTIME_ROOT: resolve(root, "override-runtime"),
          PATH: installedPath,
        }),
      },
    );
    expect(overridden.status, overridden.stderr || overridden.stdout).toBe(0);
    expect(JSON.parse(overridden.stdout)).toMatchObject({
      backend_root: overrideBackendRoot,
      selected_agent: "installed-controller",
    });

    const installedManifest = JSON.parse(
      readFileSync(resolve(installedPackageRoot, "package.json"), "utf8"),
    );
    expect(installedManifest.name).toBe("alfredo-agent");
    expect(installedManifest.optionalDependencies).toEqual({
      "alfredo-agent-linux-x64-gnu": installedManifest.version,
    });
    expect(installedManifest.dependencies ?? {}).not.toHaveProperty("@tauri-apps/cli");
    expect(installedManifest.dependencies ?? {}).not.toHaveProperty("vite");
  } finally {
    spawnSync(process.execPath, [resolve(projectRoot, "scripts", "build-npm-release.js"), "clean"], {
      cwd: projectRoot,
      encoding: "utf8",
    });
    rmSync(root, { recursive: true, force: true });
  }
}, 300_000);

test("workstation startup leaves tracker and Mission discovery unbound before selection", () => {
  const root = mkdtempSync(join(tmpdir(), "alfredo-tracker-types-"));
  const backendRoot = writeBackendFixture(root, "tracker-controller");
  const workspace = resolve(root, "workspace");
  const scratchTracker = resolve(workspace, ".scratch", "actual-work");
  const agentTracker = resolve(workspace, ".agent", "issues");
  mkdirSync(scratchTracker, { recursive: true });
  mkdirSync(agentTracker, { recursive: true });
  writeFileSync(resolve(scratchTracker, "PRD.md"), "# Actual Work\n", "utf8");
  writeFileSync(
    resolve(scratchTracker, "01-agent-work.md"),
    "# Agent Work\n\nStatus: ready-for-agent\nType: AFK\n",
    "utf8",
  );
  writeFileSync(
    resolve(scratchTracker, "02-human-review.md"),
    "# Human Review\n\nStatus: ready-for-human\nType: HITL\n",
    "utf8",
  );
  writeFileSync(resolve(agentTracker, "PRD.md"), "# Parent Tracker\n", "utf8");
  writeFileSync(
    resolve(agentTracker, "18-parent-prd.md"),
    "# Parent PRD\n\nStatus: ready-for-agent\nType: PRD\n",
    "utf8",
  );
  utimesSync(resolve(scratchTracker, "PRD.md"), new Date("2026-01-01"), new Date("2026-01-01"));
  utimesSync(resolve(agentTracker, "PRD.md"), new Date("2026-07-01"), new Date("2026-07-01"));

  try {
    const launched = spawnSync(
      process.execPath,
      [resolve(projectRoot, "bin", "alfredo.js"), "workstation", "--agent", "tracker-controller"],
      {
        cwd: workspace,
        encoding: "utf8",
        env: withBackendOverride(backendRoot, {
          ALFREDO_DESKTOP_DRY_RUN: "1",
        }),
      },
    );
    expect(launched.status, launched.stderr || launched.stdout).toBe(0);
    const plan = JSON.parse(launched.stdout);
    expect(plan.workspace_selection).toEqual({
      schema_version: 1,
      phase: "selection-required",
      starting_location: workspace,
      coding_workspace: null,
      active_mission: null,
    });
    expect(plan).not.toHaveProperty("tracker_dir");
    expect(plan).not.toHaveProperty("issues_dir");
    expect(plan).not.toHaveProperty("mission_id");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("release verification installs only the meta package through an isolated registry", () => {
  const root = mkdtempSync(join(tmpdir(), "alfredo-registry-verifier-"));
  const artifact = resolve(root, "alfredo-desktop.AppImage");
  const provider = resolve(root, "alfredo-execution-provider");
  try {
    writeFileSync(
      artifact,
      "#!/bin/sh\nprintf 'Alfredo Desktop 0.1.0\\n'\n",
      "utf8",
    );
    chmodSync(artifact, 0o755);
    writeFileSync(
      provider,
      "#!/bin/sh\nwhile IFS= read -r _line; do\n  printf '%s\\n' '{\"ok\":false,\"failure\":{\"code\":\"contract-failure\",\"message\":\"fixture rejected invalid request\",\"recoverable\":true}}'\ndone\n",
      "utf8",
    );
    chmodSync(provider, 0o755);
    const result = spawnSync(
      process.execPath,
      [
        resolve(projectRoot, "scripts", "verify-installed-release.js"),
        "--artifact",
        artifact,
        "--provider",
        provider,
      ],
      {
        cwd: projectRoot,
        encoding: "utf8",
        timeout: 300_000,
      },
    );
    expect(result.status, result.stderr || result.stdout).toBe(0);
    const verification = JSON.parse(result.stdout);
    const providerSha256 = createHash("sha256").update(readFileSync(provider)).digest("hex");
    expect(verification).toMatchObject({
      status: "pass",
      install_spec: "alfredo-agent@0.1.0",
      install_source: "isolated local registry with test fixture",
      command: "alfredo",
      invocation: "alfredo",
      package_version: "0.1.0",
      native_package: "alfredo-agent-linux-x64-gnu",
      native_version: "Alfredo Desktop 0.1.0",
      shadow_provider_sha256: providerSha256,
      shadow_provider_contract: "jsonl-structured-failure",
      registry_tarballs_fetched: {
        "alfredo-agent": 1,
        "alfredo-agent-linux-x64-gnu": 1,
      },
    });
    const verifiedRoot = resolve(projectRoot, "release", "out", "verified");
    const manifestPath = resolve(verifiedRoot, "manifest.json");
    expect(verification.verified_artifacts).toEqual({
      directory: verifiedRoot,
      manifest: manifestPath,
      publishable: false,
    });
    const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
    expect(manifest).toMatchObject({
      schema_version: 1,
      status: "verified",
      verification_kind: "test-fixture",
      publishable: false,
      package_version: "0.1.0",
      install_spec: "alfredo-agent@0.1.0",
      publish_order: ["alfredo-agent-linux-x64-gnu", "alfredo-agent"],
      shadow_execution_provider: {
        package: "alfredo-agent-linux-x64-gnu",
        path: "bin/alfredo-execution-provider",
        sha256: providerSha256,
        contract: "jsonl-structured-failure",
        verification: "installed-package",
      },
    });
    expect(manifest.packages.map((entry) => entry.name)).toEqual([
      "alfredo-agent-linux-x64-gnu",
      "alfredo-agent",
    ]);
    for (const entry of manifest.packages) {
      const tarball = resolve(verifiedRoot, entry.filename);
      expect(existsSync(tarball)).toBe(true);
      const bytes = readFileSync(tarball);
      expect(bytes.byteLength).toBe(entry.bytes);
      expect(createHash("sha256").update(bytes).digest("hex")).toBe(entry.sha256);
    }
    const checked = spawnSync(
      process.execPath,
      [resolve(projectRoot, "scripts", "check-verified-release.js"), "--allow-fixture"],
      { cwd: projectRoot, encoding: "utf8" },
    );
    expect(checked.status, checked.stderr || checked.stdout).toBe(0);
    expect(JSON.parse(checked.stdout)).toMatchObject({
      status: "verified",
      publishable: false,
      package_version: "0.1.0",
      packages: [
        { role: "platform", name: "alfredo-agent-linux-x64-gnu" },
        { role: "meta", name: "alfredo-agent" },
      ],
      shadow_execution_provider: {
        path: "bin/alfredo-execution-provider",
        sha256: providerSha256,
      },
    });

    const metaEntry = manifest.packages.find((entry) => entry.role === "meta");
    const metaTarball = resolve(verifiedRoot, metaEntry.filename);
    const originalMetaBytes = readFileSync(metaTarball);
    const tamperedMetaBytes = Buffer.from(originalMetaBytes);
    tamperedMetaBytes[0] ^= 1;
    writeFileSync(metaTarball, tamperedMetaBytes);
    const rejected = spawnSync(
      process.execPath,
      [resolve(projectRoot, "scripts", "check-verified-release.js"), "--allow-fixture"],
      { cwd: projectRoot, encoding: "utf8" },
    );
    expect(rejected.status).toBe(1);
    expect(rejected.stderr).toContain("SHA-256 mismatch");
  } finally {
    spawnSync(
      process.execPath,
      [resolve(projectRoot, "scripts", "build-npm-release.js"), "clean"],
      { cwd: projectRoot, encoding: "utf8" },
    );
    rmSync(root, { recursive: true, force: true });
  }
}, 300_000);
