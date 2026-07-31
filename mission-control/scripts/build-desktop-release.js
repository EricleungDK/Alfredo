import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(readFileSync(resolve(projectRoot, "package.json"), "utf8"));
const targetDirectory = resolve(
  process.env.ALFREDO_CARGO_TARGET_DIR ??
    resolve(tmpdir(), `alfredo-tauri-target-${manifest.version}`),
);
mkdirSync(targetDirectory, { recursive: true });

const tauriCommand = resolve(
  projectRoot,
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tauri.cmd" : "tauri",
);
const built = spawnSync(tauriCommand, ["build", "--bundles", "appimage"], {
  cwd: projectRoot,
  env: {
    ...process.env,
    CARGO_TARGET_DIR: targetDirectory,
  },
  stdio: "inherit",
});
if (built.error) throw built.error;
if (built.status !== 0) process.exit(built.status ?? 1);

if (!process.argv.includes("--native-only")) {
  const packaged = spawnSync(
    process.execPath,
    [
      resolve(projectRoot, "scripts", "build-npm-release.js"),
      "build",
      "--target-dir",
      targetDirectory,
    ],
    { cwd: projectRoot, stdio: "inherit" },
  );
  if (packaged.error) throw packaged.error;
  if (packaged.status !== 0) process.exit(packaged.status ?? 1);
}

process.stdout.write(`${JSON.stringify({ cargo_target_dir: targetDirectory })}\n`);
