#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const verifiedRoot = resolve(projectRoot, "release", "out", "verified");
const manifestPath = resolve(verifiedRoot, "manifest.json");
const expectedPackages = [
  { role: "platform", name: "alfredo-agent-linux-x64-gnu" },
  { role: "meta", name: "alfredo-agent" },
];

function fail(message) {
  throw new Error(`Verified Alfredo release is invalid: ${message}`);
}

function isInside(rootPath, candidate) {
  const pathFromRoot = relative(rootPath, candidate);
  return pathFromRoot === "" || (!pathFromRoot.startsWith("..") && !isAbsolute(pathFromRoot));
}

function readJson(path, label) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    fail(`${label} could not be read: ${error.message}`);
  }
}

function tarballManifest(path) {
  const extracted = spawnSync("tar", ["-xOf", path, "package/package.json"], {
    encoding: "utf8",
  });
  // Some WSL-mounted AppImage tarballs make Node report a post-spawn EPERM even
  // though tar exited successfully and returned the requested manifest. Trust
  // the completed exit status and then validate the JSON contents below.
  if (extracted.status !== 0 || !extracted.stdout) {
    fail(
      `package metadata could not be read from ${path}: ${
        extracted.stderr || extracted.error?.message || "tar failed"
      }`,
    );
  }
  try {
    return JSON.parse(extracted.stdout);
  } catch (error) {
    fail(`package metadata in ${path} is invalid JSON: ${error.message}`);
  }
}

function tarballFile(path, member) {
  const extracted = spawnSync("tar", ["-xOf", path, `package/${member}`], {
    encoding: null,
    maxBuffer: 128 * 1024 * 1024,
  });
  if (extracted.status !== 0 || !extracted.stdout?.length) {
    fail(
      `${member} could not be read from ${path}: ${
        extracted.stderr?.toString("utf8") || extracted.error?.message || "tar failed"
      }`,
    );
  }
  return extracted.stdout;
}

function checkedTarball(rootPath, entry, expected, version) {
  if (
    entry?.role !== expected.role ||
    entry?.name !== expected.name ||
    entry?.version !== version ||
    entry?.filename !== `${expected.name}-${version}.tgz` ||
    !Number.isSafeInteger(entry?.bytes) ||
    entry.bytes <= 0 ||
    typeof entry?.sha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(entry.sha256) ||
    typeof entry?.integrity !== "string" ||
    !entry.integrity.startsWith("sha512-")
  ) {
    fail(`${expected.role} package metadata does not match ${expected.name}@${version}`);
  }
  const candidate = resolve(rootPath, entry.filename);
  if (!isInside(rootPath, candidate)) fail(`${expected.role} tarball escapes the verified root`);
  const file = lstatSync(candidate);
  if (file.isSymbolicLink() || !file.isFile()) {
    fail(`${expected.role} tarball must be a regular non-symlink file`);
  }
  const realCandidate = realpathSync(candidate);
  if (!isInside(rootPath, realCandidate)) {
    fail(`${expected.role} tarball resolves outside the verified root`);
  }
  const bytes = readFileSync(realCandidate);
  if (bytes.length !== entry.bytes) {
    fail(`${expected.role} tarball byte count changed`);
  }
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  if (sha256 !== entry.sha256) {
    fail(`${expected.role} tarball SHA-256 mismatch`);
  }
  const integrity = `sha512-${createHash("sha512").update(bytes).digest("base64")}`;
  if (integrity !== entry.integrity) {
    fail(`${expected.role} tarball SHA-512 integrity mismatch`);
  }
  const packageJson = tarballManifest(realCandidate);
  if (packageJson.name !== expected.name || packageJson.version !== version) {
    fail(`${expected.role} tarball contains unexpected package metadata`);
  }
  return {
    role: expected.role,
    name: expected.name,
    version,
    path: realCandidate,
    sha256,
    integrity,
  };
}

function checkVerifiedRelease({ allowFixture = false } = {}) {
  const verifiedEntry = lstatSync(verifiedRoot);
  if (verifiedEntry.isSymbolicLink() || !verifiedEntry.isDirectory()) {
    fail("verified artifact root must be a regular directory");
  }
  const realVerifiedRoot = realpathSync(verifiedRoot);
  const manifest = readJson(manifestPath, "manifest");
  const sourceManifest = readJson(resolve(projectRoot, "package.json"), "source package manifest");
  const version = sourceManifest.version;
  if (
    manifest.schema_version !== 1 ||
    manifest.status !== "verified" ||
    manifest.package_version !== version ||
    manifest.install_spec !== `alfredo-agent@${version}` ||
    !Array.isArray(manifest.publish_order) ||
    JSON.stringify(manifest.publish_order) !==
      JSON.stringify(expectedPackages.map((entry) => entry.name)) ||
    !Array.isArray(manifest.packages) ||
    manifest.packages.length !== expectedPackages.length
  ) {
    fail("manifest identity, version, package set, or publish order changed");
  }
  if (manifest.publishable) {
    if (manifest.verification_kind !== "production-appimage") {
      fail("publishable artifacts were not verified with the production AppImage");
    }
  } else if (!allowFixture || manifest.verification_kind !== "test-fixture") {
    fail("fixture artifacts are not publishable");
  }
  const packages = expectedPackages.map((expected, index) =>
    checkedTarball(realVerifiedRoot, manifest.packages[index], expected, version),
  );
  const platformManifest = tarballManifest(packages[0].path);
  const metaManifest = tarballManifest(packages[1].path);
  if (metaManifest.optionalDependencies?.[platformManifest.name] !== version) {
    fail("meta tarball does not depend on the exact verified platform version");
  }
  if (metaManifest.bin?.alfredo !== "bin/alfredo.js" || metaManifest.bin?.albert !== "bin/alfredo.js") {
    fail("meta tarball does not expose both verified CLI aliases");
  }
  let adapterManifest;
  try {
    adapterManifest = JSON.parse(tarballFile(packages[0].path, "desktop.json").toString("utf8"));
  } catch (error) {
    fail(`platform tarball desktop.json is invalid: ${error.message}`);
  }
  const providerPath = "bin/alfredo-execution-provider";
  const providerBytes = tarballFile(packages[0].path, providerPath);
  const providerSha256 = createHash("sha256").update(providerBytes).digest("hex");
  const expectedProviderContract = manifest.publishable
    ? "python-rust-production-parity"
    : "jsonl-structured-failure";
  if (
    adapterManifest.shadow_provider !== providerPath ||
    adapterManifest.shadow_provider_sha256 !== providerSha256 ||
    manifest.shadow_execution_provider?.package !== expectedPackages[0].name ||
    manifest.shadow_execution_provider?.path !== providerPath ||
    manifest.shadow_execution_provider?.sha256 !== providerSha256 ||
    manifest.shadow_execution_provider?.contract !== expectedProviderContract ||
    manifest.shadow_execution_provider?.verification !== "installed-package"
  ) {
    fail("Rust shadow provider is missing or does not match its verified package evidence");
  }
  if (
    manifest.publishable &&
    (typeof manifest.shadow_execution_provider.request_sha256 !== "string" ||
      !/^[a-f0-9]{64}$/.test(manifest.shadow_execution_provider.request_sha256) ||
      manifest.shadow_execution_provider.store_unchanged !== true)
  ) {
    fail("publishable Rust shadow provider lacks production parity and store-integrity evidence");
  }
  return {
    status: "verified",
    publishable: Boolean(manifest.publishable),
    package_version: version,
    manifest: realpathSync(manifestPath),
    packages,
    shadow_execution_provider: {
      path: providerPath,
      sha256: providerSha256,
    },
  };
}

try {
  const argumentsList = process.argv.slice(2);
  if (argumentsList.some((argument) => argument !== "--allow-fixture")) {
    fail(`unknown argument: ${argumentsList.join(" ")}`);
  }
  const result = checkVerifiedRelease({ allowFixture: argumentsList.includes("--allow-fixture") });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(1);
}

export { checkVerifiedRelease };
