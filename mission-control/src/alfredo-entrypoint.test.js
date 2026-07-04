import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");

function alfredoBinPath() {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  return resolve(projectRoot, packageJson.bin.alfredo);
}

function runAlfredo(args, options = {}) {
  return spawnSync(process.execPath, [alfredoBinPath(), ...args], {
    cwd: options.cwd ?? projectRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      ...(options.env ?? {}),
    },
  });
}

function createHeadlessWorkspace() {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-headless-root-"));
  const workspace = resolve(root, "workspace");
  mkdirSync(workspace, { recursive: true });
  const issuesDir = resolve(workspace, ".agent", "issues");
  mkdirSync(issuesDir, { recursive: true });
  writeFileSync(
    resolve(issuesDir, "PRD.md"),
    "# Headless Alfredo Test Product Requirements Document\n",
    "utf8",
  );
  writeFileSync(
    resolve(issuesDir, "01-smoke.md"),
    [
      "# Smoke",
      "",
      "Status: ready-for-agent",
      "Type: AFK",
      "",
      "## What to build",
      "",
      "Exercise the headless command path.",
      "",
      "## Acceptance criteria",
      "",
      "- [ ] Headless lifecycle output is available.",
      "",
      "## Blocked by",
      "",
      "None - can start immediately.",
      "",
    ].join("\n"),
    "utf8",
  );
  const agentConfig = resolve(workspace, "agents.json");
  writeFileSync(
    agentConfig,
    `${JSON.stringify(
      {
        agents: [
          {
            id: "fake-local",
            role: "local-agent",
            provider: "test",
            runner: "fake",
            model: "deterministic-fake",
          },
          {
            id: "fake-reviewer",
            role: "frontier",
            provider: "test",
            runner: "fake",
            model: "deterministic-reviewer",
          },
        ],
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  return {
    root,
    workspace,
    runtimeRoot: resolve(root, "runtime"),
    agentConfig,
  };
}

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
  const result = runAlfredo(["--agent", "qwen3.6-27b"], {
    env: {
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  expect(JSON.parse(result.stdout)).toMatchObject({
    product: "Alfredo",
    launch: "workstation",
    selected_agent: "qwen3.6-27b",
    selected_model: "qwen3.6:27b",
  });
});

test("alfredo workstation carries the selected agent into launch", () => {
  const result = spawnSync(
    process.execPath,
    [alfredoBinPath(), "workstation", "--agent", "gemma4-12b"],
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
    selected_model: "gemma4:12b",
  });
});

test("alfredo accepts a configured model name as the selected controller", () => {
  const result = runAlfredo(["--agent", "qwen3.6:27b"], {
    env: {
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  expect(JSON.parse(result.stdout)).toMatchObject({
    product: "Alfredo",
    launch: "workstation",
    selected_agent: "qwen3.6:27b",
    selected_model: "qwen3.6:27b",
  });
});

test("alfredo uses the invocation directory as the selected coding workspace", () => {
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-workspace-"));

  try {
    const result = spawnSync(process.execPath, [alfredoBinPath(), "--agent", "qwen3.6-27b"], {
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
  const result = runAlfredo(["--agent", "not-configured"], {
    env: {
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.status).toBe(1);
  expect(result.stderr).toContain("Unknown Alfredo agent or model: not-configured");
  expect(result.stderr).toContain(".albert/agents.json");
  expect(result.stdout).toBe("");
});

test("alfredo reports separate startup preflight checks", () => {
  const result = runAlfredo(["--agent", "qwen3.6-27b"], {
    env: {
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  const plan = JSON.parse(result.stdout);
  expect(plan.preflight.map((check) => check.name)).toEqual([
    "product_install",
    "node_runtime",
    "npm_runtime",
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

test("alfredo launch passes selected agent, model, workspace, and runtime to desktop", () => {
  const runtimeRoot = mkdtempSync(resolve(tmpdir(), "alfredo-runtime-"));
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-workspace-"));

  try {
    const result = runAlfredo(["--agent", "qwen3.6-27b"], {
      cwd: workspace,
      env: {
        ALFREDO_DESKTOP_DRY_RUN: "launch",
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    const launch = JSON.parse(result.stdout);
    expect(launch.command).toEqual(["npm", "run", "desktop"]);
    expect(launch.cwd).toBe(projectRoot);
    expect(launch.env).toMatchObject({
      ALBERT_BACKEND_ROOT: repositoryRoot,
      ALFREDO_SELECTED_AGENT: "qwen3.6-27b",
      ALFREDO_SELECTED_MODEL: "qwen3.6:27b",
      ALFREDO_SELECTED_WORKSPACE: workspace,
      ALFREDO_RUNTIME_ROOT: runtimeRoot,
    });
  } finally {
    rmSync(runtimeRoot, { recursive: true, force: true });
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("alfredo reports an unwritable runtime root as a structured preflight failure", () => {
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-workspace-"));
  const runtimeParent = mkdtempSync(resolve(tmpdir(), "alfredo-runtime-parent-"));
  const runtimeRoot = resolve(runtimeParent, "runtime-file");
  writeFileSync(runtimeRoot, "not a directory", "utf8");

  try {
    const result = runAlfredo(["--agent", "qwen3.6-27b"], {
      cwd: workspace,
      env: {
        ALFREDO_DESKTOP_DRY_RUN: "launch",
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
      },
    });

    expect(result.status).toBe(1);
    expect(result.stdout).toBe("");
    expect(result.stderr).toContain("Alfredo startup preflight failed:");
    expect(result.stderr).toContain("writable_runtime");
    expect(result.stderr).toContain(`mkdir -p "${runtimeRoot}" && test -w "${runtimeRoot}"`);
  } finally {
    rmSync(workspace, { recursive: true, force: true });
    rmSync(runtimeParent, { recursive: true, force: true });
  }
});

test("alfredo agents lists configured public agent ids and models", () => {
  const result = runAlfredo(["agents"]);

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  expect(result.stdout).toContain("qwen3.6-27b\tfrontier\tollama\tollama:qwen3.6:27b");
  expect(result.stdout).toContain("gemma4-12b\tlocal-agent\tollama\tollama:gemma4:12b");
});

test("alfredo run executes terminal-only work through the Orchestrator", () => {
  const { root, workspace, runtimeRoot, agentConfig } = createHeadlessWorkspace();

  try {
    const result = runAlfredo(["run", "--agent", "fake-local", "Summarize this repo"], {
      cwd: workspace,
      env: {
        ALFREDO_AGENT_CONFIG: agentConfig,
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    expect(JSON.parse(result.stdout)).toMatchObject({
      product: "Alfredo",
      launch: "headless-run",
      selected_agent: "fake-local",
      selected_model: "deterministic-fake",
      prompt: "Summarize this repo",
      status: "evidence-ready",
      governance: {
        orchestrator: "AlbertMission",
        evidence_package: "valid",
        path_boundary: "allowed_paths",
      },
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("alfredo review accepts optional session id and executes through the Orchestrator", () => {
  const { root, workspace, runtimeRoot, agentConfig } = createHeadlessWorkspace();

  try {
    const runResult = runAlfredo(["run", "--agent", "fake-local", "Summarize this repo"], {
      cwd: workspace,
      env: {
        ALFREDO_AGENT_CONFIG: agentConfig,
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
      },
    });
    expect(runResult.stderr).toBe("");
    expect(runResult.status).toBe(0);
    const priorSessionId = JSON.parse(runResult.stdout).session_id;

    const result = runAlfredo(["review", priorSessionId, "--agent", "fake-reviewer"], {
      cwd: workspace,
      env: {
        ALFREDO_AGENT_CONFIG: agentConfig,
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    expect(JSON.parse(result.stdout)).toMatchObject({
      product: "Alfredo",
      launch: "headless-review",
      selected_agent: "fake-reviewer",
      selected_model: "deterministic-reviewer",
      review_session_id: priorSessionId,
      review_context: {
        session_id: priorSessionId,
        evidence_valid: true,
      },
      status: "evidence-ready",
      governance: {
        orchestrator: "AlbertMission",
        evidence_package: "valid",
      },
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("alfredo run rejects model aliases because per-command agent id is canonical", () => {
  const result = runAlfredo(["run", "--agent", "qwen3.6:27b", "Summarize this repo"], {
    env: {
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.status).toBe(1);
  expect(result.stderr).toContain("Unknown Alfredo agent id: qwen3.6:27b");
  expect(result.stderr).toContain("alfredo agents");
  expect(result.stdout).toBe("");
});

test("alfredo persists recent workspaces across launches", () => {
  const runtimeRoot = mkdtempSync(resolve(tmpdir(), "alfredo-runtime-"));
  const firstWorkspace = mkdtempSync(resolve(tmpdir(), "alfredo-workspace-one-"));
  const secondWorkspace = mkdtempSync(resolve(tmpdir(), "alfredo-workspace-two-"));

  try {
    const env = {
      ...process.env,
      ALFREDO_DESKTOP_DRY_RUN: "1",
      ALFREDO_RUNTIME_ROOT: runtimeRoot,
    };
    const first = spawnSync(process.execPath, [alfredoBinPath(), "--agent", "qwen3.6-27b"], {
      cwd: firstWorkspace,
      encoding: "utf8",
      env,
    });
    const second = spawnSync(process.execPath, [alfredoBinPath(), "--agent", "qwen3.6-27b"], {
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
