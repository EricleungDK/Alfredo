import {
  accessSync,
  closeSync,
  constants,
  lstatSync,
  openSync,
  readFileSync,
  readSync,
  realpathSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { dirname, isAbsolute, relative, resolve } from "node:path";

const ADAPTER_SCHEMA_VERSION = 1;
const SUPPORTED_ADAPTERS = new Map([
  [
    "linux:x64",
    {
      packageName: "alfredo-agent-linux-x64-gnu",
      libc: "glibc",
    },
  ],
]);

export class DesktopAdapterError extends Error {
  constructor(message, copyableAction) {
    super(message);
    this.name = "DesktopAdapterError";
    this.copyableAction = copyableAction;
  }
}

function readJson(path, label, copyableAction) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new DesktopAdapterError(
      `${label} is invalid: ${path} (${error.message})`,
      copyableAction,
    );
  }
}

function isInside(root, candidate) {
  const pathFromRoot = relative(root, candidate);
  return pathFromRoot === "" || (!pathFromRoot.startsWith("..") && !isAbsolute(pathFromRoot));
}

function runtimeLibc() {
  if (process.platform !== "linux") return "";
  try {
    return process.report?.getReport?.().header?.glibcVersionRuntime ? "glibc" : "unknown";
  } catch {
    return "unknown";
  }
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

function verifiedExecutable(platformRoot, relativePath, expectedSha256, label, reinstall) {
  const executable = resolve(platformRoot, relativePath);
  if (!isInside(platformRoot, executable)) {
    throw new DesktopAdapterError(
      `Alfredo ${label} escapes its package boundary: ${relativePath}`,
      reinstall,
    );
  }
  let entry;
  try {
    entry = lstatSync(executable);
  } catch (error) {
    throw new DesktopAdapterError(
      `Alfredo ${label} is missing: ${executable} (${error.message})`,
      reinstall,
    );
  }
  if (entry.isSymbolicLink() || !entry.isFile()) {
    throw new DesktopAdapterError(
      `Alfredo ${label} must be a regular non-symlink file: ${executable}`,
      reinstall,
    );
  }
  const realPlatformRoot = realpathSync(platformRoot);
  const realExecutable = realpathSync(executable);
  if (!isInside(realPlatformRoot, realExecutable)) {
    throw new DesktopAdapterError(
      `Alfredo ${label} resolves outside its package boundary: ${executable}`,
      reinstall,
    );
  }
  try {
    accessSync(realExecutable, constants.R_OK | constants.X_OK);
  } catch (error) {
    throw new DesktopAdapterError(
      `Alfredo ${label} is not readable and executable: ${realExecutable} (${error.message})`,
      reinstall,
    );
  }
  if (sha256File(realExecutable) !== expectedSha256) {
    throw new DesktopAdapterError(
      `Alfredo ${label} integrity mismatch: ${realExecutable}`,
      reinstall,
    );
  }
  return realExecutable;
}

export function resolveDesktopAdapter(projectRoot, environment = process.env) {
  const hostKey = `${process.platform}:${process.arch}`;
  const supported = SUPPORTED_ADAPTERS.get(hostKey);
  const metaManifestPath = resolve(projectRoot, "package.json");
  const metaManifest = readJson(
    metaManifestPath,
    "Alfredo package manifest",
    "npm install --global alfredo-agent",
  );
  const releaseVersion = metaManifest.version ?? "latest";
  const reinstall = supported
    ? `npm install --global --force --include=optional ` +
      `alfredo-agent@${releaseVersion} ${supported.packageName}@${releaseVersion}`
    : `npm install --global --force --include=optional alfredo-agent@${releaseVersion}`;
  if (!supported) {
    throw new DesktopAdapterError(
      `Alfredo has no desktop package for ${process.platform}/${process.arch}. ` +
        "The first release supports Ubuntu x64.",
      reinstall,
    );
  }
  const libc = runtimeLibc();
  if (supported.libc && libc !== supported.libc) {
    throw new DesktopAdapterError(
      `Alfredo requires ${supported.libc} on ${process.platform}/${process.arch}; detected ${libc}.`,
      reinstall,
    );
  }

  const requireFromMeta = createRequire(metaManifestPath);
  let platformManifestPath;
  try {
    platformManifestPath = requireFromMeta.resolve(`${supported.packageName}/package.json`);
  } catch {
    throw new DesktopAdapterError(
      `Alfredo desktop package ${supported.packageName}@${metaManifest.version} is not installed.`,
      reinstall,
    );
  }
  const platformRoot = dirname(platformManifestPath);
  const platformPackage = readJson(
    platformManifestPath,
    "Alfredo desktop package manifest",
    reinstall,
  );
  if (
    platformPackage.name !== supported.packageName ||
    platformPackage.version !== metaManifest.version
  ) {
    throw new DesktopAdapterError(
      `Alfredo desktop package version mismatch: expected ${supported.packageName}@${metaManifest.version}, ` +
        `found ${platformPackage.name ?? "unknown"}@${platformPackage.version ?? "unknown"}.`,
      reinstall,
    );
  }

  const adapterManifestPath = resolve(platformRoot, "desktop.json");
  const adapter = readJson(adapterManifestPath, "Alfredo desktop adapter manifest", reinstall);
  if (
    adapter.schema_version !== ADAPTER_SCHEMA_VERSION ||
    adapter.package !== supported.packageName ||
    adapter.version !== metaManifest.version ||
    adapter.platform !== process.platform ||
    adapter.arch !== process.arch ||
    adapter.libc !== supported.libc ||
    adapter.format !== "appimage" ||
    typeof adapter.executable_sha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(adapter.executable_sha256) ||
    typeof adapter.executable !== "string" ||
    !adapter.executable.trim() ||
    typeof adapter.shadow_provider_sha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(adapter.shadow_provider_sha256) ||
    typeof adapter.shadow_provider !== "string" ||
    !adapter.shadow_provider.trim()
  ) {
    throw new DesktopAdapterError(
      `Alfredo desktop adapter metadata does not match ${hostKey}/${supported.libc}.`,
      reinstall,
    );
  }

  const rustCandidateEnabled = environment.ALFREDO_RUST_CANDIDATE_ENABLED ?? "1";
  const rustShellEnabled = environment.ALFREDO_RUST_SHELL_ENABLED ?? "1";
  const rustLocalAgentEnabled =
    environment.ALFREDO_RUST_LOCAL_AGENT_ENABLED ?? "1";
  if (rustCandidateEnabled !== "0" && rustCandidateEnabled !== "1") {
    throw new DesktopAdapterError(
      "ALFREDO_RUST_CANDIDATE_ENABLED must be 0 or 1.",
      reinstall,
    );
  }
  if (rustShellEnabled !== "0" && rustShellEnabled !== "1") {
    throw new DesktopAdapterError(
      "ALFREDO_RUST_SHELL_ENABLED must be 0 or 1.",
      reinstall,
    );
  }
  if (rustLocalAgentEnabled !== "0" && rustLocalAgentEnabled !== "1") {
    throw new DesktopAdapterError(
      "ALFREDO_RUST_LOCAL_AGENT_ENABLED must be 0 or 1.",
      reinstall,
    );
  }
  const realExecutable = verifiedExecutable(
    platformRoot,
    adapter.executable,
    adapter.executable_sha256,
    "desktop executable",
    reinstall,
  );
  const rustProviderRequired =
    rustCandidateEnabled === "1" &&
    (rustShellEnabled === "1" || rustLocalAgentEnabled === "1");
  const realProvider = rustProviderRequired
    ? verifiedExecutable(
        platformRoot,
        adapter.shadow_provider,
        adapter.shadow_provider_sha256,
        "Rust execution provider",
        reinstall,
      )
    : "";
  return {
    kind: "native",
    packageName: supported.packageName,
    version: metaManifest.version,
    executable: realExecutable,
    environment: {
      APPIMAGE_EXTRACT_AND_RUN: "1",
      ALFREDO_RUST_CANDIDATE_ENABLED: rustCandidateEnabled,
      ALFREDO_RUST_SHELL_ENABLED: rustShellEnabled,
      ALFREDO_RUST_LOCAL_AGENT_ENABLED: rustLocalAgentEnabled,
      ALFREDO_RUST_EXECUTION_PROVIDER: realProvider,
      ALFREDO_RUST_EXECUTION_PROVIDER_SHA256: rustProviderRequired
        ? adapter.shadow_provider_sha256
        : "",
      ALFREDO_RUST_EXECUTION_PROVIDER_QUALIFIED_SHA256: rustProviderRequired
        ? adapter.shadow_provider_sha256
        : "",
    },
  };
}
