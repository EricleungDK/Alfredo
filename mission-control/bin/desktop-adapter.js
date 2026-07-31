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

export function resolveDesktopAdapter(projectRoot) {
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
    !adapter.executable.trim()
  ) {
    throw new DesktopAdapterError(
      `Alfredo desktop adapter metadata does not match ${hostKey}/${supported.libc}.`,
      reinstall,
    );
  }

  const executable = resolve(platformRoot, adapter.executable);
  if (!isInside(platformRoot, executable)) {
    throw new DesktopAdapterError(
      `Alfredo desktop executable escapes its package boundary: ${adapter.executable}`,
      reinstall,
    );
  }
  let executableEntry;
  try {
    executableEntry = lstatSync(executable);
  } catch (error) {
    throw new DesktopAdapterError(
      `Alfredo desktop executable is missing: ${executable} (${error.message})`,
      reinstall,
    );
  }
  if (executableEntry.isSymbolicLink() || !executableEntry.isFile()) {
    throw new DesktopAdapterError(
      `Alfredo desktop executable must be a regular non-symlink file: ${executable}`,
      reinstall,
    );
  }
  const realPlatformRoot = realpathSync(platformRoot);
  const realExecutable = realpathSync(executable);
  if (!isInside(realPlatformRoot, realExecutable)) {
    throw new DesktopAdapterError(
      `Alfredo desktop executable resolves outside its package boundary: ${executable}`,
      reinstall,
    );
  }
  try {
    accessSync(realExecutable, constants.R_OK | constants.X_OK);
  } catch (error) {
    throw new DesktopAdapterError(
      `Alfredo desktop executable is not readable and executable: ${realExecutable} (${error.message})`,
      reinstall,
    );
  }
  const executableSha256 = sha256File(realExecutable);
  if (executableSha256 !== adapter.executable_sha256) {
    throw new DesktopAdapterError(
      `Alfredo desktop executable integrity mismatch: ${realExecutable}`,
      reinstall,
    );
  }
  return {
    kind: "native",
    packageName: supported.packageName,
    version: metaManifest.version,
    executable: realExecutable,
    environment: { APPIMAGE_EXTRACT_AND_RUN: "1" },
  };
}
