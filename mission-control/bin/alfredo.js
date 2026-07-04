#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import {
  accessSync,
  constants,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");
const agentConfigPath = process.env.ALFREDO_AGENT_CONFIG
  ? resolve(process.env.ALFREDO_AGENT_CONFIG)
  : resolve(repositoryRoot, ".albert", "agents.json");

function runtimeRoot() {
  return process.env.ALFREDO_RUNTIME_ROOT
    ? resolve(process.env.ALFREDO_RUNTIME_ROOT)
    : resolve(homedir(), ".alfredo", "runtime");
}

function requireKnownAgent(agentIdOrModel) {
  if (!agentIdOrModel) return null;
  if (!existsSync(agentConfigPath)) {
    throw new Error(`Agent registry not found: ${agentConfigPath}`);
  }
  const registry = JSON.parse(readFileSync(agentConfigPath, "utf8"));
  const agents = Array.isArray(registry.agents) ? registry.agents : [];
  const agent = agents.find(
    (candidate) =>
      candidate.id === agentIdOrModel || candidate.model === agentIdOrModel,
  );
  if (!agent) {
    throw new Error(
      `Unknown Alfredo agent or model: ${agentIdOrModel} (${agentConfigPath})`,
    );
  }
  return agent;
}

function accessStatus(path, mode) {
  try {
    accessSync(path, mode);
    return true;
  } catch {
    return false;
  }
}

function preflightCheck(name, status, detail, copyableAction) {
  return {
    name,
    status,
    detail,
    copyable_action: copyableAction,
  };
}

function buildPreflight(plan, selectedAgentConfig) {
  const packageJsonPath = resolve(projectRoot, "package.json");
  const packageJson = existsSync(packageJsonPath)
    ? JSON.parse(readFileSync(packageJsonPath, "utf8"))
    : {};
  const runtimePath = runtimeRoot();
  const shouldRunExternalChecks = process.env.ALFREDO_DESKTOP_DRY_RUN !== "1";
  const checks = [
    preflightCheck(
      "product_install",
      existsSync(packageJsonPath) ? "pass" : "fail",
      existsSync(packageJsonPath)
        ? `Found package metadata at ${packageJsonPath}.`
        : `Missing package metadata at ${packageJsonPath}.`,
      `test -f "${packageJsonPath}"`,
    ),
    preflightCheck(
      "node_runtime",
      existsSync(process.execPath) ? "pass" : "fail",
      `Node runtime: ${process.execPath}`,
      `node --version`,
    ),
    preflightCheck(
      "desktop_shell",
      packageJson.scripts?.desktop ? "pass" : "fail",
      packageJson.scripts?.desktop
        ? `Desktop shell command: npm run desktop.`
        : "Missing package script: desktop.",
      `cd "${projectRoot}" && npm run desktop`,
    ),
    preflightCheck(
      "backend_process",
      existsSync(resolve(repositoryRoot, "albert_mvp")) ? "pass" : "fail",
      existsSync(resolve(repositoryRoot, "albert_mvp"))
        ? `Backend module found at ${resolve(repositoryRoot, "albert_mvp")}.`
        : `Backend module missing at ${resolve(repositoryRoot, "albert_mvp")}.`,
      `cd "${repositoryRoot}" && python3 -m albert_mvp --help`,
    ),
    preflightCheck(
      "workspace_access",
      existsSync(plan.selected_workspace) &&
        accessStatus(plan.selected_workspace, constants.R_OK | constants.W_OK)
        ? "pass"
        : "fail",
      `Selected workspace: ${plan.selected_workspace}`,
      `cd "${plan.selected_workspace}"`,
    ),
    preflightCheck(
      "writable_runtime",
      existsSync(runtimePath) && accessStatus(runtimePath, constants.R_OK | constants.W_OK)
        ? "pass"
        : "fail",
      `Runtime root: ${runtimePath}`,
      `mkdir -p "${runtimePath}" && test -w "${runtimePath}"`,
    ),
  ];

  if (selectedAgentConfig?.runner === "ollama") {
    if (!shouldRunExternalChecks) {
      checks.push(
        preflightCheck(
          "ollama",
          "not_run",
          "Dry-run skipped external Ollama availability check.",
          "ollama list",
        ),
        preflightCheck(
          "required_model",
          "not_run",
          `Dry-run skipped model availability check for ${selectedAgentConfig.model}.`,
          `ollama pull ${selectedAgentConfig.model}`,
        ),
      );
      return checks;
    }

    const ollama = spawnSync("ollama", ["list"], {
      encoding: "utf8",
      timeout: 5000,
    });
    const ollamaAvailable = ollama.status === 0;
    const modelAvailable =
      ollamaAvailable && ollama.stdout.includes(selectedAgentConfig.model);
    checks.push(
      preflightCheck(
        "ollama",
        ollamaAvailable ? "pass" : "fail",
        ollamaAvailable ? "Ollama responded to ollama list." : "Ollama did not respond.",
        "ollama list",
      ),
      preflightCheck(
        "required_model",
        modelAvailable ? "pass" : "fail",
        modelAvailable
          ? `Required model is available: ${selectedAgentConfig.model}.`
          : `Required model is unavailable: ${selectedAgentConfig.model}.`,
        `ollama pull ${selectedAgentConfig.model}`,
      ),
    );
  } else {
    checks.push(
      preflightCheck(
        "ollama",
        "not_applicable",
        "Selected agent does not use Ollama.",
        "alfredo agents",
      ),
      preflightCheck(
        "required_model",
        "not_applicable",
        "Selected agent does not require an Ollama model.",
        "alfredo agents",
      ),
    );
  }

  return checks;
}

function recordRecentWorkspace(selectedWorkspace) {
  if (process.env.ALFREDO_DESKTOP_DRY_RUN === "1" && !process.env.ALFREDO_RUNTIME_ROOT) {
    return [];
  }
  const runtimePath = runtimeRoot();
  mkdirSync(runtimePath, { recursive: true });
  const recentPath = resolve(runtimePath, "recent-workspaces.json");
  const existing = existsSync(recentPath)
    ? JSON.parse(readFileSync(recentPath, "utf8"))
    : [];
  const recent = [
    selectedWorkspace,
    ...existing.filter((workspace) => workspace !== selectedWorkspace),
  ].slice(0, 10);
  writeFileSync(recentPath, `${JSON.stringify(recent, null, 2)}\n`, "utf8");
  return recent;
}

function parseWorkstationLaunch(argv) {
  const args = [...argv];
  if (args[0] === "workstation") {
    args.shift();
  }
  let selectedAgent = "";
  while (args.length > 0) {
    const arg = args.shift();
    if (arg === "--agent") {
      selectedAgent = args.shift() ?? "";
      if (!selectedAgent) throw new Error("--agent requires an agent id");
      continue;
    }
    throw new Error(`Unsupported workstation option: ${arg}`);
  }
  const selectedAgentConfig = requireKnownAgent(selectedAgent);
  const plan = {
    product: "Alfredo",
    launch: "workstation",
    selected_agent: selectedAgent,
    selected_model: selectedAgentConfig?.model ?? "",
    project_root: projectRoot,
    backend_root: repositoryRoot,
    selected_workspace: process.cwd(),
  };
  const recentWorkspaces = recordRecentWorkspace(plan.selected_workspace);
  return {
    ...plan,
    recent_workspaces: recentWorkspaces,
    preflight: buildPreflight(plan, selectedAgentConfig),
  };
}

function launchDesktop(plan) {
  const failures = plan.preflight.filter((check) => check.status === "fail");
  if (failures.length > 0) {
    process.stderr.write("Alfredo startup preflight failed:\n");
    for (const failure of failures) {
      process.stderr.write(
        `- ${failure.name}: ${failure.detail}\n  ${failure.copyable_action}\n`,
      );
    }
    process.exit(1);
  }
  const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
  const child = spawn(npmCommand, ["run", "desktop"], {
    cwd: projectRoot,
    env: {
      ...process.env,
      ALBERT_BACKEND_ROOT: plan.backend_root,
      ALFREDO_SELECTED_AGENT: plan.selected_agent,
      ALFREDO_SELECTED_WORKSPACE: plan.selected_workspace,
    },
    stdio: "inherit",
  });
  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? 1);
  });
}

try {
  const plan = parseWorkstationLaunch(process.argv.slice(2));
  if (process.env.ALFREDO_DESKTOP_DRY_RUN === "1") {
    process.stdout.write(`${JSON.stringify(plan)}\n`);
  } else {
    launchDesktop(plan);
  }
} catch (error) {
  process.stderr.write(`Error: ${error.message}\n`);
  process.exit(1);
}
