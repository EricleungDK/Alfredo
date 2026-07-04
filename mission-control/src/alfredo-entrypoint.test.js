import { mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");

test("published package exposes Alfredo and deprecated Albert npm bins", () => {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  const packageLock = JSON.parse(
    readFileSync(resolve(projectRoot, "package-lock.json"), "utf8"),
  );

  expect(packageJson.name).toBe("alfredo");
  expect(packageJson.private).not.toBe(true);
  expect(packageJson.bin).toEqual({
    alfredo: "./bin/alfredo.js",
    albert: "./bin/alfredo.js",
  });
  expect(packageLock.name).toBe("alfredo");
  expect(packageLock.packages[""].name).toBe("alfredo");
  expect(packageLock.packages[""].bin).toEqual(packageJson.bin);
  expect(statSync(resolve(projectRoot, packageJson.bin.alfredo)).mode & 0o111).not.toBe(0);
});

test("alfredo defaults to workstation launch with the selected agent", () => {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  const binPath = resolve(projectRoot, packageJson.bin.alfredo);

  const result = spawnSync(process.execPath, [binPath, "--agent", "qwen3.6-27b"], {
    cwd: projectRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  expect(JSON.parse(result.stdout)).toMatchObject({
    product: "Alfredo",
    launch: "workstation",
    selected_agent: "qwen3.6-27b",
  });
});

test("alfredo workstation carries the selected agent into launch", () => {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  const binPath = resolve(projectRoot, packageJson.bin.alfredo);

  const result = spawnSync(
    process.execPath,
    [binPath, "workstation", "--agent", "gemma4-12b"],
    {
      cwd: projectRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        ALFREDO_DESKTOP_DRY_RUN: "1",
      },
    },
  );

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  expect(JSON.parse(result.stdout)).toMatchObject({
    product: "Alfredo",
    launch: "workstation",
    selected_agent: "gemma4-12b",
  });
});

test("alfredo uses the invocation directory as the selected coding workspace", () => {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  const binPath = resolve(projectRoot, packageJson.bin.alfredo);
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-workspace-"));

  try {
    const result = spawnSync(process.execPath, [binPath, "--agent", "qwen3.6-27b"], {
      cwd: workspace,
      encoding: "utf8",
      env: {
        ...process.env,
        ALFREDO_DESKTOP_DRY_RUN: "1",
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    const plan = JSON.parse(result.stdout);
    expect(plan.project_root).toBe(projectRoot);
    expect(plan.backend_root).toBe(repositoryRoot);
    expect(plan.selected_workspace).toBe(workspace);
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("alfredo rejects an unknown selected agent before launch", () => {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  const binPath = resolve(projectRoot, packageJson.bin.alfredo);

  const result = spawnSync(process.execPath, [binPath, "--agent", "not-configured"], {
    cwd: projectRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.status).toBe(1);
  expect(result.stderr).toContain("Unknown Alfredo agent or model: not-configured");
  expect(result.stderr).toContain(".albert/agents.json");
  expect(result.stdout).toBe("");
});

test("alfredo reports separate startup preflight checks", () => {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  const binPath = resolve(projectRoot, packageJson.bin.alfredo);

  const result = spawnSync(process.execPath, [binPath, "--agent", "qwen3.6-27b"], {
    cwd: projectRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  const plan = JSON.parse(result.stdout);
  expect(plan.preflight.map((check) => check.name)).toEqual([
    "product_install",
    "node_runtime",
    "desktop_shell",
    "backend_process",
    "workspace_access",
    "writable_runtime",
    "ollama",
    "required_model",
  ]);
  expect(
    plan.preflight.every(
      (check) => typeof check.copyable_action === "string" && check.copyable_action.length > 0,
    ),
  ).toBe(true);
});

test("alfredo persists recent workspaces across launches", () => {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  const binPath = resolve(projectRoot, packageJson.bin.alfredo);
  const runtimeRoot = mkdtempSync(resolve(tmpdir(), "alfredo-runtime-"));
  const firstWorkspace = mkdtempSync(resolve(tmpdir(), "alfredo-workspace-one-"));
  const secondWorkspace = mkdtempSync(resolve(tmpdir(), "alfredo-workspace-two-"));

  try {
    const env = {
      ...process.env,
      ALFREDO_DESKTOP_DRY_RUN: "1",
      ALFREDO_RUNTIME_ROOT: runtimeRoot,
    };
    const first = spawnSync(process.execPath, [binPath, "--agent", "qwen3.6-27b"], {
      cwd: firstWorkspace,
      encoding: "utf8",
      env,
    });
    const second = spawnSync(process.execPath, [binPath, "--agent", "qwen3.6-27b"], {
      cwd: secondWorkspace,
      encoding: "utf8",
      env,
    });

    expect(first.status).toBe(0);
    expect(second.status).toBe(0);
    expect(JSON.parse(first.stdout).recent_workspaces).toEqual([firstWorkspace]);
    expect(JSON.parse(second.stdout).recent_workspaces).toEqual([
      secondWorkspace,
      firstWorkspace,
    ]);
  } finally {
    rmSync(runtimeRoot, { recursive: true, force: true });
    rmSync(firstWorkspace, { recursive: true, force: true });
    rmSync(secondWorkspace, { recursive: true, force: true });
  }
});

test("desktop shell presents Alfredo as the app title", () => {
  const tauriConfig = JSON.parse(
    readFileSync(resolve(projectRoot, "src-tauri", "tauri.conf.json"), "utf8"),
  );

  expect(tauriConfig.productName).toBe("Alfredo");
  expect(tauriConfig.app.windows[0].title).toBe("Alfredo");
});
