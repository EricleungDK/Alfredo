import { createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";

const SHA256_PATTERN = /^[a-f0-9]{64}$/;

function fail(message) {
  throw new Error(message);
}

function inside(root, candidate) {
  const suffix = relative(root, candidate);
  return suffix !== "" && !suffix.startsWith("..") && !isAbsolute(suffix);
}

function validateCanonicalFiles(files) {
  if (!Array.isArray(files) || files.length === 0) {
    fail("fixture canonical_files must be a non-empty array");
  }
  const seen = new Set();
  return files
    .map((entry, index) => {
      if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
        fail(`canonical file ${index + 1} must be an object`);
      }
      if (
        typeof entry.path !== "string" ||
        !entry.path ||
        isAbsolute(entry.path) ||
        entry.path.split(/[\\/]/).some((part) => part === "" || part === "." || part === "..")
      ) {
        fail(`canonical file path escapes the fixture root: ${entry.path ?? ""}`);
      }
      if (seen.has(entry.path)) fail(`canonical fixture repeats ${entry.path}`);
      seen.add(entry.path);
      if (typeof entry.content !== "string") {
        fail(`canonical file ${entry.path} content must be UTF-8 text`);
      }
      if (!Number.isSafeInteger(entry.mode) || entry.mode < 0o400 || entry.mode > 0o777) {
        fail(`canonical file ${entry.path} mode must be an integer from 0400 to 0777`);
      }
      return { path: entry.path, mode: entry.mode, content: entry.content };
    })
    .sort((left, right) => left.path.localeCompare(right.path));
}

export function canonicalTreeSha256(files) {
  const hash = createHash("sha256");
  for (const entry of validateCanonicalFiles(files)) {
    const bytes = Buffer.from(entry.content, "utf8");
    hash.update(entry.path);
    hash.update("\0");
    hash.update(entry.mode.toString(8));
    hash.update("\0");
    hash.update(String(bytes.byteLength));
    hash.update("\0");
    hash.update(bytes);
    hash.update("\0");
  }
  return hash.digest("hex");
}

export function materializeCanonicalFixture({ fixture, scratchRoot, installRoot }) {
  if (!isAbsolute(scratchRoot) || !isAbsolute(installRoot)) {
    fail("scratch_root and fixture install root must be absolute");
  }
  const checkedScratch = realpathSync(scratchRoot);
  const absoluteInstall = resolve(installRoot);
  if (!inside(checkedScratch, absoluteInstall)) {
    fail("fixture install root must stay inside scratch_root");
  }
  const files = validateCanonicalFiles(fixture?.canonical_files);
  const expectedSha256 = fixture?.canonical_tree_sha256;
  if (typeof expectedSha256 !== "string" || !SHA256_PATTERN.test(expectedSha256)) {
    fail("fixture canonical_tree_sha256 must be a lowercase SHA-256");
  }
  const declaredSha256 = canonicalTreeSha256(files);
  if (declaredSha256 !== expectedSha256) {
    fail("fixture canonical tree does not match canonical_tree_sha256");
  }

  try {
    const entry = lstatSync(absoluteInstall);
    if (entry.isSymbolicLink() || !entry.isDirectory()) {
      fail("fixture install root must be a directory, never a symlink");
    }
    rmSync(absoluteInstall, { recursive: true });
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  mkdirSync(absoluteInstall, { recursive: true, mode: 0o700 });
  let byteCount = 0;
  for (const file of files) {
    const target = resolve(absoluteInstall, file.path);
    if (!inside(absoluteInstall, target)) {
      fail(`canonical file path escapes the fixture root: ${file.path}`);
    }
    mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
    writeFileSync(target, file.content, {
      encoding: "utf8",
      mode: file.mode,
      flag: "wx",
    });
    chmodSync(target, file.mode);
    byteCount += Buffer.byteLength(file.content, "utf8");
  }

  const installedFiles = files.map((file) => ({
    ...file,
    content: readFileSync(resolve(absoluteInstall, file.path), "utf8"),
  }));
  const installedSha256 = canonicalTreeSha256(installedFiles);
  if (installedSha256 !== expectedSha256) {
    fail("materialized fixture does not match canonical_tree_sha256");
  }
  return {
    fixture_id: fixture.fixture_id,
    install_root: absoluteInstall,
    file_count: files.length,
    byte_count: byteCount,
    canonical_tree_sha256: installedSha256,
  };
}

function requiredAbsoluteInside(scratchRoot, path, label) {
  if (typeof path !== "string" || !isAbsolute(path)) fail(`${label} must be absolute`);
  const absolute = resolve(path);
  if (!inside(scratchRoot, absolute)) fail(`${label} must stay inside scratch_root`);
  return absolute;
}

function wait(milliseconds) {
  return new Promise((resolveWait) => setTimeout(resolveWait, milliseconds));
}

function stopProcessGroup(child) {
  if (!child?.pid) return;
  try {
    process.kill(-child.pid, "SIGCONT");
    process.kill(-child.pid, "SIGTERM");
  } catch (error) {
    if (error?.code !== "ESRCH") child.kill("SIGTERM");
  }
}

function initializeWorkspaceGit(installRoot) {
  const workspace = resolve(installRoot, "workspace");
  try {
    if (!lstatSync(workspace).isDirectory()) return;
  } catch (error) {
    if (error?.code === "ENOENT") return;
    throw error;
  }
  const result = spawnSync("git", ["init", "--quiet", workspace], {
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
  });
  if (result.status !== 0) {
    fail(`fixture workspace Git initialization failed: ${result.stderr.trim()}`);
  }
}

export function createProductionLifecycle({ scratch_root: scratchRoot }) {
  if (!isAbsolute(scratchRoot)) fail("scratch_root must be absolute");
  const checkedScratch = realpathSync(scratchRoot);

  return {
    async startWarmSession({ run_id: runId, cohort_id: cohortId, variant, fixture }) {
      const config = variant?.warm_session;
      if (!config || typeof config !== "object" || Array.isArray(config)) {
        fail(`variant ${variant?.name ?? ""}.warm_session is required`);
      }
      const launchCommand = config.launch_command;
      if (!Array.isArray(launchCommand) || launchCommand.length === 0) {
        fail(`variant ${variant.name}.warm_session.launch_command is required`);
      }
      const launcher = realpathSync(launchCommand[0]);
      if (launcher !== variant.installed_launcher_path) {
        fail(`variant ${variant.name} warm desktop must use the exact installed launcher`);
      }
      const installRoot = requiredAbsoluteInside(
        checkedScratch,
        config.fixture_install_root,
        "fixture_install_root",
      );
      const readyMarker = requiredAbsoluteInside(
        checkedScratch,
        config.ready_marker_path,
        "ready_marker_path",
      );
      const controlPath = requiredAbsoluteInside(
        checkedScratch,
        config.control_path,
        "control_path",
      );
      materializeCanonicalFixture({ fixture, scratchRoot: checkedScratch, installRoot });
      initializeWorkspaceGit(installRoot);
      for (const path of [readyMarker, controlPath]) {
        try {
          unlinkSync(path);
        } catch (error) {
          if (error?.code !== "ENOENT") throw error;
        }
      }
      mkdirSync(dirname(readyMarker), { recursive: true, mode: 0o700 });
      mkdirSync(dirname(controlPath), { recursive: true, mode: 0o700 });
      const desktopSessionId = `${runId}-${cohortId}-${variant.name}-desktop`;
      const child = spawn(launcher, launchCommand.slice(1), {
        cwd: config.cwd ? realpathSync(config.cwd) : dirname(launcher),
        env: {
          ...process.env,
          ...(variant.environment ?? {}),
          ...(config.environment ?? {}),
          ALFREDO_GUI_SMOKE: "1",
          ALFREDO_WARM_READY_MARKER: readyMarker,
          ALFREDO_MEASUREMENT_CONTROL_PATH: controlPath,
          ALFREDO_MEASUREMENT_DESKTOP_SESSION_ID: desktopSessionId,
          ALFREDO_RUNTIME_ROOT: resolve(installRoot, "runtime"),
        },
        detached: true,
        stdio: ["ignore", "ignore", "pipe"],
      });
      let stderr = "";
      let exit = null;
      child.stderr.setEncoding("utf8");
      child.stderr.on("data", (chunk) => {
        stderr = `${stderr}${chunk}`.slice(-16_384);
      });
      child.on("close", (code, signal) => {
        exit = { code, signal };
      });
      const deadline = Date.now() + (config.timeout_ms ?? 120_000);
      while (Date.now() <= deadline) {
        if (exit) {
          fail(
            `warm desktop exited before readiness ` +
              `(code=${exit.code ?? "null"}, signal=${exit.signal ?? "none"}): ${stderr}`,
          );
        }
        try {
          const marker = JSON.parse(readFileSync(readyMarker, "utf8"));
          if (
            marker?.schema_version === 1 &&
            marker.status === "ready" &&
            Number.isSafeInteger(marker.process_id) &&
            marker.process_id > 0 &&
            marker.desktop_session_id === desktopSessionId
          ) {
            return {
              child,
              desktop_pid: marker.process_id,
              desktop_session_id: desktopSessionId,
              install_root: installRoot,
              control_path: controlPath,
              ready_marker_path: readyMarker,
            };
          }
        } catch (error) {
          if (error?.code !== "ENOENT" && !(error instanceof SyntaxError)) throw error;
        }
        await wait(5);
      }
      stopProcessGroup(child);
      fail("warm desktop timed out before an exact readiness handshake");
    },

    async prepareSample(request, session = null) {
      const config = request.variant?.warm_session;
      const installRoot = session?.install_root ?? resolve(
        checkedScratch,
        "process-cold",
        request.fixture.fixture_id,
      );
      const proof = materializeCanonicalFixture({
        fixture: request.fixture,
        scratchRoot: checkedScratch,
        installRoot,
      });
      initializeWorkspaceGit(installRoot);
      if (!session) return { fixture_proof: proof };

      const identity = {
        jsonl_path: request.raw_jsonl,
        run_id: request.run_id,
        sample_id: request.sample_id,
        cohort_id: request.cohort_id,
        correlation_id: request.correlation_id,
        fixture_id: request.fixture.fixture_id,
        fixture_sha256: request.fixture.sha256,
        source_sha256: request.variant.source_sha256,
        artifact_sha256: request.variant.artifact_sha256,
        variant: request.variant.name,
        workflow: request.workflow,
        mode: request.mode,
        desktop_pid: session.desktop_pid,
        desktop_session_id: session.desktop_session_id,
      };
      const temporary = `${session.control_path}.tmp-${process.pid}`;
      writeFileSync(temporary, `${JSON.stringify(identity)}\n`, {
        encoding: "utf8",
        mode: 0o600,
        flag: "wx",
      });
      renameSync(temporary, session.control_path);
      return {
        fixture_proof: proof,
        desktop_pid: session.desktop_pid,
        desktop_session_id: session.desktop_session_id,
        control_path: config.control_path,
      };
    },

    async activateWarmSession(session) {
      if (!session?.child?.pid) fail("warm desktop session has no owned process group");
      process.kill(-session.child.pid, "SIGCONT");
    },

    async deactivateWarmSession(session) {
      if (!session?.child?.pid) fail("warm desktop session has no owned process group");
      process.kill(-session.child.pid, "SIGSTOP");
    },

    async stopWarmSession(session) {
      stopProcessGroup(session?.child);
      if (session?.child && session.child.exitCode === null) {
        await Promise.race([
          new Promise((resolveClose) => session.child.once("close", resolveClose)),
          wait(2_000),
        ]);
      }
    },
  };
}
