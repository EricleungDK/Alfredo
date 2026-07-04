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
import { delimiter, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");
const agentConfigPath = process.env.ALFREDO_AGENT_CONFIG
  ? resolve(process.env.ALFREDO_AGENT_CONFIG)
  : resolve(repositoryRoot, ".albert", "agents.json");
const PREFLIGHT_PASS = "pass";
const PREFLIGHT_FAIL = "fail";
const PREFLIGHT_NOT_RUN = "not_run";
const PREFLIGHT_NOT_APPLICABLE = "not_applicable";

function runtimeRoot() {
  return process.env.ALFREDO_RUNTIME_ROOT
    ? resolve(process.env.ALFREDO_RUNTIME_ROOT)
    : resolve(homedir(), ".alfredo", "runtime");
}

function agentRegistry() {
  if (!existsSync(agentConfigPath)) {
    throw new Error(`Agent registry not found: ${agentConfigPath}`);
  }
  const registry = JSON.parse(readFileSync(agentConfigPath, "utf8"));
  return Array.isArray(registry.agents) ? registry.agents : [];
}

function requireKnownAgent(agentIdOrModel, options = {}) {
  if (!agentIdOrModel) return null;
  const agents = agentRegistry();
  const agent = agents.find(
    (candidate) =>
      candidate.id === agentIdOrModel ||
      (options.allowModelAlias === true && candidate.model === agentIdOrModel),
  );
  if (!agent) {
    const configured = agents.map((candidate) => candidate.id).join(", ") || "none";
    const expected = options.allowModelAlias === true ? "agent or model" : "agent id";
    throw new Error(
      `Unknown Alfredo ${expected}: ${agentIdOrModel}. Run "alfredo agents" and use one of: ${configured}. Registry: ${agentConfigPath}`,
    );
  }
  return agent;
}

function loadAgentRegistry() {
  return agentRegistry();
}

function dryRunMode() {
  return process.env.ALFREDO_DESKTOP_DRY_RUN ?? "";
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
  const shouldRunExternalChecks = dryRunMode() === "";
  const pythonCommand = process.env.ALBERT_PYTHON ?? (process.platform === "win32" ? "python" : "python3");
  const npmVersion = shouldRunExternalChecks
    ? spawnSync("npm", ["--version"], { encoding: "utf8", timeout: 5000 })
    : null;
  const backendHelp = shouldRunExternalChecks
    ? spawnSync(pythonCommand, ["-m", "albert_mvp", "--help"], {
        cwd: repositoryRoot,
        encoding: "utf8",
        timeout: 5000,
      })
    : null;
  const shouldCheckRuntime = dryRunMode() !== "1" || Boolean(process.env.ALFREDO_RUNTIME_ROOT);
  let runtimeStatus = PREFLIGHT_NOT_RUN;
  let runtimeDetail = "Dry-run skipped default runtime writability check.";
  if (shouldCheckRuntime) {
    runtimeStatus = PREFLIGHT_FAIL;
    runtimeDetail = `Runtime root: ${runtimePath}`;
    try {
      mkdirSync(runtimePath, { recursive: true });
      if (accessStatus(runtimePath, constants.R_OK | constants.W_OK)) {
        runtimeStatus = PREFLIGHT_PASS;
      } else {
        runtimeDetail = `Runtime root is not writable: ${runtimePath}`;
      }
    } catch (error) {
      runtimeDetail = `Runtime root is not writable: ${runtimePath} (${error.message})`;
    }
  }
  const checks = [
    preflightCheck(
      "product_install",
      existsSync(packageJsonPath) ? PREFLIGHT_PASS : PREFLIGHT_FAIL,
      existsSync(packageJsonPath)
        ? `Found package metadata at ${packageJsonPath}.`
        : `Missing package metadata at ${packageJsonPath}.`,
      `test -f "${packageJsonPath}"`,
    ),
    preflightCheck(
      "node_runtime",
      existsSync(process.execPath) ? PREFLIGHT_PASS : PREFLIGHT_FAIL,
      `Node runtime: ${process.execPath}`,
      `node --version`,
    ),
    preflightCheck(
      "npm_runtime",
      shouldRunExternalChecks
        ? npmVersion?.status === 0
          ? PREFLIGHT_PASS
          : PREFLIGHT_FAIL
        : PREFLIGHT_NOT_RUN,
      shouldRunExternalChecks
        ? npmVersion?.status === 0
          ? `npm runtime: ${npmVersion.stdout.trim()}.`
          : `npm runtime unavailable: ${(npmVersion?.stderr || npmVersion?.error?.message || "npm --version failed").toString().trim()}.`
        : "Dry-run skipped npm runtime availability check.",
      "npm --version",
    ),
    preflightCheck(
      "desktop_shell",
      packageJson.scripts?.desktop ? PREFLIGHT_PASS : PREFLIGHT_FAIL,
      packageJson.scripts?.desktop
        ? `Desktop shell command: npm run desktop.`
        : "Missing package script: desktop.",
      `cd "${projectRoot}" && npm run desktop`,
    ),
    preflightCheck(
      "backend_process",
      shouldRunExternalChecks
        ? backendHelp?.status === 0
          ? PREFLIGHT_PASS
          : PREFLIGHT_FAIL
        : existsSync(resolve(repositoryRoot, "albert_mvp"))
          ? PREFLIGHT_NOT_RUN
          : PREFLIGHT_FAIL,
      shouldRunExternalChecks
        ? backendHelp?.status === 0
          ? `Backend process responds to ${pythonCommand} -m albert_mvp --help.`
          : `Backend process did not start: ${(backendHelp?.stderr || backendHelp?.error?.message || `${pythonCommand} -m albert_mvp --help failed`).toString().trim()}.`
        : existsSync(resolve(repositoryRoot, "albert_mvp"))
          ? "Dry-run skipped backend process startup check."
          : `Backend module missing at ${resolve(repositoryRoot, "albert_mvp")}.`,
      `cd "${repositoryRoot}" && ${pythonCommand} -m albert_mvp --help`,
    ),
    preflightCheck(
      "workspace_access",
      existsSync(plan.selected_workspace) &&
        accessStatus(plan.selected_workspace, constants.R_OK | constants.W_OK)
        ? PREFLIGHT_PASS
        : PREFLIGHT_FAIL,
      `Selected workspace: ${plan.selected_workspace}`,
      `cd "${plan.selected_workspace}"`,
    ),
    preflightCheck(
      "writable_runtime",
      runtimeStatus,
      runtimeDetail,
      `mkdir -p "${runtimePath}" && test -w "${runtimePath}"`,
    ),
  ];

  if (selectedAgentConfig?.runner === "ollama") {
    if (!shouldRunExternalChecks) {
      checks.push(
        preflightCheck(
          "ollama",
          PREFLIGHT_NOT_RUN,
          "Dry-run skipped external Ollama availability check.",
          "ollama list",
        ),
        preflightCheck(
          "required_model",
          PREFLIGHT_NOT_RUN,
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
        ollamaAvailable ? PREFLIGHT_PASS : PREFLIGHT_FAIL,
        ollamaAvailable ? "Ollama responded to ollama list." : "Ollama did not respond.",
        "ollama list",
      ),
      preflightCheck(
        "required_model",
        modelAvailable ? PREFLIGHT_PASS : PREFLIGHT_FAIL,
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
        PREFLIGHT_NOT_APPLICABLE,
        "Selected agent does not use Ollama.",
        "alfredo agents",
      ),
      preflightCheck(
        "required_model",
        PREFLIGHT_NOT_APPLICABLE,
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
  const selectedAgentConfig = requireKnownAgent(selectedAgent, { allowModelAlias: true });
  const plan = {
    product: "Alfredo",
    launch: "workstation",
    selected_agent: selectedAgent,
    selected_model: selectedAgentConfig?.model ?? "",
    runtime_root: runtimeRoot(),
    project_root: projectRoot,
    backend_root: repositoryRoot,
    selected_workspace: process.cwd(),
  };
  const preflight = buildPreflight(plan, selectedAgentConfig);
  const runtimeReady = !preflight.some(
    (check) => check.name === "writable_runtime" && check.status === PREFLIGHT_FAIL,
  );
  const recentWorkspaces = runtimeReady ? recordRecentWorkspace(plan.selected_workspace) : [];
  return {
    ...plan,
    recent_workspaces: recentWorkspaces,
    preflight,
  };
}

function parseAgentOption(args) {
  let selectedAgent = "";
  const remaining = [];
  while (args.length > 0) {
    const arg = args.shift();
    if (arg === "--agent") {
      selectedAgent = args.shift() ?? "";
      if (!selectedAgent) throw new Error("--agent requires an agent id");
      continue;
    }
    remaining.push(arg);
  }
  return { selectedAgent, remaining };
}

function parseHeadlessRun(argv) {
  const { selectedAgent, remaining } = parseAgentOption([...argv]);
  if (!selectedAgent) throw new Error("alfredo run requires --agent <agent-id>");
  if (remaining.length !== 1 || !remaining[0]?.trim()) {
    throw new Error('alfredo run requires exactly one prompt argument, for example: alfredo run --agent qwen3.6-27b "Fix the tests"');
  }
  const selectedAgentConfig = requireKnownAgent(selectedAgent);
  return {
    product: "Alfredo",
    launch: "headless-run",
    selected_agent: selectedAgent,
    selected_model: selectedAgentConfig.model,
    prompt: remaining[0],
    selected_workspace: process.cwd(),
    runtime_root: runtimeRoot(),
  };
}

function parseHeadlessReview(argv) {
  const { selectedAgent, remaining } = parseAgentOption([...argv]);
  if (!selectedAgent) throw new Error("alfredo review requires --agent <agent-id>");
  if (remaining.length > 1) {
    throw new Error("alfredo review accepts at most one optional session id");
  }
  const selectedAgentConfig = requireKnownAgent(selectedAgent);
  return {
    product: "Alfredo",
    launch: "headless-review",
    selected_agent: selectedAgent,
    selected_model: selectedAgentConfig.model,
    session_id: remaining[0] ?? "",
    selected_workspace: process.cwd(),
    runtime_root: runtimeRoot(),
  };
}

function pythonCommand() {
  return process.env.ALBERT_PYTHON ?? (process.platform === "win32" ? "python" : "python3");
}

function trackerArgs(selectedWorkspace) {
  const agentIssuesTracker = resolve(selectedWorkspace, ".agent", "issues");
  if (existsSync(resolve(agentIssuesTracker, "PRD.md"))) {
    return {
      trackerDir: agentIssuesTracker,
      issuesDir: agentIssuesTracker,
    };
  }
  const trackerDir = resolve(selectedWorkspace, ".agent");
  return {
    trackerDir,
    issuesDir: resolve(trackerDir, "issues"),
  };
}

function headlessContextArgs(selectedWorkspace) {
  const { trackerDir, issuesDir } = trackerArgs(selectedWorkspace);
  return [
    "--target-repo",
    selectedWorkspace,
    "--tracker-dir",
    trackerDir,
    "--issues-dir",
    issuesDir,
    "--runtime-root",
    runtimeRoot(),
    "--mission-id",
    "alfredo-headless",
    "--agent-config",
    agentConfigPath,
  ];
}

function runHeadlessBackend(plan) {
  const backendArgs = [
    plan.launch === "headless-run" ? "headless-run" : "headless-review",
    ...headlessContextArgs(plan.selected_workspace),
    "--agent",
    plan.selected_agent,
    "--allowed-path",
    plan.selected_workspace,
  ];
  if (plan.launch === "headless-run") {
    backendArgs.push(plan.prompt);
  } else if (plan.session_id) {
    backendArgs.push(plan.session_id);
  }
  return spawnSync(pythonCommand(), ["-m", "albert_mvp", ...backendArgs], {
    cwd: repositoryRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      PYTHONPATH: process.env.PYTHONPATH
        ? `${repositoryRoot}${delimiter}${process.env.PYTHONPATH}`
        : repositoryRoot,
    },
  });
}

function runAgentsBackend() {
  return spawnSync(pythonCommand(), ["-m", "albert_mvp", "agents", ...headlessContextArgs(process.cwd())], {
    cwd: repositoryRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      PYTHONPATH: process.env.PYTHONPATH
        ? `${repositoryRoot}${delimiter}${process.env.PYTHONPATH}`
        : repositoryRoot,
    },
  });
}

function launchDryRunPlan(plan, npmCommand) {
  return {
    command: [npmCommand, "run", "desktop"],
    cwd: projectRoot,
    env: {
      ALBERT_BACKEND_ROOT: plan.backend_root,
      ALFREDO_SELECTED_AGENT: plan.selected_agent,
      ALFREDO_SELECTED_MODEL: plan.selected_model,
      ALFREDO_SELECTED_WORKSPACE: plan.selected_workspace,
      ALFREDO_RUNTIME_ROOT: plan.runtime_root,
    },
  };
}

function launchDesktop(plan) {
  const failures = plan.preflight.filter((check) => check.status === PREFLIGHT_FAIL);
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
  if (dryRunMode() === "launch") {
    process.stdout.write(`${JSON.stringify(launchDryRunPlan(plan, npmCommand))}\n`);
    return;
  }
  const child = spawn(npmCommand, ["run", "desktop"], {
    cwd: projectRoot,
    env: {
      ...process.env,
      ALBERT_BACKEND_ROOT: plan.backend_root,
      ALFREDO_SELECTED_AGENT: plan.selected_agent,
      ALFREDO_SELECTED_MODEL: plan.selected_model,
      ALFREDO_SELECTED_WORKSPACE: plan.selected_workspace,
      ALFREDO_RUNTIME_ROOT: plan.runtime_root,
    },
    stdio: "inherit",
  });
  child.on("error", (error) => {
    process.stderr.write("Alfredo startup preflight failed:\n");
    process.stderr.write(
      `- desktop_shell: Unable to launch npm desktop process: ${error.message}\n  cd "${projectRoot}" && npm run desktop\n`,
    );
    process.exit(1);
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
  const argv = process.argv.slice(2);
  const command = argv[0] ?? "";
  if (command === "agents") {
    if (argv.length !== 1) throw new Error("alfredo agents does not accept additional arguments");
    const result = runAgentsBackend();
    if (result.error) throw result.error;
    process.stdout.write(result.stdout);
    process.stderr.write(result.stderr);
    process.exit(result.status ?? 1);
  }
  if (command === "run") {
    const plan = parseHeadlessRun(argv.slice(1));
    if (dryRunMode()) {
      process.stdout.write(`${JSON.stringify(plan)}\n`);
    } else {
      const result = runHeadlessBackend(plan);
      if (result.error) throw result.error;
      process.stdout.write(result.stdout);
      process.stderr.write(result.stderr);
      process.exit(result.status ?? 1);
    }
    process.exit(0);
  }
  if (command === "review") {
    const plan = parseHeadlessReview(argv.slice(1));
    if (dryRunMode()) {
      process.stdout.write(`${JSON.stringify(plan)}\n`);
    } else {
      const result = runHeadlessBackend(plan);
      if (result.error) throw result.error;
      process.stdout.write(result.stdout);
      process.stderr.write(result.stderr);
      process.exit(result.status ?? 1);
    }
    process.exit(0);
  }

  const plan = parseWorkstationLaunch(argv);
  if (dryRunMode() === "1") {
    process.stdout.write(`${JSON.stringify(plan)}\n`);
  } else {
    launchDesktop(plan);
  }
} catch (error) {
  process.stderr.write(`Error: ${error.message}\n`);
  process.exit(1);
}
