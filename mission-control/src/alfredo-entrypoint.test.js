import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");

// Subprocess-heavy launcher cases need a file-local worker budget when the
// complete frontend suite is contending for process and filesystem resources.
vi.setConfig({ testTimeout: 15_000 });

function binPath(name) {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  return resolve(projectRoot, packageJson.bin[name]);
}

function alfredoBinPath() {
  return binPath("alfredo");
}

function albertBinPath() {
  return binPath("albert");
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

function normalizeBinPaths(bin) {
  return Object.fromEntries(
    Object.entries(bin).map(([name, path]) => [
      name,
      path.startsWith("./") ? path : `./${path}`,
    ]),
  );
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
            role: "local-agent",
            provider: "test",
            runner: "fake",
            model: "deterministic-reviewer",
            routing: "worker",
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

test("development package keeps Alfredo bins while public release packaging stays separate", () => {
  const packageJson = JSON.parse(
    readFileSync(resolve(projectRoot, "package.json"), "utf8"),
  );
  const packageLock = JSON.parse(
    readFileSync(resolve(projectRoot, "package-lock.json"), "utf8"),
  );

  expect(packageJson.name).toBe("alfredo-workstation-development");
  expect(packageJson.private).toBe(true);
  expect(packageJson.bin).toEqual({
    alfredo: "bin/alfredo.js",
    albert: "bin/alfredo.js",
  });
  expect(packageLock.name).toBe("alfredo-workstation-development");
  expect(packageLock.packages[""].name).toBe("alfredo-workstation-development");
  expect(normalizeBinPaths(packageLock.packages[""].bin)).toEqual(
    normalizeBinPaths(packageJson.bin),
  );
  expect(statSync(resolve(projectRoot, packageJson.bin.alfredo)).mode & 0o111).not.toBe(0);
});

test("alfredo exposes a valid public version command", () => {
  const result = runAlfredo(["--version"]);

  expect(result.status).toBe(0);
  expect(result.stderr).toBe("");
  expect(result.stdout).toBe("Alfredo 0.1.0\n");
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

test("alfredo with no subcommand opens the workstation as the default launch intent", () => {
  const result = runAlfredo([], {
    env: {
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  expect(JSON.parse(result.stdout)).toMatchObject({
    product: "Alfredo",
    launch: "workstation",
    selected_agent: "qwen3-14b",
    selected_model: "qwen3:14b",
  });
});

test("alfredo workstation rejects a worker as the selected controller", () => {
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

  expect(result.status).toBe(1);
  expect(result.stdout).toBe("");
  expect(result.stderr).toContain("not an eligible Alfredo workstation controller");
  expect(result.stderr).toContain("gemma4-12b");
});

test("alfredo workstation rejects a cloud model as its local controller", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-cloud-controller-"));
  const agentConfig = resolve(root, "agents.json");
  writeFileSync(
    agentConfig,
    `${JSON.stringify({
      agents: [
        {
          id: "cloud-controller",
          role: "frontier",
          provider: "ollama",
          runner: "ollama",
          model: "qwen:cloud",
          routing: "controller",
        },
      ],
    })}\n`,
    "utf8",
  );

  try {
    const result = runAlfredo(["--agent", "cloud-controller"], {
      env: {
        ALFREDO_AGENT_CONFIG: agentConfig,
        ALFREDO_DESKTOP_DRY_RUN: "1",
      },
    });

    expect(result.status).toBe(1);
    expect(result.stdout).toBe("");
    expect(result.stderr).toContain("not an eligible Alfredo workstation controller");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("alfredo workstation rejects controllers without local provider and runner boundaries", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-remote-controller-"));
  const agentConfig = resolve(root, "agents.json");

  try {
    for (const controller of [
      {
        id: "remote-provider-controller",
        role: "frontier",
        provider: "remote",
        runner: "ollama",
        model: "qwen:14b",
        routing: "controller",
      },
      {
        id: "remote-runner-controller",
        role: "frontier",
        provider: "local",
        runner: "remote-api",
        model: "qwen:14b",
        routing: "controller",
      },
    ]) {
      writeFileSync(
        agentConfig,
        `${JSON.stringify({ agents: [controller] })}\n`,
        "utf8",
      );
      const result = runAlfredo(["--agent", controller.id], {
        env: {
          ALFREDO_AGENT_CONFIG: agentConfig,
          ALFREDO_DESKTOP_DRY_RUN: "1",
        },
      });

      expect(result.status).toBe(1);
      expect(result.stdout).toBe("");
      expect(result.stderr).toContain("not an eligible Alfredo workstation controller");
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("alfredo workstation rejects an unavailable configured controller", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-unavailable-controller-"));
  const agentConfig = resolve(root, "agents.json");
  writeFileSync(
    agentConfig,
    `${JSON.stringify({
      agents: [{
        id: "offline-controller",
        role: "frontier",
        provider: "local",
        runner: "fake",
        routing: "controller",
        availability: "disconnected",
        availability_reason: "controller is offline",
      }],
    })}\n`,
    "utf8",
  );

  try {
    const result = runAlfredo(["--agent", "offline-controller"], {
      env: {
        ALFREDO_AGENT_CONFIG: agentConfig,
        ALFREDO_DESKTOP_DRY_RUN: "1",
      },
    });

    expect(result.status).toBe(1);
    expect(result.stdout).toBe("");
    expect(result.stderr).toContain("not an eligible Alfredo workstation controller");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
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
    selected_agent: "qwen3.6-27b",
    selected_model: "qwen3.6:27b",
  });
});

test("alfredo uses the invocation directory only as the starting location", () => {
  const startingLocation = mkdtempSync(resolve(tmpdir(), "alfredo-starting-location-"));

  try {
    const result = spawnSync(process.execPath, [alfredoBinPath(), "--agent", "qwen3.6-27b"], {
      cwd: startingLocation,
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
    expect(plan.starting_location).toBe(startingLocation);
    expect(plan.workspace_selection).toEqual({
      schema_version: 1,
      phase: "selection-required",
      starting_location: startingLocation,
      coding_workspace: null,
      active_mission: null,
    });
    expect(plan).not.toHaveProperty("selected_workspace");
    expect(plan).not.toHaveProperty("tracker_dir");
    expect(plan).not.toHaveProperty("issues_dir");
    expect(plan).not.toHaveProperty("mission_id");
  } finally {
    rmSync(startingLocation, { recursive: true, force: true });
  }
});

test("alfredo does not bind a tracker from the Starting Location before selection", () => {
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-tracker-workspace-"));
  const oldTracker = resolve(workspace, ".scratch", "old-mission");
  const activeTracker = resolve(workspace, ".scratch", "active-mission");
  mkdirSync(resolve(oldTracker, "issues"), { recursive: true });
  mkdirSync(resolve(activeTracker, "issues"), { recursive: true });
  writeFileSync(resolve(oldTracker, "PRD.md"), "# Old mission\n", "utf8");
  writeFileSync(resolve(activeTracker, "PRD.md"), "# Active mission\n", "utf8");
  writeFileSync(
    resolve(oldTracker, "issues", "01-old.md"),
    "# Old\n\nStatus: complete\n",
    "utf8",
  );
  writeFileSync(
    resolve(activeTracker, "issues", "01-active.md"),
    "# Active\n\nStatus: Completed\n",
    "utf8",
  );
  utimesSync(resolve(oldTracker, "PRD.md"), new Date(1_000), new Date(1_000));
  utimesSync(resolve(activeTracker, "PRD.md"), new Date(2_000), new Date(2_000));

  try {
    const result = runAlfredo([], {
      cwd: workspace,
      env: {
        ALFREDO_DESKTOP_DRY_RUN: "1",
        ALBERT_TRACKER_DIR: "",
        ALBERT_ISSUES_DIR: "",
        ALBERT_MISSION_ID: "",
      },
    });

    expect(result.status).toBe(0);
    const plan = JSON.parse(result.stdout);
    expect(plan.starting_location).toBe(workspace);
    expect(plan).not.toHaveProperty("tracker_dir");
    expect(plan).not.toHaveProperty("mission_id");
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("alfredo does not infer a Mission from ready issue work before selection", () => {
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-ready-tracker-workspace-"));
  const completedTracker = resolve(
    workspace,
    ".scratch",
    "alfredo-console-first-workstation-redesign",
  );
  const agentTracker = resolve(workspace, ".agent", "issues");
  mkdirSync(resolve(completedTracker, "issues"), { recursive: true });
  mkdirSync(agentTracker, { recursive: true });
  writeFileSync(
    resolve(completedTracker, "PRD.md"),
    "Status: ready-for-agent\n\n# Newer redesign PRD\n",
    "utf8",
  );
  writeFileSync(
    resolve(completedTracker, "issues", "01-layout.md"),
    "# Layout\n\nStatus: complete\n",
    "utf8",
  );
  writeFileSync(
    resolve(agentTracker, "PRD.md"),
    "Status: ready-for-agent\n\n# Command Deck PRD\n",
    "utf8",
  );
  writeFileSync(
    resolve(agentTracker, "18-rebuild-workstation.md"),
    "# Rebuild workstation\n\nStatus: ready-for-agent\n",
    "utf8",
  );
  utimesSync(resolve(agentTracker, "PRD.md"), new Date(1_000), new Date(1_000));
  utimesSync(resolve(completedTracker, "PRD.md"), new Date(2_000), new Date(2_000));

  try {
    const result = runAlfredo([], {
      cwd: workspace,
      env: {
        ALFREDO_DESKTOP_DRY_RUN: "1",
        ALBERT_TRACKER_DIR: "",
        ALBERT_ISSUES_DIR: "",
        ALBERT_MISSION_ID: "",
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    const plan = JSON.parse(result.stdout);
    expect(plan.starting_location).toBe(workspace);
    expect(plan).not.toHaveProperty("tracker_dir");
    expect(plan).not.toHaveProperty("mission_id");
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("alfredo keeps tracker identity unbound while Coding Workspace selection is pending", () => {
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-tracker-identity-workspace-"));
  const scratchTracker = resolve(workspace, ".scratch", "issues");
  const agentTracker = resolve(workspace, ".agent", "issues");
  mkdirSync(resolve(scratchTracker, "issues"), { recursive: true });
  mkdirSync(agentTracker, { recursive: true });
  writeFileSync(resolve(scratchTracker, "PRD.md"), "# Completed scratch tracker\n", "utf8");
  writeFileSync(
    resolve(scratchTracker, "issues", "01-done.md"),
    "# Done\n\nStatus: complete\n",
    "utf8",
  );
  writeFileSync(resolve(agentTracker, "PRD.md"), "# Agent tracker\n", "utf8");
  writeFileSync(
    resolve(agentTracker, "01-ready.md"),
    "# Ready\n\nStatus: ready-for-human\n",
    "utf8",
  );

  try {
    const result = runAlfredo([], {
      cwd: workspace,
      env: {
        ALFREDO_DESKTOP_DRY_RUN: "1",
        ALBERT_TRACKER_DIR: "",
        ALBERT_ISSUES_DIR: "",
        ALBERT_MISSION_ID: "",
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    const plan = JSON.parse(result.stdout);
    expect(plan.starting_location).toBe(workspace);
    expect(plan).not.toHaveProperty("tracker_dir");
    expect(plan).not.toHaveProperty("mission_id");
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("alfredo does not inspect launchable work before a repository is acknowledged", () => {
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-launchable-tracker-workspace-"));
  const agentWork = resolve(workspace, ".scratch", "agent-work");
  const humanReview = resolve(workspace, ".scratch", "human-review");
  for (const tracker of [agentWork, humanReview]) {
    mkdirSync(resolve(tracker, "issues"), { recursive: true });
    writeFileSync(resolve(tracker, "PRD.md"), `# ${tracker}\n`, "utf8");
  }
  writeFileSync(
    resolve(agentWork, "issues", "01-agent.md"),
    "# Agent work\n\nStatus: ready-for-agent\n",
    "utf8",
  );
  writeFileSync(
    resolve(humanReview, "issues", "01-human.md"),
    "# Human review\n\nStatus: ready-for-human\n",
    "utf8",
  );
  utimesSync(resolve(agentWork, "PRD.md"), new Date(1_000), new Date(1_000));
  utimesSync(resolve(humanReview, "PRD.md"), new Date(2_000), new Date(2_000));

  try {
    const result = runAlfredo([], {
      cwd: workspace,
      env: {
        ALFREDO_DESKTOP_DRY_RUN: "1",
        ALBERT_TRACKER_DIR: "",
        ALBERT_ISSUES_DIR: "",
        ALBERT_MISSION_ID: "",
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    const plan = JSON.parse(result.stdout);
    expect(plan.starting_location).toBe(workspace);
    expect(plan).not.toHaveProperty("tracker_dir");
    expect(plan).not.toHaveProperty("mission_id");
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("alfredo leaves equally ready trackers unbound before repository selection", () => {
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-tracker-tie-workspace-"));
  const alphaTracker = resolve(workspace, ".scratch", "alpha-mission");
  const zuluTracker = resolve(workspace, ".scratch", "zulu-mission");
  for (const tracker of [zuluTracker, alphaTracker]) {
    mkdirSync(resolve(tracker, "issues"), { recursive: true });
    writeFileSync(resolve(tracker, "PRD.md"), `# ${tracker}\n`, "utf8");
    writeFileSync(
      resolve(tracker, "issues", "01-ready.md"),
      "# Ready\n\nStatus: ready-for-agent\n",
      "utf8",
    );
    utimesSync(resolve(tracker, "PRD.md"), new Date(1_000), new Date(1_000));
  }

  try {
    const result = runAlfredo([], {
      cwd: workspace,
      env: {
        ALFREDO_DESKTOP_DRY_RUN: "1",
        ALBERT_TRACKER_DIR: "",
        ALBERT_ISSUES_DIR: "",
        ALBERT_MISSION_ID: "",
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    const plan = JSON.parse(result.stdout);
    expect(plan.starting_location).toBe(workspace);
    expect(plan).not.toHaveProperty("tracker_dir");
    expect(plan).not.toHaveProperty("mission_id");
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("alfredo does not bind even an explicit tracker before repository selection", () => {
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-explicit-tracker-workspace-"));
  const explicitTracker = resolve(workspace, "chosen-tracker");
  const discoveredTracker = resolve(workspace, ".agent", "issues");
  mkdirSync(resolve(explicitTracker, "issues"), { recursive: true });
  mkdirSync(discoveredTracker, { recursive: true });
  writeFileSync(resolve(explicitTracker, "PRD.md"), "# Explicit tracker\n", "utf8");
  writeFileSync(
    resolve(explicitTracker, "issues", "01-done.md"),
    "# Done\n\nStatus: complete\n",
    "utf8",
  );
  writeFileSync(resolve(discoveredTracker, "PRD.md"), "# Discovered tracker\n", "utf8");
  writeFileSync(
    resolve(discoveredTracker, "01-ready.md"),
    "# Ready\n\nStatus: ready-for-agent\n",
    "utf8",
  );

  try {
    const result = runAlfredo([], {
      cwd: workspace,
      env: {
        ALFREDO_DESKTOP_DRY_RUN: "1",
        ALBERT_TRACKER_DIR: explicitTracker,
        ALBERT_ISSUES_DIR: "",
        ALBERT_MISSION_ID: "explicit-mission",
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    const plan = JSON.parse(result.stdout);
    expect(plan.starting_location).toBe(workspace);
    expect(plan).not.toHaveProperty("tracker_dir");
    expect(plan).not.toHaveProperty("mission_id");
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
    "sandbox_runtime",
    "starting_location_access",
    "writable_runtime",
    "ollama",
    "required_model",
  ]);
  expect(
    plan.preflight.every(
      (check) => typeof check.copyable_action === "string" && check.copyable_action.length > 0,
    ),
  ).toBe(true);
  expect(plan.preflight.find((check) => check.name === "sandbox_runtime")).toMatchObject({
    status: "not_run",
    copyable_action: "bwrap --version",
  });
});

test("alfredo reports missing lockfile dependencies before starting the development desktop", () => {
  const sourceRoot = mkdtempSync(resolve(tmpdir(), "alfredo-missing-tauri-source-"));
  const sourceProject = resolve(sourceRoot, "mission-control");
  const sourceBin = resolve(sourceProject, "bin");
  const sourceScripts = resolve(sourceProject, "scripts");
  const runtimeRoot = resolve(sourceRoot, "runtime");
  mkdirSync(sourceBin, { recursive: true });
  mkdirSync(sourceScripts, { recursive: true });
  mkdirSync(resolve(sourceRoot, "albert_mvp"));
  mkdirSync(resolve(sourceRoot, ".albert"));
  writeFileSync(resolve(sourceProject, "package.json"), '{"type":"module"}\n', "utf8");
  writeFileSync(resolve(sourceRoot, ".albert", "agents.json"), '{"agents":[]}\n', "utf8");
  copyFileSync(alfredoBinPath(), resolve(sourceBin, "alfredo.js"));
  copyFileSync(
    resolve(projectRoot, "bin", "desktop-adapter.js"),
    resolve(sourceBin, "desktop-adapter.js"),
  );
  copyFileSync(
    resolve(projectRoot, "scripts", "performance-recorder.js"),
    resolve(sourceScripts, "performance-recorder.js"),
  );

  try {
    const result = spawnSync(
      process.execPath,
      [resolve(sourceBin, "alfredo.js"), "--agent", "qwen3.6-27b"],
      {
        cwd: sourceRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          ALBERT_BACKEND_ROOT: repositoryRoot,
          ALFREDO_RUNTIME_ROOT: runtimeRoot,
        },
      },
    );

    expect(result.status).toBe(1);
    expect(result.stdout).toBe("");
    expect(result.stderr).toContain("Alfredo startup preflight failed:");
    expect(result.stderr).toContain("Local Tauri CLI is unavailable");
    expect(result.stderr).toContain("npm ci");
    expect(result.stderr).not.toContain("tauri: command not found");
  } finally {
    rmSync(sourceRoot, { recursive: true, force: true });
  }
});

test("alfredo reports a missing Cargo toolchain before starting the development desktop", () => {
  const runtimeRoot = mkdtempSync(resolve(tmpdir(), "alfredo-missing-cargo-runtime-"));

  try {
    const result = runAlfredo(["--agent", "qwen3.6-27b"], {
      env: {
        CARGO: "alfredo-cargo-unavailable-for-test",
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
      },
    });

    expect(result.status).toBe(1);
    expect(result.stdout).toBe("");
    expect(result.stderr).toContain("Alfredo startup preflight failed:");
    expect(result.stderr).toContain("desktop_shell");
    expect(result.stderr).toContain("Cargo is unavailable");
    expect(result.stderr).toContain("alfredo-cargo-unavailable-for-test --version");
    expect(result.stderr).not.toContain("failed to run cargo metadata");
  } finally {
    rmSync(runtimeRoot, { recursive: true, force: true });
  }
});

test("alfredo launch passes controller, Starting Location, and runtime without a workspace", () => {
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
      ALFREDO_INSTALL_ROOT: projectRoot,
      ALFREDO_STARTING_LOCATION: workspace,
      ALFREDO_SELECTED_AGENT: "qwen3.6-27b",
      ALFREDO_SELECTED_MODEL: "qwen3.6:27b",
      ALFREDO_RUNTIME_ROOT: runtimeRoot,
    });
    expect(launch.env).not.toHaveProperty("ALFREDO_SELECTED_WORKSPACE");
    expect(launch.env).not.toHaveProperty("ALBERT_MISSION_ID");
  } finally {
    rmSync(runtimeRoot, { recursive: true, force: true });
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("alfredo resolves a relative custom registry before changing desktop cwd", () => {
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-relative-registry-"));
  const workspace = resolve(root, "workspace");
  const runtimeRoot = resolve(root, "runtime");
  const registryDir = resolve(workspace, "config");
  const agentConfig = resolve(registryDir, "agents.json");
  mkdirSync(registryDir, { recursive: true });
  writeFileSync(
    agentConfig,
    `${JSON.stringify({
      agents: [
        {
          id: "custom-controller",
          role: "frontier",
          provider: "test",
          runner: "fake",
          model: "deterministic-controller",
          routing: "controller",
        },
      ],
    })}\n`,
    "utf8",
  );

  try {
    const result = runAlfredo(["--agent", "custom-controller"], {
      cwd: workspace,
      env: {
        ALFREDO_AGENT_CONFIG: "config/agents.json",
        ALFREDO_DESKTOP_DRY_RUN: "launch",
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    expect(JSON.parse(result.stdout).env.ALFREDO_AGENT_CONFIG).toBe(agentConfig);
  } finally {
    rmSync(root, { recursive: true, force: true });
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

test("alfredo review without a session id keeps review-oriented headless intent", () => {
  const result = runAlfredo(["review", "--agent", "gemma4-12b"], {
    env: {
      ALFREDO_DESKTOP_DRY_RUN: "1",
    },
  });

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  expect(JSON.parse(result.stdout)).toMatchObject({
    product: "Alfredo",
    launch: "headless-review",
    selected_agent: "gemma4-12b",
    selected_model: "gemma4:12b",
    session_id: "",
  });
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

test("alfredo does not turn invocation directories into recent workspaces", () => {
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
    expect(JSON.parse(first.stdout).recent_workspaces).toEqual([]);
    expect(JSON.parse(second.stdout).recent_workspaces).toEqual([]);
  } finally {
    rmSync(runtimeRoot, { recursive: true, force: true });
    rmSync(firstWorkspace, { recursive: true, force: true });
    rmSync(secondWorkspace, { recursive: true, force: true });
  }
});

test("alfredo restores the selected workstation controller from runtime launch context", () => {
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
    const second = spawnSync(process.execPath, [alfredoBinPath()], {
      cwd: secondWorkspace,
      encoding: "utf8",
      env,
    });

    expect(first.status).toBe(0);
    expect(second.status).toBe(0);
    expect(JSON.parse(second.stdout)).toMatchObject({
      selected_agent: "qwen3.6-27b",
      selected_model: "qwen3.6:27b",
      starting_location: secondWorkspace,
      runtime_root: runtimeRoot,
      recent_workspaces: [],
    });
    expect(JSON.parse(readFileSync(resolve(runtimeRoot, "launch-context.json"), "utf8"))).toMatchObject({
      schema_version: 1,
      selected_agent: "qwen3.6-27b",
      selected_model: "qwen3.6:27b",
      starting_location: secondWorkspace,
      runtime_root: runtimeRoot,
    });
  } finally {
    rmSync(runtimeRoot, { recursive: true, force: true });
    rmSync(firstWorkspace, { recursive: true, force: true });
    rmSync(secondWorkspace, { recursive: true, force: true });
  }
});

test("deprecated Albert npm alias preserves the default Alfredo workstation intent", () => {
  const result = spawnSync(process.execPath, [albertBinPath(), "--agent", "qwen3.6-27b"], {
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

test("alfredo explicit agent ignores malformed persisted launch context", () => {
  const runtimeRoot = mkdtempSync(resolve(tmpdir(), "alfredo-runtime-"));
  const workspace = mkdtempSync(resolve(tmpdir(), "alfredo-workspace-"));

  try {
    writeFileSync(resolve(runtimeRoot, "launch-context.json"), "{not-json", "utf8");
    const result = spawnSync(process.execPath, [alfredoBinPath(), "--agent", "qwen3.6-27b"], {
      cwd: workspace,
      encoding: "utf8",
      env: {
        ...process.env,
        ALFREDO_DESKTOP_DRY_RUN: "1",
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
      },
    });

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    expect(JSON.parse(result.stdout)).toMatchObject({
      selected_agent: "qwen3.6-27b",
      selected_model: "qwen3.6:27b",
      starting_location: workspace,
    });
  } finally {
    rmSync(runtimeRoot, { recursive: true, force: true });
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("desktop shell presents Alfredo as the app title", () => {
  const tauriConfig = JSON.parse(
    readFileSync(resolve(projectRoot, "src-tauri", "tauri.conf.json"), "utf8"),
  );

  expect(tauriConfig.productName).toBe("Alfredo");
  expect(tauriConfig.app.windows[0].title).toBe("Alfredo");
});
