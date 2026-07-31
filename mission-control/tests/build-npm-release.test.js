import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { expect, test } from "vitest";

import {
  assertBackendSourceAllowlist,
  assertMatchingReleaseVersions,
  cargoPackageVersionFromText,
  validateTarget,
} from "../scripts/build-npm-release.js";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

const validTarget = {
  platform: "linux",
  arch: "x64",
  libc: "glibc",
  package: "alfredo-agent-linux-x64-gnu",
  artifact_directory: "release/bundle/appimage",
  artifact_suffix: ".AppImage",
  executable: "bin/alfredo-desktop.AppImage",
};

test("release target validation keeps catalog paths inside their boundaries", () => {
  expect(validateTarget({ ...validTarget })).toEqual(validTarget);
  expect(() =>
    validateTarget({ ...validTarget, artifact_directory: "../outside" }),
  ).toThrow(/escapes its release boundary/);
  expect(() =>
    validateTarget({ ...validTarget, executable: "../../outside.AppImage" }),
  ).toThrow(/escapes its release boundary/);
  expect(() =>
    validateTarget({ ...validTarget, executable: "alfredo-desktop.AppImage" }),
  ).toThrow(/below a package subdirectory/);
  expect(() => validateTarget({ ...validTarget, package: "../outside" })).toThrow(
    /Invalid Alfredo release package name/,
  );
});

test("Cargo package version parsing handles a terminal package section", () => {
  expect(
    cargoPackageVersionFromText(
      '[workspace]\nmembers = ["zebra"]\n\n[package]\nname = "zebra"\nversion = "9.8.7"\n',
    ),
  ).toBe("9.8.7");
  expect(
    cargoPackageVersionFromText(
      '[package]\nname = "alfredo-desktop"\nversion = "0.1.0"\n\n[lib]\nname = "zebra"\n',
    ),
  ).toBe("0.1.0");
});

test("release version and backend allowlist checks fail closed", () => {
  expect(() => assertMatchingReleaseVersions(["0.1.0", "0.2.0", "0.1.0"])).toThrow(
    /must match/,
  );
  expect(() => assertMatchingReleaseVersions(["0.1.0", "", "0.1.0"])).toThrow(
    /must match/,
  );
  expect(() => assertBackendSourceAllowlist(["__init__.py", "secret.py"])).toThrow(
    /allowlist is stale/,
  );
});

test("manual npm promotion publishes the exact verified platform and meta tarballs with provenance", () => {
  const workflow = readFileSync(
    resolve(repositoryRoot, ".github", "workflows", "publish-npm.yml"),
    "utf8",
  );
  expect(workflow).toContain("workflow_dispatch:");
  expect(workflow).toContain("id-token: write");
  expect(workflow).toContain("environment: npm-production");
  expect(workflow).toContain("secrets.NPM_TOKEN");
  expect(workflow).toContain("Publish with a first-release token or npm trusted publishing");
  expect(workflow).not.toContain('test -n "${NODE_AUTH_TOKEN}"');
  expect(workflow).toContain("dbus-run-session -- xvfb-run -a npm run release:verify");
  expect(workflow).toContain("npm run release:check");
  expect(workflow).toContain("npx playwright install --with-deps chromium");
  expect(workflow).toContain("npm run test:layout");
  const platformPublish =
    "npm publish release/out/verified/alfredo-agent-linux-x64-gnu-${VERSION}.tgz --access public --provenance";
  const metaPublish =
    "npm publish release/out/verified/alfredo-agent-${VERSION}.tgz --access public --provenance";
  expect(workflow).toContain(platformPublish);
  expect(workflow).toContain(metaPublish);
  expect(workflow.indexOf(platformPublish)).toBeLessThan(workflow.indexOf(metaPublish));
  expect(workflow).toContain("alfredo-agent@${VERSION}");
  expect(workflow).toContain("alfredo --version");
  expect(workflow).toContain("github.event.repository.private");
  expect(workflow).toContain('refs/heads/main');
  expect(workflow).toContain("dist.integrity");
  expect(workflow).toContain("EXPECTED_PLATFORM_INTEGRITY");
  expect(workflow).toContain("EXPECTED_META_INTEGRITY");
  expect(workflow).toContain("npm audit signatures --json --include-attestations");
  expect(workflow).toContain("https://slsa.dev/provenance/");
  const platformProvenanceCheck = 'verify_npm_provenance "alfredo-agent-linux-x64-gnu@${VERSION}"';
  const metaProvenanceCheck = 'verify_npm_provenance "alfredo-agent@${VERSION}"';
  expect(workflow).toContain(platformProvenanceCheck);
  expect(workflow).toContain(metaProvenanceCheck);
  expect(workflow.indexOf(platformProvenanceCheck)).toBeLessThan(workflow.indexOf(metaPublish));
  expect(workflow.indexOf(metaPublish)).toBeLessThan(workflow.indexOf(metaProvenanceCheck));
  expect(workflow).toContain('NODE_BIN="$(dirname "$(command -v node)")"');
  expect(workflow).toContain('test "$(command -v alfredo)" = "${PREFIX}/bin/alfredo"');
  expect(workflow).toContain("marker.backend_root");
});
