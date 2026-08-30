import {
  chmodSync,
  closeSync,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { basename, dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");
const releaseRoot = resolve(projectRoot, "release", "out");
const sourceBackend = resolve(repositoryRoot, "albert_mvp");
const sourceRegistry = resolve(repositoryRoot, ".albert", "agents.json");
const sourceManifest = JSON.parse(
  readFileSync(resolve(projectRoot, "package.json"), "utf8"),
);
const targetCatalog = JSON.parse(
  readFileSync(resolve(projectRoot, "release", "targets.json"), "utf8"),
);
const BACKEND_SOURCE_FILES = [
  "__init__.py",
  "__main__.py",
  "agents.py",
  "capabilities.py",
  "cli.py",
  "core.py",
  "execution.py",
  "execution_cutover.py",
  "local_agent_execution_cutover.py",
  "execution_shadow.py",
  "inference.py",
  "inference_qualification.py",
  "performance.py",
  "process_supervisor.py",
  "retirement.py",
  "server.py",
  "tui.py",
  "workspace.py",
  "workspace_selection.py",
];

function assertReleaseRoot() {
  if (dirname(releaseRoot) !== resolve(projectRoot, "release") || basename(releaseRoot) !== "out") {
    throw new Error(`Refusing unsafe Alfredo release output path: ${releaseRoot}`);
  }
}

function clean() {
  assertReleaseRoot();
  rmSync(releaseRoot, { recursive: true, force: true });
}

function regularSource(path, label, directory = false) {
  if (!existsSync(path)) throw new Error(`Missing Alfredo ${label}: ${path}`);
  const entry = lstatSync(path);
  if (entry.isSymbolicLink() || (directory ? !entry.isDirectory() : !entry.isFile())) {
    throw new Error(`Alfredo ${label} must be a regular ${directory ? "directory" : "file"}: ${path}`);
  }
}

function isInside(root, candidate) {
  const pathFromRoot = relative(root, candidate);
  return pathFromRoot === "" || (!pathFromRoot.startsWith("..") && !isAbsolute(pathFromRoot));
}

function resolveInside(root, path, label) {
  if (typeof path !== "string" || !path.trim() || isAbsolute(path)) {
    throw new Error(`Alfredo ${label} must be a non-empty relative path.`);
  }
  const resolved = resolve(root, path);
  if (!isInside(root, resolved)) {
    throw new Error(`Alfredo ${label} escapes its release boundary: ${path}`);
  }
  return resolved;
}

function validatePackageName(name) {
  if (typeof name !== "string" || !/^[a-z0-9][a-z0-9._-]*$/.test(name)) {
    throw new Error(`Invalid Alfredo release package name: ${String(name)}`);
  }
}

export function validateTarget(target) {
  if (!target || typeof target !== "object") throw new Error("Invalid Alfredo release target.");
  for (const field of ["platform", "arch", "libc"]) {
    if (typeof target[field] !== "string" || !/^[a-z0-9_-]+$/.test(target[field])) {
      throw new Error(`Invalid Alfredo release target ${field}: ${String(target[field])}`);
    }
  }
  validatePackageName(target.package);
  resolveInside(resolve("/tmp", "alfredo-catalog-check"), target.artifact_directory, "artifact directory");
  const executable = resolveInside(
    resolve("/tmp", "alfredo-catalog-check"),
    target.executable,
    "platform executable",
  );
  if (dirname(relative(resolve("/tmp", "alfredo-catalog-check"), executable)) === ".") {
    throw new Error("Alfredo platform executable must live below a package subdirectory.");
  }
  const shadowProvider = resolveInside(
    resolve("/tmp", "alfredo-catalog-check"),
    target.shadow_provider,
    "shadow provider",
  );
  if (dirname(relative(resolve("/tmp", "alfredo-catalog-check"), shadowProvider)) === ".") {
    throw new Error("Alfredo shadow provider must live below a package subdirectory.");
  }
  if (typeof target.artifact_suffix !== "string" || !/^\.[A-Za-z0-9]+$/.test(target.artifact_suffix)) {
    throw new Error(`Invalid Alfredo artifact suffix: ${String(target.artifact_suffix)}`);
  }
  return target;
}

function validatedTargets() {
  if (targetCatalog.schema_version !== 1 || !Array.isArray(targetCatalog.targets)) {
    throw new Error("Unsupported Alfredo release target catalog.");
  }
  const targets = targetCatalog.targets.map(validateTarget);
  const packageNames = targets.map((target) => target.package);
  if (new Set(packageNames).size !== packageNames.length) {
    throw new Error("Alfredo release target package names must be unique.");
  }
  return targets;
}

export function cargoPackageVersionFromText(cargoManifest) {
  let inPackageSection = false;
  for (const line of cargoManifest.split(/\r?\n/)) {
    const section = line.trim().match(/^\[([^\]]+)\]$/)?.[1] ?? "";
    if (section) {
      if (inPackageSection) break;
      inPackageSection = section === "package";
      continue;
    }
    if (!inPackageSection) continue;
    const version = line.match(/^\s*version\s*=\s*"([^"]+)"\s*$/)?.[1];
    if (version) return version;
  }
  return "";
}

function cargoPackageVersion() {
  const cargoManifest = readFileSync(resolve(projectRoot, "src-tauri", "Cargo.toml"), "utf8");
  return cargoPackageVersionFromText(cargoManifest);
}

export function assertMatchingReleaseVersions(versions) {
  if (
    versions.some((version) => typeof version !== "string" || !version) ||
    new Set(versions).size !== 1
  ) {
    throw new Error(`Alfredo release versions must match across npm/Cargo/Tauri: ${versions.join(", ")}`);
  }
}

function assertVersionCoherence() {
  const tauriVersion = JSON.parse(
    readFileSync(resolve(projectRoot, "src-tauri", "tauri.conf.json"), "utf8"),
  ).version;
  const versions = [sourceManifest.version, cargoPackageVersion(), tauriVersion];
  assertMatchingReleaseVersions(versions);
}

function sha256File(path) {
  const hash = createHash("sha256");
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  const file = openSync(path, "r");
  try {
    let bytesRead;
    do {
      bytesRead = readSync(file, buffer, 0, buffer.length, null);
      if (bytesRead > 0) hash.update(buffer.subarray(0, bytesRead));
    } while (bytesRead > 0);
  } finally {
    closeSync(file);
  }
  return hash.digest("hex");
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function packageMetadata(name, description) {
  return {
    name,
    version: sourceManifest.version,
    description,
    repository: {
      type: "git",
      url: "git+https://github.com/EricleungDK/Alfredo.git",
    },
    homepage: "https://github.com/EricleungDK/Alfredo#readme",
    bugs: "https://github.com/EricleungDK/Alfredo/issues",
    keywords: ["coding-agent", "desktop", "local-first", "tauri"],
    publishConfig: { access: "public" },
  };
}

export function assertBackendSourceAllowlist(discoveredPythonFiles) {
  const discovered = [...discoveredPythonFiles].sort();
  const allowed = [...BACKEND_SOURCE_FILES].sort();
  if (JSON.stringify(discovered) !== JSON.stringify(allowed)) {
    throw new Error(`Alfredo backend release allowlist is stale: ${discovered.join(", ")}`);
  }
}

function copyBackend(destination) {
  regularSource(sourceBackend, "Python backend", true);
  regularSource(sourceRegistry, "agent registry");
  const backendRoot = resolve(destination, "bundled-backend");
  const discoveredPythonFiles = readdirSync(sourceBackend, { withFileTypes: true })
    .filter((entry) => entry.isFile() && !entry.isSymbolicLink() && entry.name.endsWith(".py"))
    .map((entry) => entry.name)
    .sort();
  assertBackendSourceAllowlist(discoveredPythonFiles);
  const packagedBackendRoot = resolve(backendRoot, "albert_mvp");
  mkdirSync(packagedBackendRoot, { recursive: true });
  for (const filename of BACKEND_SOURCE_FILES) {
    const source = resolve(sourceBackend, filename);
    regularSource(source, `backend source ${filename}`);
    copyFileSync(source, resolve(packagedBackendRoot, filename));
  }
  mkdirSync(resolve(backendRoot, ".albert"), { recursive: true });
  copyFileSync(sourceRegistry, resolve(backendRoot, ".albert", "agents.json"));
}

function selectTarget() {
  const target = validatedTargets().find(
    (candidate) => candidate.platform === process.platform && candidate.arch === process.arch,
  );
  if (!target) {
    throw new Error(`No Alfredo release target for ${process.platform}/${process.arch}.`);
  }
  return target;
}

function findArtifact(target) {
  const artifactIndex = process.argv.indexOf("--artifact");
  if (artifactIndex >= 0 && process.argv[artifactIndex + 1]) {
    const artifact = resolve(process.argv[artifactIndex + 1]);
    regularSource(artifact, "Tauri desktop artifact");
    return artifact;
  }
  const targetDirectoryIndex = process.argv.indexOf("--target-dir");
  const cargoTargetDirectory =
    targetDirectoryIndex >= 0 && process.argv[targetDirectoryIndex + 1]
      ? resolve(process.argv[targetDirectoryIndex + 1])
      : resolve(projectRoot, "src-tauri", "target");
  const artifactDirectory = resolveInside(
    cargoTargetDirectory,
    target.artifact_directory,
    "artifact directory",
  );
  regularSource(artifactDirectory, "Tauri artifact directory", true);
  const candidates = readdirSync(artifactDirectory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && !entry.isSymbolicLink() && entry.name.endsWith(target.artifact_suffix))
    .map((entry) => resolve(artifactDirectory, entry.name));
  if (candidates.length !== 1) {
    throw new Error(
      `Expected exactly one Alfredo ${target.artifact_suffix} in ${artifactDirectory}; found ${candidates.length}.`,
    );
  }
  regularSource(candidates[0], "Tauri desktop artifact");
  return candidates[0];
}

function findShadowProvider() {
  const providerIndex = process.argv.indexOf("--provider");
  if (providerIndex >= 0 && process.argv[providerIndex + 1]) {
    const provider = resolve(process.argv[providerIndex + 1]);
    regularSource(provider, "Rust shadow execution provider");
    return provider;
  }
  const targetDirectoryIndex = process.argv.indexOf("--target-dir");
  const cargoTargetDirectory =
    targetDirectoryIndex >= 0 && process.argv[targetDirectoryIndex + 1]
      ? resolve(process.argv[targetDirectoryIndex + 1])
      : resolve(projectRoot, "src-tauri", "target");
  const provider = resolve(cargoTargetDirectory, "release", "alfredo-execution-provider");
  regularSource(provider, "Rust shadow execution provider");
  return provider;
}

function build() {
  clean();
  try {
    assertVersionCoherence();
    const target = selectTarget();
    const artifact = findArtifact(target);
    const shadowProvider = findShadowProvider();
    const metaRoot = resolve(releaseRoot, "alfredo-agent");
    const platformRoot = resolveInside(releaseRoot, target.package, "platform package");
    mkdirSync(resolve(metaRoot, "bin"), { recursive: true });
    mkdirSync(resolve(metaRoot, "scripts"), { recursive: true });
    const platformExecutable = resolveInside(
      platformRoot,
      target.executable,
      "platform executable",
    );
    const packagedShadowProvider = resolveInside(
      platformRoot,
      target.shadow_provider,
      "packaged shadow provider",
    );
    mkdirSync(dirname(platformExecutable), { recursive: true });
    mkdirSync(dirname(packagedShadowProvider), { recursive: true });

    copyFileSync(resolve(projectRoot, "bin", "alfredo.js"), resolve(metaRoot, "bin", "alfredo.js"));
    copyFileSync(
      resolve(projectRoot, "bin", "desktop-adapter.js"),
      resolve(metaRoot, "bin", "desktop-adapter.js"),
    );
    copyFileSync(
      resolve(projectRoot, "scripts", "performance-recorder.js"),
      resolve(metaRoot, "scripts", "performance-recorder.js"),
    );
    chmodSync(resolve(metaRoot, "bin", "alfredo.js"), 0o755);
    copyBackend(metaRoot);

    const optionalDependencies = Object.fromEntries(
      validatedTargets().map((candidate) => [candidate.package, sourceManifest.version]),
    );
    writeJson(resolve(metaRoot, "package.json"), {
      ...packageMetadata(
        "alfredo-agent",
        "Local-first coding-agent workstation with an installed native desktop",
      ),
      type: "module",
      bin: {
        alfredo: "bin/alfredo.js",
        albert: "bin/alfredo.js",
      },
      files: ["bin", "bundled-backend", "scripts/performance-recorder.js"],
      optionalDependencies,
      engines: { node: ">=20" },
    });
    writeFileSync(
      resolve(metaRoot, "README.md"),
      [
        "# Alfredo",
        "",
        "Install with `npm install --global alfredo-agent`, then run `alfredo` from a coding workspace.",
        "",
      ].join("\n"),
      "utf8",
    );

    copyFileSync(artifact, platformExecutable);
    chmodSync(platformExecutable, 0o755);
    copyFileSync(shadowProvider, packagedShadowProvider);
    chmodSync(packagedShadowProvider, 0o755);
    const executableSha256 = sha256File(platformExecutable);
    const shadowProviderSha256 = sha256File(packagedShadowProvider);
    writeJson(resolve(platformRoot, "desktop.json"), {
      schema_version: 1,
      package: target.package,
      version: sourceManifest.version,
      platform: target.platform,
      arch: target.arch,
      libc: target.libc,
      format: "appimage",
      executable: target.executable,
      executable_sha256: executableSha256,
      shadow_provider: target.shadow_provider,
      shadow_provider_sha256: shadowProviderSha256,
    });
    writeJson(resolve(platformRoot, "package.json"), {
      ...packageMetadata(
        target.package,
        "Ubuntu x64 native desktop adapter for the Alfredo coding-agent workstation",
      ),
      os: [target.platform],
      cpu: [target.arch],
      libc: [target.libc],
      files: ["bin", "desktop.json"],
    });
    writeFileSync(
      resolve(platformRoot, "README.md"),
      "# Alfredo Ubuntu x64 desktop adapter\n\nInstalled automatically by `alfredo-agent`.\n",
      "utf8",
    );

    process.stdout.write(
      `${JSON.stringify({
        meta: metaRoot,
        platform: platformRoot,
        artifact: platformExecutable,
        shadow_provider: packagedShadowProvider,
      })}\n`,
    );
  } catch (error) {
    clean();
    throw error;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const action = process.argv[2];
  if (action === "build") {
    build();
  } else if (action === "clean") {
    clean();
  } else {
    throw new Error('Expected Alfredo npm release action "build" or "clean".');
  }
}
