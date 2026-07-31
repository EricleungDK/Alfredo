import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createHash } from "node:crypto";

import { expect, test } from "vitest";

import { DesktopAdapterError, resolveDesktopAdapter } from "../bin/desktop-adapter.js";

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function adapterFixture() {
  const root = mkdtempSync(join(tmpdir(), "alfredo-desktop-adapter-"));
  const nodeModules = resolve(root, "node_modules");
  const metaRoot = resolve(nodeModules, "alfredo-agent");
  const platformRoot = resolve(nodeModules, "alfredo-agent-linux-x64-gnu");
  const executable = resolve(platformRoot, "bin", "alfredo-desktop.AppImage");
  mkdirSync(resolve(metaRoot, "bin"), { recursive: true });
  mkdirSync(resolve(platformRoot, "bin"), { recursive: true });
  writeJson(resolve(metaRoot, "package.json"), {
    name: "alfredo-agent",
    version: "0.1.0",
    type: "module",
  });
  writeJson(resolve(platformRoot, "package.json"), {
    name: "alfredo-agent-linux-x64-gnu",
    version: "0.1.0",
  });
  writeFileSync(executable, "#!/bin/sh\nexit 0\n", "utf8");
  chmodSync(executable, 0o755);
  const executableSha256 = createHash("sha256")
    .update("#!/bin/sh\nexit 0\n")
    .digest("hex");
  writeJson(resolve(platformRoot, "desktop.json"), {
    schema_version: 1,
    package: "alfredo-agent-linux-x64-gnu",
    version: "0.1.0",
    platform: "linux",
    arch: "x64",
    libc: "glibc",
    format: "appimage",
    executable: "bin/alfredo-desktop.AppImage",
    executable_sha256: executableSha256,
  });
  return { root, metaRoot, platformRoot, executable };
}

test("resolves an exact-version executable from the supported platform package", () => {
  const fixture = adapterFixture();
  try {
    expect(resolveDesktopAdapter(fixture.metaRoot)).toEqual({
      kind: "native",
      packageName: "alfredo-agent-linux-x64-gnu",
      version: "0.1.0",
      executable: fixture.executable,
      environment: { APPIMAGE_EXTRACT_AND_RUN: "1" },
    });
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("fails with a reinstall action when the optional platform package is absent", () => {
  const fixture = adapterFixture();
  try {
    rmSync(fixture.platformRoot, { recursive: true, force: true });
    expect(() => resolveDesktopAdapter(fixture.metaRoot)).toThrowError(DesktopAdapterError);
    try {
      resolveDesktopAdapter(fixture.metaRoot);
    } catch (error) {
      expect(error.message).toContain("is not installed");
      expect(error.copyableAction).toBe(
        "npm install --global --force --include=optional alfredo-agent@0.1.0 alfredo-agent-linux-x64-gnu@0.1.0",
      );
    }
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("rejects a platform package whose version differs from the public CLI package", () => {
  const fixture = adapterFixture();
  try {
    writeJson(resolve(fixture.platformRoot, "package.json"), {
      name: "alfredo-agent-linux-x64-gnu",
      version: "0.2.0",
    });
    expect(() => resolveDesktopAdapter(fixture.metaRoot)).toThrow(/version mismatch/);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("rejects symlinked and non-executable desktop artifacts", () => {
  const fixture = adapterFixture();
  try {
    const external = resolve(fixture.root, "external-desktop");
    writeFileSync(external, "#!/bin/sh\nexit 0\n", "utf8");
    chmodSync(external, 0o755);
    rmSync(fixture.executable);
    symlinkSync(external, fixture.executable);
    expect(() => resolveDesktopAdapter(fixture.metaRoot)).toThrow(/non-symlink/);

    rmSync(fixture.executable);
    writeFileSync(fixture.executable, "not executable\n", "utf8");
    chmodSync(fixture.executable, 0o644);
    expect(() => resolveDesktopAdapter(fixture.metaRoot)).toThrow(/not readable and executable/);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("rejects a desktop artifact path outside the platform package", () => {
  const fixture = adapterFixture();
  try {
    writeJson(resolve(fixture.platformRoot, "desktop.json"), {
      schema_version: 1,
      package: "alfredo-agent-linux-x64-gnu",
      version: "0.1.0",
      platform: "linux",
      arch: "x64",
      libc: "glibc",
      format: "appimage",
      executable: "../outside.AppImage",
      executable_sha256: "0".repeat(64),
    });
    expect(() => resolveDesktopAdapter(fixture.metaRoot)).toThrow(/escapes its package boundary/);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("rejects missing, malformed, and mismatched desktop artifact digests", () => {
  const fixture = adapterFixture();
  try {
    const manifestPath = resolve(fixture.platformRoot, "desktop.json");
    const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
    writeJson(manifestPath, { ...manifest, executable_sha256: undefined });
    expect(() => resolveDesktopAdapter(fixture.metaRoot)).toThrow(/metadata does not match/);

    writeJson(manifestPath, { ...manifest, executable_sha256: "not-a-digest" });
    expect(() => resolveDesktopAdapter(fixture.metaRoot)).toThrow(/metadata does not match/);

    writeJson(manifestPath, manifest);
    writeFileSync(fixture.executable, "#!/bin/sh\nexit 1\n", "utf8");
    chmodSync(fixture.executable, 0o755);
    expect(() => resolveDesktopAdapter(fixture.metaRoot)).toThrow(/integrity mismatch/);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});
