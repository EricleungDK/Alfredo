#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import {
  accessSync,
  constants,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import {
  basename,
  delimiter,
  dirname,
  isAbsolute,
  relative,
  resolve,
  sep,
} from "node:path";
import { fileURLToPath } from "node:url";

import { DesktopAdapterError, resolveDesktopAdapter } from "./desktop-adapter.js";
import {
  createPerformanceRecorder,
  performanceEnvironment,
} from "../scripts/performance-recorder.js";

const launcherMeasurementMetadata = performanceEnvironment();
const launcherPerformance =
  launcherMeasurementMetadata?.workflow === "startup"
    ? createPerformanceRecorder({
        ...launcherMeasurementMetadata,
        source: "launcher",
        clock_id: `launcher:${process.pid}`,
      })
    : null;
launcherPerformance?.mark("S1", "start");
let launcherMeasurementFinished = false;

function finishLauncherMeasurement(desktopKind) {
  if (!launcherPerformance || launcherMeasurementFinished) return;
  launcherMeasurementFinished = true;
  launcherPerformance.mark("S1", "end", { desktop_kind: desktopKind });
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");
const bundledBackendRoot = resolve(projectRoot, "bundled-backend");
const repositoryLayoutAvailable =
  projectRoot === resolve(repositoryRoot, "mission-control") &&
  existsSync(resolve(repositoryRoot, "albert_mvp")) &&
  existsSync(resolve(repositoryRoot, ".albert", "agents.json"));
const bundledBackendAvailable =
  existsSync(resolve(bundledBackendRoot, "albert_mvp")) &&
  existsSync(resolve(bundledBackendRoot, ".albert", "agents.json"));
const backendRoot = process.env.ALBERT_BACKEND_ROOT
  ? resolve(process.env.ALBERT_BACKEND_ROOT)
  : repositoryLayoutAvailable
    ? repositoryRoot
    : bundledBackendAvailable
      ? bundledBackendRoot
      : repositoryRoot;
const agentConfigPath = process.env.ALFREDO_AGENT_CONFIG
  ? resolve(process.env.ALFREDO_AGENT_CONFIG)
  : resolve(backendRoot, ".albert", "agents.json");
const PREFLIGHT_PASS = "pass";
const PREFLIGHT_FAIL = "fail";
const PREFLIGHT_WARNING = "warning";
const PREFLIGHT_NOT_RUN = "not_run";
const PREFLIGHT_NOT_APPLICABLE = "not_applicable";
const LAUNCH_CONTEXT_SCHEMA_VERSION = 1;

function runtimeRoot() {
  return process.env.ALFREDO_RUNTIME_ROOT
    ? resolve(process.env.ALFREDO_RUNTIME_ROOT)
    : resolve(homedir(), ".alfredo", "runtime");
}

function pathIsInsideOrEqual(candidate, root) {
  const relativePath = relative(resolve(root), resolve(candidate));
  return (
    relativePath === "" ||
    (relativePath !== ".." &&
      !relativePath.startsWith(`..${sep}`) &&
      !isAbsolute(relativePath))
  );
}

function protectedInstallRoots() {
  return [
    backendRoot,
    projectRoot,
    runtimeRoot(),
    process.env.ALFREDO_INSTALL_ROOT?.trim(),
  ]
    .filter(Boolean)
    .map((path) => resolve(path));
}

function startingLocationIsProtected(path) {
  return protectedInstallRoots().some((root) => pathIsInsideOrEqual(path, root));
}

function safeStartingLocation() {
  const configured = process.env.ALFREDO_STARTING_LOCATION?.trim()
    ? resolve(process.env.ALFREDO_STARTING_LOCATION)
    : resolve(process.cwd());
  if (!startingLocationIsProtected(configured)) return configured;

  let candidate = configured;
  while (startingLocationIsProtected(candidate)) {
    const parent = resolve(candidate, "..");
    if (parent === candidate) break;
    candidate = parent;
  }
  if (existsSync(candidate) && !startingLocationIsProtected(candidate)) {
    return candidate;
  }

  return [dirname(backendRoot), dirname(runtimeRoot()), tmpdir()]
    .map((path) => resolve(path))
    .find((path) => existsSync(path) && !startingLocationIsProtected(path))
    ?? configured;
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

function desktopLaunchTarget(selectedWorkspace) {
  if (repositoryLayoutAvailable) {
    const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
    return {
      kind: "development",
      command: [npmCommand, "run", "desktop"],
      cwd: projectRoot,
      detail: "Repository development shell: npm run desktop.",
      copyable_action: `cd "${projectRoot}" && npm run desktop`,
      environment: {},
    };
  }
  try {
    const adapter = resolveDesktopAdapter(projectRoot);
    return {
      kind: adapter.kind,
      command: [adapter.executable],
      cwd: selectedWorkspace,
      detail: `Installed desktop package: ${adapter.packageName}@${adapter.version}.`,
      copyable_action: `APPIMAGE_EXTRACT_AND_RUN=1 "${adapter.executable}" --version`,
      environment: adapter.environment,
      version: adapter.version,
    };
  } catch (error) {
    if (!(error instanceof DesktopAdapterError)) throw error;
    return {
      kind: "unavailable",
      command: [],
      cwd: selectedWorkspace,
      detail: error.message,
      copyable_action: error.copyableAction,
      environment: {},
    };
  }
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

function buildPreflight(plan, selectedAgentConfig, desktopTarget) {
  const packageJsonPath = resolve(projectRoot, "package.json");
  const packageJson = existsSync(packageJsonPath)
    ? JSON.parse(readFileSync(packageJsonPath, "utf8"))
    : {};
  const runtimePath = runtimeRoot();
  const shouldRunExternalChecks = dryRunMode() === "";
  const requiresNpmRuntime = desktopTarget.kind === "development";
  const pythonCommand = process.env.ALBERT_PYTHON ?? (process.platform === "win32" ? "python" : "python3");
  const tauriCommand = resolve(
    projectRoot,
    "node_modules",
    ".bin",
    process.platform === "win32" ? "tauri.cmd" : "tauri",
  );
  const cargoCommand = process.env.CARGO ?? "cargo";
  const npmVersion = shouldRunExternalChecks && requiresNpmRuntime
    ? spawnSync("npm", ["--version"], { encoding: "utf8", timeout: 5000 })
    : null;
  const tauriReady =
    !requiresNpmRuntime ||
    !shouldRunExternalChecks ||
    accessStatus(tauriCommand, constants.R_OK | constants.X_OK);
  const cargoVersion = shouldRunExternalChecks && requiresNpmRuntime && tauriReady
    ? spawnSync(cargoCommand, ["--version"], { encoding: "utf8", timeout: 5000 })
    : null;
  const developmentDesktopReady =
    !requiresNpmRuntime ||
    !shouldRunExternalChecks ||
    (tauriReady && cargoVersion?.status === 0);
  const backendHelp = shouldRunExternalChecks
    ? spawnSync(pythonCommand, ["-m", "albert_mvp", "--help"], {
        cwd: backendRoot,
        encoding: "utf8",
        timeout: 5000,
      })
    : null;
  const sandboxProbe = shouldRunExternalChecks && process.platform === "linux"
    ? spawnSync("bwrap", ["--version"], { encoding: "utf8", timeout: 5000 })
    : null;
  const desktopProbe =
    shouldRunExternalChecks && desktopTarget.kind === "native"
      ? spawnSync(desktopTarget.command[0], ["--version"], {
          cwd: desktopTarget.cwd,
          encoding: "utf8",
          timeout: 15_000,
          env: {
            ...process.env,
            ...desktopTarget.environment,
          },
        })
      : null;
  const desktopProbeExpected = `Alfredo Desktop ${desktopTarget.version ?? ""}`;
  const desktopReady =
    desktopTarget.kind !== "unavailable" &&
    developmentDesktopReady &&
    (desktopTarget.kind !== "native" ||
      !shouldRunExternalChecks ||
      (desktopProbe?.status === 0 && desktopProbe.stdout.trim() === desktopProbeExpected));
  const desktopDetail =
    desktopTarget.kind === "development" && shouldRunExternalChecks
      ? !tauriReady
        ? `${desktopTarget.detail} Local Tauri CLI is unavailable; install the lockfile dependencies.`
        : cargoVersion?.status !== 0
          ? `${desktopTarget.detail} Cargo is unavailable: ${(
              cargoVersion?.stderr ||
              cargoVersion?.error?.message ||
              `${cargoCommand} --version failed`
            )
              .toString()
              .trim()}.`
          : `${desktopTarget.detail} Tauri CLI and Cargo are available.`
      : desktopTarget.kind === "native" && shouldRunExternalChecks
      ? desktopReady
        ? `${desktopTarget.detail} Native probe: ${desktopProbeExpected}.`
        : `${desktopTarget.detail} Native probe failed: ${(
            desktopProbe?.stderr ||
            desktopProbe?.stdout ||
            desktopProbe?.error?.message ||
            "no version response"
          )
            .toString()
            .trim()}.`
      : desktopTarget.detail;
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
      !requiresNpmRuntime
        ? PREFLIGHT_NOT_APPLICABLE
        : shouldRunExternalChecks
        ? npmVersion?.status === 0
          ? PREFLIGHT_PASS
          : PREFLIGHT_FAIL
        : PREFLIGHT_NOT_RUN,
      !requiresNpmRuntime
        ? "The installed native desktop does not invoke npm at launch."
        : shouldRunExternalChecks
        ? npmVersion?.status === 0
          ? `npm runtime: ${npmVersion.stdout.trim()}.`
          : `npm runtime unavailable: ${(npmVersion?.stderr || npmVersion?.error?.message || "npm --version failed").toString().trim()}.`
        : "Dry-run skipped npm runtime availability check.",
      requiresNpmRuntime ? "npm --version" : "alfredo --version",
    ),
    preflightCheck(
      "desktop_shell",
      desktopReady ? PREFLIGHT_PASS : PREFLIGHT_FAIL,
      desktopDetail,
      desktopTarget.kind === "development" && shouldRunExternalChecks && !tauriReady
        ? `cd "${projectRoot}" && npm ci`
        : desktopTarget.kind === "development" && shouldRunExternalChecks && cargoVersion?.status !== 0
          ? `${cargoCommand} --version`
          : desktopTarget.copyable_action,
    ),
    preflightCheck(
      "backend_process",
      shouldRunExternalChecks
        ? backendHelp?.status === 0
          ? PREFLIGHT_PASS
          : PREFLIGHT_FAIL
        : existsSync(resolve(backendRoot, "albert_mvp"))
          ? PREFLIGHT_NOT_RUN
          : PREFLIGHT_FAIL,
      shouldRunExternalChecks
        ? backendHelp?.status === 0
          ? `Backend process responds to ${pythonCommand} -m albert_mvp --help.`
          : `Backend process did not start: ${(backendHelp?.stderr || backendHelp?.error?.message || `${pythonCommand} -m albert_mvp --help failed`).toString().trim()}.`
        : existsSync(resolve(backendRoot, "albert_mvp"))
          ? "Dry-run skipped backend process startup check."
          : `Backend module missing at ${resolve(backendRoot, "albert_mvp")}.`,
      `cd "${backendRoot}" && ${pythonCommand} -m albert_mvp --help`,
    ),
    preflightCheck(
      "sandbox_runtime",
      process.platform !== "linux"
        ? PREFLIGHT_NOT_APPLICABLE
        : shouldRunExternalChecks
          ? sandboxProbe?.status === 0
            ? PREFLIGHT_PASS
            : PREFLIGHT_WARNING
          : PREFLIGHT_NOT_RUN,
      process.platform !== "linux"
        ? "Bubblewrap sandboxing applies to the supported Ubuntu workstation package."
        : shouldRunExternalChecks
          ? sandboxProbe?.status === 0
            ? `Bubblewrap sandbox runtime: ${sandboxProbe.stdout.trim()}.`
            : "Bubblewrap is unavailable; Alfredo can open, but governed coding and shell work requires it."
          : "Dry-run skipped Bubblewrap availability check.",
      "bwrap --version",
    ),
    preflightCheck(
      "starting_location_access",
      existsSync(plan.starting_location) &&
        accessStatus(plan.starting_location, constants.R_OK | constants.W_OK)
        ? PREFLIGHT_PASS
        : PREFLIGHT_FAIL,
      `Starting location: ${plan.starting_location}`,
      `cd "${plan.starting_location}"`,
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
        ollamaAvailable ? PREFLIGHT_PASS : PREFLIGHT_WARNING,
        ollamaAvailable
          ? "Ollama responded to ollama list."
          : "Ollama is not available yet; Alfredo can open, but local model work stays unavailable until it is installed.",
        "ollama list",
      ),
      preflightCheck(
        "required_model",
        modelAvailable ? PREFLIGHT_PASS : PREFLIGHT_WARNING,
        modelAvailable
          ? `Required model is available: ${selectedAgentConfig.model}.`
          : `Required model is unavailable: ${selectedAgentConfig.model}. Alfredo can open before the model is pulled.`,
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

function recentWorkspaces() {
  if (!shouldPersistRuntimeState()) return [];
  const recentPath = resolve(runtimeRoot(), "recent-workspaces.json");
  if (!existsSync(recentPath)) return [];
  try {
    const recent = JSON.parse(readFileSync(recentPath, "utf8"));
    return Array.isArray(recent)
      ? recent.filter((workspace) => typeof workspace === "string" && workspace.trim()).slice(0, 10)
      : [];
  } catch {
    return [];
  }
}

function filesystemIdentity(path) {
  const normalized = String(path).replaceAll("\\", "/");
  return process.platform === "win32" || /^\/mnt\/[a-z]\//i.test(normalized)
    ? normalized.toLowerCase()
    : normalized;
}

function shouldPersistRuntimeState() {
  return !(process.env.ALFREDO_DESKTOP_DRY_RUN === "1" && !process.env.ALFREDO_RUNTIME_ROOT);
}

function loadPersistedLaunchContext() {
  if (!shouldPersistRuntimeState()) return null;
  const launchContextPath = resolve(runtimeRoot(), "launch-context.json");
  if (!existsSync(launchContextPath)) return null;
  try {
    const persisted = JSON.parse(readFileSync(launchContextPath, "utf8"));
    if (persisted.schema_version !== LAUNCH_CONTEXT_SCHEMA_VERSION) return null;
    return persisted;
  } catch {
    return null;
  }
}

function recordLaunchContext(plan) {
  if (!shouldPersistRuntimeState()) return;
  const runtimePath = runtimeRoot();
  mkdirSync(runtimePath, { recursive: true });
  const launchContextPath = resolve(runtimePath, "launch-context.json");
  writeFileSync(
    launchContextPath,
    `${JSON.stringify(
      {
        schema_version: LAUNCH_CONTEXT_SCHEMA_VERSION,
        selected_agent: plan.selected_agent,
        selected_model: plan.selected_model,
        starting_location: plan.starting_location,
        runtime_root: plan.runtime_root,
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
}

const LOCAL_EXECUTION_PROVIDERS = new Set([
  "command",
  "fake",
  "local",
  "ollama",
  "test",
  "test-harness",
]);
const LOCAL_EXECUTION_RUNNERS = new Set(["command", "fake", "ollama"]);

function hasLocalExecutionBoundary(agent) {
  return LOCAL_EXECUTION_PROVIDERS.has(String(agent?.provider ?? "").trim().toLowerCase()) &&
    LOCAL_EXECUTION_RUNNERS.has(String(agent?.runner ?? "").trim().toLowerCase());
}

function isEligibleWorkstationController(agent) {
  if (
    !agent ||
    !hasLocalExecutionBoundary(agent) ||
    agent.delegate_only === true ||
    agent.requires_approval === true ||
    String(agent.availability ?? "available").trim().toLowerCase() !== "available" ||
    String(agent.model ?? "").toLowerCase().endsWith(":cloud")
  ) return false;
  const routing = String(agent.routing ?? "").toLowerCase();
  const role = String(agent.role ?? "").toLowerCase();
  return routing
    ? ["controller", "router", "frontier"].includes(routing)
    : role === "frontier";
}

function resolveWorkstationAgent(explicitSelectedAgent) {
  const agents = agentRegistry();
  const eligibleControllers = agents.filter(isEligibleWorkstationController);
  const defaultController = eligibleControllers.find(
    (agent) => String(agent.routing).toLowerCase() === "controller",
  ) ?? eligibleControllers.find(
    (agent) => String(agent.routing).toLowerCase() === "router",
  ) ?? eligibleControllers[0];
  const resolveCandidate = (candidate) => {
    const config = requireKnownAgent(candidate, { allowModelAlias: true });
    if (!isEligibleWorkstationController(config)) {
      const available = eligibleControllers.map((agent) => agent.id).join(", ") || "none";
      throw new Error(
        `Agent ${candidate} (${config.id}) is not an eligible Alfredo workstation controller. ` +
        `Choose an ungated controller/router Frontier Model: ${available}.`,
      );
    }
    return { selectedAgent: config.id, selectedAgentConfig: config };
  };

  if (explicitSelectedAgent) return resolveCandidate(explicitSelectedAgent);
  const persisted = loadPersistedLaunchContext();
  if (persisted?.selected_agent) {
    try {
      return resolveCandidate(persisted.selected_agent);
    } catch {
      // A stale, removed, aliased, or role-ineligible preference must not break startup.
    }
  }
  return defaultController
    ? { selectedAgent: defaultController.id, selectedAgentConfig: defaultController }
    : { selectedAgent: "", selectedAgentConfig: null };
}

function parseWorkstationLaunch(argv) {
  const args = [...argv];
  if (args[0] === "workstation") {
    args.shift();
  }
  let explicitSelectedAgent = "";
  while (args.length > 0) {
    const arg = args.shift();
    if (arg === "--agent") {
      explicitSelectedAgent = args.shift() ?? "";
      if (!explicitSelectedAgent) throw new Error("--agent requires an agent id");
      continue;
    }
    throw new Error(`Unsupported workstation option: ${arg}`);
  }
  const { selectedAgent, selectedAgentConfig } = resolveWorkstationAgent(explicitSelectedAgent);
  const startingLocation = safeStartingLocation();
  const plan = {
    product: "Alfredo",
    launch: "workstation",
    selected_agent: selectedAgent,
    selected_model: selectedAgentConfig?.model ?? "",
    runtime_root: runtimeRoot(),
    project_root: projectRoot,
    backend_root: backendRoot,
    starting_location: startingLocation,
    workspace_selection: {
      schema_version: 1,
      phase: "selection-required",
      starting_location: startingLocation,
      coding_workspace: null,
      active_mission: null,
    },
  };
  const desktopAdapter = desktopLaunchTarget(plan.starting_location);
  const preflight = buildPreflight(plan, selectedAgentConfig, desktopAdapter);
  const runtimeReady = !preflight.some(
    (check) => check.name === "writable_runtime" && check.status === PREFLIGHT_FAIL,
  );
  const launchPlan = {
    ...plan,
    desktop_adapter: desktopAdapter,
    recent_workspaces: runtimeReady ? recentWorkspaces() : [],
    preflight,
  };
  if (runtimeReady) recordLaunchContext(launchPlan);
  return launchPlan;
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
    throw new Error('alfredo run requires exactly one prompt argument, for example: alfredo run --agent gemma4-12b "Fix the tests"');
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

const AGENT_READY_TICKET_STATUSES = new Set(["approved", "ready", "ready-for-agent"]);
const READY_TICKET_STATUSES = new Set([...AGENT_READY_TICKET_STATUSES, "ready-for-human"]);
const TERMINAL_TICKET_STATUSES = new Set([
  "canceled",
  "cancelled",
  "closed",
  "complete",
  "completed",
  "done",
  "merged",
  "wont-fix",
  "wontfix",
]);

function trackerTicketStatuses(issuesDir) {
  if (!existsSync(issuesDir)) return [];
  try {
    return readdirSync(issuesDir, { withFileTypes: true })
      .filter((entry) => entry.isFile() && /^\d+[-_].+\.md$/i.test(entry.name))
      .map((entry) => {
        try {
          const issue = readFileSync(resolve(issuesDir, entry.name), "utf8");
          const ticketType = issue.match(/^\s*Type:\s*(.+?)\s*$/im)?.[1]?.trim().toLowerCase() ?? "";
          if (ticketType === "prd") return "";
          return issue.match(/^\s*Status:\s*(.+?)\s*$/im)?.[1]?.trim().toLowerCase() ?? "";
        } catch {
          return "";
        }
      })
      .filter(Boolean);
  } catch {
    return [];
  }
}

function trackerCandidate(trackerDir, missionId) {
  const issuesDir = existsSync(resolve(trackerDir, "issues"))
    ? resolve(trackerDir, "issues")
    : trackerDir;
  const statuses = trackerTicketStatuses(issuesDir);
  let modifiedAt = 0;
  try {
    modifiedAt = statSync(resolve(trackerDir, "PRD.md")).mtimeMs;
  } catch {
    // An unreadable timestamp should not make discovery fail. The normal
    // preflight will still report workspace access problems before launch.
  }
  return {
    trackerDir,
    issuesDir,
    missionId,
    hasAgentReadyTicket: statuses.some((status) => AGENT_READY_TICKET_STATUSES.has(status)),
    hasReadyTicket: statuses.some((status) => READY_TICKET_STATUSES.has(status)),
    hasActionableTicket: statuses.some((status) => !TERMINAL_TICKET_STATUSES.has(status)),
    modifiedAt,
  };
}

function compareTrackerCandidates(left, right) {
  const agentReady =
    Number(right.hasAgentReadyTicket) - Number(left.hasAgentReadyTicket);
  if (agentReady !== 0) return agentReady;
  const ready = Number(right.hasReadyTicket) - Number(left.hasReadyTicket);
  if (ready !== 0) return ready;
  const actionable = Number(right.hasActionableTicket) - Number(left.hasActionableTicket);
  if (actionable !== 0) return actionable;
  const modified = right.modifiedAt - left.modifiedAt;
  if (modified !== 0) return modified;
  const leftIdentity = filesystemIdentity(left.trackerDir);
  const rightIdentity = filesystemIdentity(right.trackerDir);
  return leftIdentity < rightIdentity ? -1 : leftIdentity > rightIdentity ? 1 : 0;
}

function trackerArgs(selectedWorkspace) {
  if (process.env.ALBERT_TRACKER_DIR) {
    const trackerDir = resolve(process.env.ALBERT_TRACKER_DIR);
    const issuesDir = process.env.ALBERT_ISSUES_DIR
      ? resolve(process.env.ALBERT_ISSUES_DIR)
      : existsSync(resolve(trackerDir, "issues"))
        ? resolve(trackerDir, "issues")
        : trackerDir;
    return {
      trackerDir,
      issuesDir,
      missionId: process.env.ALBERT_MISSION_ID || basename(trackerDir),
    };
  }

  const candidates = [];
  const scratchRoot = resolve(selectedWorkspace, ".scratch");
  if (existsSync(scratchRoot)) {
    try {
      candidates.push(
        ...readdirSync(scratchRoot, { withFileTypes: true })
          .filter((entry) => entry.isDirectory())
          .map((entry) => resolve(scratchRoot, entry.name))
          .filter((trackerDir) => existsSync(resolve(trackerDir, "PRD.md")))
          .map((trackerDir) => trackerCandidate(trackerDir, basename(trackerDir))),
      );
    } catch {
      // Fall through to the conventional .agent tracker when .scratch cannot
      // be inspected; workspace access preflight owns the user-facing error.
    }
  }

  const agentIssuesTracker = resolve(selectedWorkspace, ".agent", "issues");
  if (existsSync(resolve(agentIssuesTracker, "PRD.md"))) {
    candidates.push(trackerCandidate(agentIssuesTracker, "agent-issues"));
  }
  if (candidates.length > 0) {
    const selected = candidates.sort(compareTrackerCandidates)[0];
    return {
      trackerDir: selected.trackerDir,
      issuesDir: selected.issuesDir,
      missionId: selected.missionId,
    };
  }
  const trackerDir = resolve(selectedWorkspace, ".agent");
  return {
    trackerDir,
    issuesDir: resolve(trackerDir, "issues"),
    missionId: basename(selectedWorkspace),
  };
}

function headlessContextArgs(selectedWorkspace) {
  const { trackerDir, issuesDir, missionId } = trackerArgs(selectedWorkspace);
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
    missionId,
    "--agent-config",
    agentConfigPath,
  ];
}

function headlessExecutionEnvironment(environment = process.env) {
  if (repositoryLayoutAvailable) return {};
  const candidateEnabled = environment.ALFREDO_RUST_CANDIDATE_ENABLED ?? "1";
  const localAgentEnabled =
    environment.ALFREDO_RUST_LOCAL_AGENT_ENABLED ?? "1";
  if (![candidateEnabled, localAgentEnabled].every((value) => value === "0" || value === "1")) {
    throw new Error(
      "ALFREDO_RUST_CANDIDATE_ENABLED and ALFREDO_RUST_LOCAL_AGENT_ENABLED must be 0 or 1",
    );
  }
  if (candidateEnabled === "0" || localAgentEnabled === "0") {
    return {
      ALFREDO_RUST_CANDIDATE_ENABLED: candidateEnabled,
      ALFREDO_RUST_LOCAL_AGENT_ENABLED: localAgentEnabled,
      ALFREDO_RUST_EXECUTION_PROVIDER: "",
      ALFREDO_RUST_EXECUTION_PROVIDER_SHA256: "",
    };
  }
  return resolveDesktopAdapter(projectRoot, {
    ...environment,
    ALFREDO_RUST_SHELL_ENABLED:
      environment.ALFREDO_RUST_SHELL_ENABLED ?? "0",
  }).environment;
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
    cwd: backendRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      ...headlessExecutionEnvironment(),
      PYTHONPATH: process.env.PYTHONPATH
        ? `${backendRoot}${delimiter}${process.env.PYTHONPATH}`
        : backendRoot,
    },
  });
}

function runAgentsBackend() {
  return spawnSync(pythonCommand(), ["-m", "albert_mvp", "agents", ...headlessContextArgs(process.cwd())], {
    cwd: backendRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      PYTHONPATH: process.env.PYTHONPATH
        ? `${backendRoot}${delimiter}${process.env.PYTHONPATH}`
        : backendRoot,
    },
  });
}

function launchDryRunPlan(plan) {
  return {
    command: plan.desktop_adapter.command,
    cwd: plan.desktop_adapter.cwd,
    env: {
      ...plan.desktop_adapter.environment,
      ALBERT_BACKEND_ROOT: plan.backend_root,
      ALFREDO_INSTALL_ROOT: plan.project_root,
      ALFREDO_STARTING_LOCATION: plan.starting_location,
      ALFREDO_AGENT_CONFIG: agentConfigPath,
      ALFREDO_SELECTED_AGENT: plan.selected_agent,
      ALFREDO_SELECTED_MODEL: plan.selected_model,
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
  if (dryRunMode() === "launch") {
    finishLauncherMeasurement(plan.desktop_adapter.kind);
    process.stdout.write(`${JSON.stringify(launchDryRunPlan(plan))}\n`);
    return;
  }
  const [desktopCommand, ...desktopArgs] = plan.desktop_adapter.command;
  finishLauncherMeasurement(plan.desktop_adapter.kind);
  const desktopEnvironment = {
    ...process.env,
    ...plan.desktop_adapter.environment,
    ALBERT_BACKEND_ROOT: plan.backend_root,
    ALFREDO_INSTALL_ROOT: plan.project_root,
    ALFREDO_STARTING_LOCATION: plan.starting_location,
    ALFREDO_AGENT_CONFIG: agentConfigPath,
    ALFREDO_SELECTED_AGENT: plan.selected_agent,
    ALFREDO_SELECTED_MODEL: plan.selected_model,
    ALFREDO_RUNTIME_ROOT: plan.runtime_root,
  };
  delete desktopEnvironment.ALFREDO_SELECTED_WORKSPACE;
  delete desktopEnvironment.ALBERT_MISSION_ID;
  const child = spawn(desktopCommand, desktopArgs, {
    cwd: plan.desktop_adapter.cwd,
    env: desktopEnvironment,
    stdio: "inherit",
  });
  let forwardedSignal = "";
  const forwardSignal = (signal) => {
    forwardedSignal = signal;
    if (!child.kill(signal)) process.exit(1);
  };
  process.once("SIGINT", () => forwardSignal("SIGINT"));
  process.once("SIGTERM", () => forwardSignal("SIGTERM"));
  child.on("error", (error) => {
    process.stderr.write("Alfredo startup preflight failed:\n");
    process.stderr.write(
      `- desktop_shell: Unable to launch Alfredo desktop process: ${error.message}\n  ${plan.desktop_adapter.copyable_action}\n`,
    );
    process.exit(1);
  });
  child.on("exit", (code, signal) => {
    if (forwardedSignal) {
      process.exit(code ?? 0);
      return;
    }
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
  if (command === "--version" || command === "-V") {
    if (argv.length !== 1) throw new Error(`${command} does not accept additional arguments`);
    const manifest = JSON.parse(readFileSync(resolve(projectRoot, "package.json"), "utf8"));
    process.stdout.write(`Alfredo ${manifest.version}\n`);
    process.exit(0);
  }
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
