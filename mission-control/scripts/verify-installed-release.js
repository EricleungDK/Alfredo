import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { basename, delimiter, dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const releaseRoot = resolve(projectRoot, "release", "out");
const metaRoot = resolve(releaseRoot, "alfredo-agent");
const platformRoot = resolve(releaseRoot, "alfredo-agent-linux-x64-gnu");
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const root = mkdtempSync(join(tmpdir(), "alfredo-release-verification-"));
const META_PACK_FILES = [
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
  "bundled-backend/albert_mvp/performance.py",
  "bundled-backend/albert_mvp/process_supervisor.py",
  "bundled-backend/albert_mvp/server.py",
  "bundled-backend/albert_mvp/tui.py",
  "bundled-backend/albert_mvp/workspace.py",
  "bundled-backend/albert_mvp/workspace_selection.py",
  "package.json",
  "scripts/performance-recorder.js",
];
const PLATFORM_PACK_FILES = [
  "README.md",
  "bin/alfredo-desktop.AppImage",
  "desktop.json",
  "package.json",
];

function cleanEnvironment(overrides = {}) {
  const environment = { ...process.env };
  for (const key of Object.keys(environment)) {
    if (/^(?:ALBERT|ALFREDO)_/.test(key)) delete environment[key];
  }
  for (const key of [
    "NODE_PATH",
    "PYTHONPATH",
    "npm_config_prefix",
    "NPM_CONFIG_PREFIX",
    "npm_config_registry",
    "NPM_CONFIG_REGISTRY",
  ]) {
    delete environment[key];
  }
  return { ...environment, ...overrides };
}

function signalProcessTree(child, signal) {
  if (!child.pid) return false;
  try {
    if (process.platform === "win32") return child.kill(signal);
    process.kill(-child.pid, signal);
    return true;
  } catch (error) {
    if (error.code === "ESRCH") return false;
    throw error;
  }
}

function processTreeIsLive(child) {
  if (!child.pid) return false;
  if (process.platform === "win32") return child.exitCode === null && child.signalCode === null;
  try {
    process.kill(-child.pid, 0);
    return true;
  } catch (error) {
    if (error.code === "ESRCH") return false;
    if (error.code === "EPERM") return true;
    throw error;
  }
}

async function stopProcessTree(child, closed, graceMilliseconds = 5_000) {
  signalProcessTree(child, "SIGTERM");
  const deadline = Date.now() + graceMilliseconds;
  while (processTreeIsLive(child) && Date.now() < deadline) await delay(100);
  if (processTreeIsLive(child)) {
    signalProcessTree(child, "SIGKILL");
    const killDeadline = Date.now() + 2_000;
    while (processTreeIsLive(child) && Date.now() < killDeadline) await delay(50);
  }
  await Promise.race([closed, delay(2_000)]);
  if (processTreeIsLive(child)) {
    throw new Error(`Unable to stop verification process group ${child.pid}.`);
  }
}

async function run(command, args, options = {}) {
  const child = spawn(command, args, {
    cwd: options.cwd ?? projectRoot,
    env: options.env ?? cleanEnvironment(),
    stdio: ["ignore", "pipe", "pipe"],
    detached: process.platform !== "win32",
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  const closed = new Promise((fulfill) => {
    child.once("error", (error) => fulfill({ error, status: null, signal: null }));
    child.once("close", (status, signal) => fulfill({ error: null, status, signal }));
  });
  const timedOut = Symbol("timed-out");
  let timeout;
  const timeoutReached = new Promise((fulfill) => {
    timeout = setTimeout(() => fulfill(timedOut), options.timeout ?? 300_000);
  });
  const outcome = await Promise.race([
    closed,
    timeoutReached,
  ]);
  clearTimeout(timeout);
  if (outcome === timedOut) {
    await stopProcessTree(child, closed);
    throw new Error(`${options.label ?? command} failed (timeout):\n${stdout}${stderr}`);
  }
  if (outcome.error || outcome.status !== 0) {
    throw new Error(
      `${options.label ?? command} failed (${outcome.status ?? outcome.signal ?? "no status"}):\n` +
        `${stdout}${stderr}${outcome.error?.message ?? ""}`,
    );
  }
  return { status: outcome.status, stdout, stderr };
}

function assertExactPackFiles(actualFiles, expected, label) {
  const actual = [...actualFiles].sort();
  const wanted = [...expected].sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    throw new Error(
      `${label} npm pack manifest changed.\nExpected: ${wanted.join(", ")}\nActual: ${actual.join(", ")}`,
    );
  }
}

async function pack(packageRoot, destination, cache, expectedFiles) {
  const manifest = JSON.parse(readFileSync(resolve(packageRoot, "package.json"), "utf8"));
  const packed = await run(
    npmCommand,
    ["pack", "--json", "--pack-destination", destination],
    {
      cwd: packageRoot,
      label: `npm pack ${packageRoot}`,
      env: cleanEnvironment({ npm_config_cache: cache }),
    },
  );
  const packRecord = packed.stdout.trim() ? JSON.parse(packed.stdout)[0] : null;
  const filename = packRecord?.filename ?? `${manifest.name}-${manifest.version}.tgz`;
  const tarballPath = resolve(destination, filename);
  const listed = await run("tar", ["-tzf", tarballPath], {
    label: `tar manifest ${packageRoot}`,
  });
  const files = listed.stdout
    .split(/\r?\n/)
    .filter((entry) => entry.startsWith("package/") && !entry.endsWith("/"))
    .map((entry) => entry.slice("package/".length));
  assertExactPackFiles(files, expectedFiles, basename(packageRoot));
  return {
    path: tarballPath,
  };
}

function packageDescriptor(packageRoot, tarball) {
  const manifest = JSON.parse(readFileSync(resolve(packageRoot, "package.json"), "utf8"));
  const bytes = readFileSync(tarball);
  return {
    manifest,
    tarball,
    bytes,
    shasum: createHash("sha1").update(bytes).digest("hex"),
    sha256: createHash("sha256").update(bytes).digest("hex"),
    integrity: `sha512-${createHash("sha512").update(bytes).digest("base64")}`,
    metadataRequests: 0,
    tarballRequests: 0,
  };
}

function preserveVerifiedArtifacts(metaDescriptor, platformDescriptor, fixtureArtifact) {
  const verifiedRoot = resolve(releaseRoot, "verified");
  const stagingRoot = resolve(releaseRoot, `.verified-${process.pid}`);
  rmSync(stagingRoot, { recursive: true, force: true });
  mkdirSync(stagingRoot, { recursive: true });
  try {
    const ordered = [
      { role: "platform", descriptor: platformDescriptor },
      { role: "meta", descriptor: metaDescriptor },
    ];
    const packages = ordered.map(({ role, descriptor }) => {
      const filename = basename(descriptor.tarball);
      copyFileSync(descriptor.tarball, resolve(stagingRoot, filename));
      return {
        role,
        name: descriptor.manifest.name,
        version: descriptor.manifest.version,
        filename,
        bytes: descriptor.bytes.length,
        sha256: descriptor.sha256,
        integrity: descriptor.integrity,
      };
    });
    const publishable = !fixtureArtifact;
    const manifest = {
      schema_version: 1,
      status: "verified",
      verification_kind: publishable ? "production-appimage" : "test-fixture",
      publishable,
      package_version: metaDescriptor.manifest.version,
      install_spec: `${metaDescriptor.manifest.name}@${metaDescriptor.manifest.version}`,
      publish_order: packages.map((entry) => entry.name),
      packages,
    };
    writeFileSync(
      resolve(stagingRoot, "manifest.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
      "utf8",
    );
    rmSync(verifiedRoot, { recursive: true, force: true });
    renameSync(stagingRoot, verifiedRoot);
    return {
      directory: verifiedRoot,
      manifest: resolve(verifiedRoot, "manifest.json"),
      publishable,
    };
  } catch (error) {
    rmSync(stagingRoot, { recursive: true, force: true });
    throw error;
  }
}

async function startRegistry(descriptors) {
  const server = createServer((request, response) => {
    let pathname;
    try {
      pathname = decodeURIComponent(new URL(request.url ?? "/", "http://registry.local").pathname);
    } catch {
      response.writeHead(400).end("invalid request");
      return;
    }
    const descriptor = descriptors.find(
      (candidate) => pathname === `/${candidate.manifest.name}`,
    );
    if (descriptor) {
      descriptor.metadataRequests += 1;
      const origin = `http://${request.headers.host}`;
      const metadata = {
        _id: descriptor.manifest.name,
        name: descriptor.manifest.name,
        "dist-tags": { latest: descriptor.manifest.version },
        versions: {
          [descriptor.manifest.version]: {
            ...descriptor.manifest,
            dist: {
              tarball: `${origin}/tarballs/${basename(descriptor.tarball)}`,
              shasum: descriptor.shasum,
              integrity: descriptor.integrity,
            },
          },
        },
      };
      const body = Buffer.from(JSON.stringify(metadata));
      response.writeHead(200, {
        "content-type": "application/json",
        "content-length": body.length,
        "cache-control": "no-store",
      });
      if (request.method !== "HEAD") response.end(body);
      else response.end();
      return;
    }
    const tarball = descriptors.find(
      (candidate) => pathname === `/tarballs/${basename(candidate.tarball)}`,
    );
    if (tarball) {
      response.writeHead(200, {
        "content-type": "application/octet-stream",
        "content-length": tarball.bytes.length,
        "cache-control": "no-store",
      });
      if (request.method !== "HEAD") {
        tarball.tarballRequests += 1;
        response.end(tarball.bytes);
      } else response.end();
      return;
    }
    response.writeHead(404, { "content-type": "application/json" });
    response.end(JSON.stringify({ error: "not_found" }));
  });
  await new Promise((fulfill, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", fulfill);
  });
  const address = server.address();
  if (!address || typeof address === "string") {
    server.close();
    throw new Error("Local npm registry did not expose a TCP address.");
  }
  return {
    url: `http://127.0.0.1:${address.port}`,
    close: () => new Promise((fulfill, reject) => server.close((error) => error ? reject(error) : fulfill())),
  };
}

function isInside(rootPath, candidate) {
  const pathFromRoot = relative(rootPath, candidate);
  return pathFromRoot === "" || (!pathFromRoot.startsWith("..") && !isAbsolute(pathFromRoot));
}

function assertInstalledPath(rootPath, candidate, label) {
  if (!isInside(rootPath, candidate)) {
    throw new Error(`${label} escaped its verified installation root: ${candidate}`);
  }
}

function delay(milliseconds) {
  return new Promise((fulfill) => setTimeout(fulfill, milliseconds));
}

async function runGuiSmoke(command, cwd, environment, markerPath, expectedBackendRoot) {
  rmSync(markerPath, { force: true });
  const child = spawn(command, [], {
    cwd,
    env: { ...environment, ALFREDO_GUI_SMOKE: "1" },
    stdio: ["ignore", "pipe", "pipe"],
    detached: process.platform !== "win32",
  });
  let stdout = "";
  let stderr = "";
  let spawnError = null;
  let closeInfo = null;
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout = `${stdout}${chunk}`.slice(-100_000);
  });
  child.stderr.on("data", (chunk) => {
    stderr = `${stderr}${chunk}`.slice(-100_000);
  });
  child.once("error", (error) => {
    spawnError = error;
  });
  const closed = new Promise((fulfill) => {
    child.once("close", (status, signal) => {
      closeInfo = { status, signal };
      fulfill(closeInfo);
    });
  });

  const deadline = Date.now() + 45_000;
  let marker = null;
  while (Date.now() < deadline) {
    if (spawnError) break;
    if (existsSync(markerPath)) {
      try {
        marker = JSON.parse(readFileSync(markerPath, "utf8"));
        break;
      } catch {
        // The atomic-sized marker may be observed between create and write.
      }
    }
    if (closeInfo) break;
    await delay(200);
  }

  if (!marker) {
    await stopProcessTree(child, closed);
    throw new Error(
      `Installed Alfredo did not reach a GUI-plus-backend ready state.\n` +
        `stdout:\n${stdout}\nstderr:\n${stderr}\n` +
        `process: ${spawnError?.message ?? JSON.stringify(closeInfo) ?? "timed out"}`,
    );
  }
  if (
    marker.schema_version !== 1 ||
    marker.status !== "ready" ||
    marker.phase !== "selection-required" ||
    marker.starting_location !== cwd ||
    marker.coding_workspace !== null ||
    marker.active_mission !== null ||
    marker.backend_root !== expectedBackendRoot ||
    !Number.isInteger(marker.process_id)
  ) {
    await stopProcessTree(child, closed);
    throw new Error(`Installed Alfredo wrote an invalid GUI smoke marker: ${JSON.stringify(marker)}`);
  }

  await delay(750);
  if (closeInfo) {
    await stopProcessTree(child, closed);
    throw new Error(
      `Installed Alfredo exited immediately after GUI readiness (${JSON.stringify(closeInfo)}).\n` +
        `${stdout}${stderr}`,
    );
  }
  await stopProcessTree(child, closed, 10_000);
  return {
    status: "pass",
    window: "frontend loaded",
    backend: "selection-required launch context ready",
    process_id: marker.process_id,
  };
}

let registry;
try {
  const artifactIndex = process.argv.indexOf("--artifact");
  const fixtureArtifact = artifactIndex >= 0 ? process.argv[artifactIndex + 1] : "";
  if (artifactIndex >= 0 && !fixtureArtifact) {
    throw new Error("--artifact requires an executable fixture path.");
  }
  if (fixtureArtifact) {
    await run(
      process.execPath,
      [resolve(projectRoot, "scripts", "build-npm-release.js"), "build", "--artifact", fixtureArtifact],
      { label: "fixture npm release stage" },
    );
  } else {
    await run(npmCommand, ["run", "release:build"], {
      label: "production AppImage build",
      timeout: 600_000,
    });
  }

  const packedRoot = resolve(root, "packed");
  const cache = resolve(root, "npm-cache");
  const prefix = resolve(root, "prefix");
  const workspace = resolve(root, "workspace");
  const runtime = resolve(root, "runtime");
  mkdirSync(packedRoot, { recursive: true });
  mkdirSync(workspace, { recursive: true });
  const packedMeta = await pack(metaRoot, packedRoot, cache, META_PACK_FILES);
  const packedPlatform = await pack(platformRoot, packedRoot, cache, PLATFORM_PACK_FILES);
  const descriptors = [
    packageDescriptor(metaRoot, packedMeta.path),
    packageDescriptor(platformRoot, packedPlatform.path),
  ];
  const metaDescriptor = descriptors[0];
  const platformDescriptor = descriptors[1];
  if (
    metaDescriptor.manifest.optionalDependencies?.[platformDescriptor.manifest.name] !==
    metaDescriptor.manifest.version
  ) {
    throw new Error("Staged Alfredo packages do not share one exact optional-dependency version.");
  }

  registry = await startRegistry(descriptors);
  await run(
    npmCommand,
    [
      "install",
      "--global",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      "--package-lock=false",
      "--prefer-online",
      "--prefix",
      prefix,
      "--registry",
      registry.url,
      `${metaDescriptor.manifest.name}@${metaDescriptor.manifest.version}`,
    ],
    {
      label: "meta-only clean-registry npm install",
      env: cleanEnvironment({ npm_config_cache: cache }),
    },
  );
  if (descriptors.some((descriptor) => descriptor.tarballRequests < 1)) {
    throw new Error(
      `Meta-only npm install did not fetch every release tarball: ${descriptors
        .map((descriptor) => `${descriptor.manifest.name}=${descriptor.tarballRequests}`)
        .join(", ")}`,
    );
  }
  await registry.close();
  registry = null;

  const installedModulesRoot = realpathSync(resolve(prefix, "lib", "node_modules"));
  const installedMeta = realpathSync(resolve(installedModulesRoot, "alfredo-agent"));
  assertInstalledPath(installedModulesRoot, installedMeta, "Installed Alfredo meta package");
  const installedMetaManifestPath = resolve(installedMeta, "package.json");
  const installedPlatformManifestPath = realpathSync(
    createRequire(installedMetaManifestPath).resolve(
      "alfredo-agent-linux-x64-gnu/package.json",
    ),
  );
  const installedPlatform = realpathSync(dirname(installedPlatformManifestPath));
  assertInstalledPath(installedModulesRoot, installedPlatform, "Installed Alfredo platform package");
  const bundledBackend = resolve(installedMeta, "bundled-backend");
  const installedAgentConfig = resolve(bundledBackend, ".albert", "agents.json");
  const nativeExecutable = resolve(
    installedPlatform,
    "bin",
    "alfredo-desktop.AppImage",
  );
  const metaManifest = JSON.parse(readFileSync(installedMetaManifestPath, "utf8"));
  const platformManifest = JSON.parse(
    readFileSync(installedPlatformManifestPath, "utf8"),
  );
  const adapterManifest = JSON.parse(
    readFileSync(resolve(installedPlatform, "desktop.json"), "utf8"),
  );
  if (
    metaManifest.version !== platformManifest.version ||
    metaManifest.version !== adapterManifest.version ||
    metaManifest.optionalDependencies?.[platformManifest.name] !== metaManifest.version
  ) {
    throw new Error("Installed Alfredo package and adapter versions do not match.");
  }

  const installedPath = [
    resolve(prefix, "bin"),
    dirname(process.execPath),
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
  ].join(delimiter);
  const baseLaunchEnvironment = cleanEnvironment({
    PATH: installedPath,
    ALFREDO_RUNTIME_ROOT: runtime,
  });
  const productVersion = await run("alfredo", ["--version"], {
    cwd: workspace,
    label: "installed Alfredo product version",
    env: baseLaunchEnvironment,
  });
  if (productVersion.stdout.trim() !== `Alfredo ${metaManifest.version}`) {
    throw new Error(`Installed Alfredo returned an unexpected version: ${productVersion.stdout}`);
  }

  const planned = await run("alfredo", [], {
    cwd: workspace,
    label: "installed PATH no-argument product plan",
    env: { ...baseLaunchEnvironment, ALFREDO_DESKTOP_DRY_RUN: "1" },
  });
  const productPlan = JSON.parse(planned.stdout);
  if (
    productPlan.project_root !== installedMeta ||
    productPlan.backend_root !== bundledBackend ||
    productPlan.starting_location !== workspace ||
    productPlan.workspace_selection?.phase !== "selection-required" ||
    productPlan.workspace_selection?.starting_location !== workspace ||
    productPlan.workspace_selection?.coding_workspace !== null ||
    productPlan.workspace_selection?.active_mission !== null ||
    "selected_workspace" in productPlan
  ) {
    throw new Error(`Installed Alfredo resolved a non-installed product plan: ${planned.stdout}`);
  }
  assertInstalledPath(installedMeta, productPlan.project_root, "Product root");
  assertInstalledPath(installedMeta, productPlan.backend_root, "Backend root");

  const launch = await run("alfredo", [], {
    cwd: workspace,
    label: "installed PATH no-argument native launch plan",
    env: { ...baseLaunchEnvironment, ALFREDO_DESKTOP_DRY_RUN: "launch" },
  });
  const launchPlan = JSON.parse(launch.stdout);
  if (
    launchPlan.command?.length !== 1 ||
    launchPlan.command[0] !== nativeExecutable ||
    launchPlan.cwd !== workspace ||
    launchPlan.env?.APPIMAGE_EXTRACT_AND_RUN !== "1" ||
    launchPlan.env?.ALBERT_BACKEND_ROOT !== bundledBackend ||
    launchPlan.env?.ALFREDO_AGENT_CONFIG !== installedAgentConfig
  ) {
    throw new Error(`Installed Alfredo resolved an invalid native launch plan: ${launch.stdout}`);
  }
  if (/\b(?:npm|cargo|tauri)\b/i.test(launchPlan.command.join(" "))) {
    throw new Error(`Installed Alfredo launch still depends on a development command: ${launch.stdout}`);
  }

  const nativeVersion = await run(nativeExecutable, ["--version"], {
    cwd: workspace,
    label: "installed AppImage native version probe",
    env: cleanEnvironment({
      PATH: "/usr/bin:/bin",
      APPIMAGE_EXTRACT_AND_RUN: "1",
    }),
  });
  const expectedNativeVersion = `Alfredo Desktop ${metaManifest.version}`;
  if (nativeVersion.stdout.trim() !== expectedNativeVersion) {
    throw new Error(`Installed AppImage returned an unexpected version: ${nativeVersion.stdout}`);
  }

  const digest = createHash("sha256")
    .update(readFileSync(nativeExecutable))
    .digest("hex");
  if (adapterManifest.executable_sha256 !== digest) {
    throw new Error("Installed AppImage does not match the adapter manifest digest.");
  }
  const guiSmoke = fixtureArtifact
    ? { status: "not_run_fixture" }
    : await runGuiSmoke(
        "alfredo",
        workspace,
        baseLaunchEnvironment,
        resolve(runtime, "gui-smoke-ready.json"),
        bundledBackend,
      );
  const verifiedArtifacts = preserveVerifiedArtifacts(
    metaDescriptor,
    platformDescriptor,
    fixtureArtifact,
  );
  process.stdout.write(
    `${JSON.stringify(
      {
        status: "pass",
        install_spec: `${metaManifest.name}@${metaManifest.version}`,
        install_source: fixtureArtifact
          ? "isolated local registry with test fixture"
          : "isolated local registry with production AppImage",
        command: "alfredo",
        invocation: "alfredo",
        package_version: metaManifest.version,
        native_package: platformManifest.name,
        native_bytes: statSync(nativeExecutable).size,
        native_sha256: digest,
        native_version: nativeVersion.stdout.trim(),
        gui_smoke: guiSmoke,
        registry_tarballs_fetched: Object.fromEntries(
          descriptors.map((descriptor) => [descriptor.manifest.name, descriptor.tarballRequests]),
        ),
        verified_artifacts: verifiedArtifacts,
      },
      null,
      2,
    )}\n`,
  );
} finally {
  if (registry) await registry.close();
  rmSync(root, { recursive: true, force: true });
}
