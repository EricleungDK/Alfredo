import { spawn, type ChildProcess } from "node:child_process";
import { randomBytes, timingSafeEqual } from "node:crypto";
import type { IncomingMessage, ServerResponse } from "node:http";
import { homedir } from "node:os";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import type { Readable, Writable } from "node:stream";
import { TextDecoder } from "node:util";
import { fileURLToPath } from "node:url";

import type { Plugin, ResolvedConfig, ViteDevServer } from "vite";

export const ALFREDO_LOCALHOST_ENDPOINT = "/__alfredo/invoke";
export const ALFREDO_LOCALHOST_TOKEN_HEADER = "x-alfredo-bridge-token";

const LOOPBACK_HOST = "127.0.0.1";
const DEFAULT_REQUEST_BODY_LIMIT_BYTES = 1 * 1024 * 1024;
const DEFAULT_RESPONSE_LINE_LIMIT_BYTES = 16 * 1024 * 1024;
const DEFAULT_REQUEST_TIMEOUT_MS = 5 * 60 * 1000;
const DEFAULT_WORKSTATION_SESSION_RUN_TIMEOUT_MS = 2 * 60 * 60 * 1000;
const DEFAULT_MAX_IN_FLIGHT_REQUESTS = 32;
const MAX_IDENTIFIER_LENGTH = 128;
const MAX_COMMAND_LENGTH = 128;
const BRIDGE_COMMAND = "cargo";
const BRIDGE_ARGUMENTS = [
  "run",
  "--quiet",
  "--no-default-features",
  "--manifest-path",
  "src-tauri/Cargo.toml",
  "--bin",
  "alfredo-localhost-bridge",
] as const;

const missionControlDirectory = dirname(dirname(fileURLToPath(import.meta.url)));
const strictUtf8Decoder = new TextDecoder("utf-8", { fatal: true });

interface InvokeRequest {
  readonly id: string;
  readonly command: string;
  readonly args: Record<string, unknown>;
}

interface PendingRequest {
  readonly response: ServerResponse;
  readonly payload: InvokeRequest;
  timeout?: ReturnType<typeof setTimeout>;
}

export interface BridgeSpawnContext {
  readonly missionControlRoot: string;
  readonly environment: NodeJS.ProcessEnv;
}

export interface SpawnedBridge {
  readonly child: ChildProcess;
  stop(): void;
}

export interface LocalhostBridgePluginOptions {
  readonly allowAppleContainerPortForwarding?: boolean;
  readonly missionControlRoot?: string;
  readonly token?: string;
  readonly requestBodyLimitBytes?: number;
  readonly responseLineLimitBytes?: number;
  readonly requestTimeoutMs?: number;
  readonly workstationSessionRunTimeoutMs?: number;
  readonly maxInFlightRequests?: number;
  readonly spawnBridge?: (context: BridgeSpawnContext) => SpawnedBridge;
}

function terminateWindowsProcessTree(processId: number, force: boolean): void {
  const arguments_ = ["/PID", String(processId), "/T"];
  if (force) arguments_.push("/F");
  let terminator: ChildProcess;
  try {
    terminator = spawn("taskkill", arguments_, {
      shell: false,
      stdio: "ignore",
      windowsHide: true,
    });
  } catch {
    return;
  }
  const timeout = setTimeout(() => {
    try {
      terminator.kill();
    } catch {
      // The bounded taskkill helper may already have exited.
    }
  }, 2_000);
  timeout.unref();
  const finished = () => clearTimeout(timeout);
  terminator.once("error", finished);
  terminator.once("exit", finished);
  terminator.unref();
}

function signalBridgeProcess(
  child: ChildProcess,
  processGroupId: number | undefined,
  signal: NodeJS.Signals,
): void {
  if (process.platform === "win32" && child.pid !== undefined) {
    terminateWindowsProcessTree(child.pid, signal === "SIGKILL");
    return;
  }
  if (process.platform !== "win32" && processGroupId !== undefined) {
    try {
      process.kill(-processGroupId, signal);
      return;
    } catch {
      // Fall through to the direct-child signal if the process group has
      // already disappeared or was not created by this platform.
    }
  }

  if (child.exitCode !== null) return;

  try {
    child.kill(signal);
  } catch {
    // The child may have exited between the liveness check and the signal.
  }
}

function spawnDefaultBridge(context: BridgeSpawnContext): SpawnedBridge {
  const child = spawn(BRIDGE_COMMAND, [...BRIDGE_ARGUMENTS], {
    cwd: context.missionControlRoot,
    detached: process.platform !== "win32",
    env: context.environment,
    shell: false,
    stdio: ["pipe", "pipe", "inherit"],
    windowsHide: true,
  });
  const processGroupId = process.platform === "win32" ? undefined : child.pid;
  let stopped = false;

  const stop = () => {
    if (stopped) return;
    stopped = true;
    signalBridgeProcess(child, processGroupId, "SIGTERM");
    const forceStop = setTimeout(
      () => signalBridgeProcess(child, processGroupId, "SIGKILL"),
      2_000,
    );
    forceStop.unref();
  };

  return { child, stop };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readSingleHeader(request: IncomingMessage, name: string): string | undefined {
  const value = request.headers[name];
  return typeof value === "string" ? value : undefined;
}

function tokenMatches(candidate: string | undefined, expected: string): boolean {
  if (candidate === undefined) return false;
  const candidateBytes = Buffer.from(candidate, "utf8");
  const expectedBytes = Buffer.from(expected, "utf8");
  return (
    candidateBytes.byteLength === expectedBytes.byteLength &&
    timingSafeEqual(candidateBytes, expectedBytes)
  );
}

export function isLoopbackPeer(address: string | undefined): boolean {
  return address === "127.0.0.1" || address === "::1" || address === "::ffff:127.0.0.1";
}

function sendJsonError(response: ServerResponse, status: number, code: string): void {
  if (response.writableEnded || response.destroyed) return;
  response.statusCode = status;
  response.setHeader("cache-control", "no-store");
  response.setHeader("content-type", "application/json");
  response.setHeader("x-content-type-options", "nosniff");
  response.end(JSON.stringify({ error: code }));
}

function sendBridgeEnvelope(response: ServerResponse, envelope: string): void {
  if (response.writableEnded || response.destroyed) return;
  response.statusCode = 200;
  response.setHeader("cache-control", "no-store");
  response.setHeader("content-type", "application/json");
  response.setHeader("x-content-type-options", "nosniff");
  response.end(envelope);
}

type BodyReadResult =
  | { readonly kind: "body"; readonly body: Buffer }
  | { readonly kind: "invalid" }
  | { readonly kind: "too-large" };

function contentLengthWithinLimit(
  request: IncomingMessage,
  limit: number,
): "valid" | "invalid" | "too-large" {
  const header = readSingleHeader(request, "content-length");
  if (header === undefined) return "valid";
  if (!/^\d+$/.test(header)) return "invalid";
  const length = Number(header);
  if (!Number.isSafeInteger(length)) return "invalid";
  return length > limit ? "too-large" : "valid";
}

function readBoundedBody(request: IncomingMessage, limit: number): Promise<BodyReadResult> {
  const declaredLength = contentLengthWithinLimit(request, limit);
  if (declaredLength !== "valid") return Promise.resolve({ kind: declaredLength });

  return new Promise((resolveBody) => {
    const chunks: Buffer[] = [];
    let received = 0;
    let complete = false;

    const finish = (result: BodyReadResult) => {
      if (complete) return;
      complete = true;
      request.removeListener("data", onData);
      request.removeListener("end", onEnd);
      request.removeListener("aborted", onAborted);
      request.removeListener("error", onError);
      resolveBody(result);
    };
    const onData = (chunk: Buffer | string) => {
      const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      if (bytes.byteLength > limit - received) {
        finish({ kind: "too-large" });
        request.resume();
        return;
      }
      received += bytes.byteLength;
      chunks.push(bytes);
    };
    const onEnd = () => finish({ kind: "body", body: Buffer.concat(chunks, received) });
    const onAborted = () => finish({ kind: "invalid" });
    const onError = () => finish({ kind: "invalid" });

    request.on("data", onData);
    request.once("end", onEnd);
    request.once("aborted", onAborted);
    request.once("error", onError);
  });
}

function parseInvokeRequest(body: Buffer): InvokeRequest | undefined {
  let decoded: string;
  let parsed: unknown;
  try {
    decoded = strictUtf8Decoder.decode(body);
    parsed = JSON.parse(decoded) as unknown;
  } catch {
    return undefined;
  }
  if (!isRecord(parsed)) return undefined;

  const keys = Object.keys(parsed).sort();
  if (keys.length !== 3 || keys[0] !== "args" || keys[1] !== "command" || keys[2] !== "id") {
    return undefined;
  }
  if (
    typeof parsed.id !== "string" ||
    parsed.id.length === 0 ||
    Buffer.byteLength(parsed.id, "utf8") > MAX_IDENTIFIER_LENGTH ||
    typeof parsed.command !== "string" ||
    !/^[a-z][a-z0-9_]*$/.test(parsed.command) ||
    parsed.command.length > MAX_COMMAND_LENGTH ||
    !isRecord(parsed.args)
  ) {
    return undefined;
  }
  return {
    id: parsed.id,
    command: parsed.command,
    args: parsed.args,
  };
}

class BridgeGateway {
  private readonly child: ChildProcess;
  private readonly stdin: Writable;
  private readonly stdout: Readable;
  private readonly pending = new Map<string, PendingRequest>();
  private outputBuffer = Buffer.alloc(0);
  private unavailable = false;
  private closing = false;

  constructor(
    private readonly spawned: SpawnedBridge,
    private readonly responseLineLimitBytes: number,
    private readonly requestTimeoutMs: number,
    private readonly workstationSessionRunTimeoutMs: number,
    private readonly maxInFlightRequests: number,
  ) {
    this.child = spawned.child;
    if (this.child.stdin === null || this.child.stdout === null) {
      spawned.stop();
      throw new Error("Alfredo localhost bridge requires piped stdin and stdout");
    }
    this.stdin = this.child.stdin;
    this.stdout = this.child.stdout;
    this.stdout.on("data", (chunk: Buffer | string) => this.acceptOutput(chunk));
    this.stdout.once("end", () => this.bridgeBecameUnavailable());
    this.child.once("error", () => this.bridgeBecameUnavailable());
    this.child.once("exit", () => this.bridgeBecameUnavailable());
  }

  enqueue(response: ServerResponse, payload: InvokeRequest): void {
    if (this.unavailable || this.closing) {
      sendJsonError(response, 503, "bridge-unavailable");
      return;
    }
    if (this.pending.has(payload.id)) {
      sendJsonError(response, 409, "duplicate-request-id");
      return;
    }
    if (this.pending.size >= this.maxInFlightRequests) {
      sendJsonError(response, 429, "bridge-capacity-exceeded");
      return;
    }

    const pending: PendingRequest = {
      response,
      payload,
    };
    this.pending.set(payload.id, pending);
    const timeoutMs =
      payload.command === "workstation_session_run"
        ? this.workstationSessionRunTimeoutMs
        : this.requestTimeoutMs;
    pending.timeout = setTimeout(() => this.requestTimedOut(pending), timeoutMs);
    pending.timeout.unref();

    const line = `${JSON.stringify(pending.payload)}\n`;
    try {
      this.stdin.write(line, "utf8", (error) => {
        if (
          error !== null &&
          error !== undefined &&
          this.pending.get(payload.id) === pending
        ) {
          this.bridgeBecameUnavailable();
        }
      });
    } catch {
      this.bridgeBecameUnavailable();
    }
  }

  close(): void {
    if (this.closing) return;
    this.closing = true;
    this.failPending(503, "bridge-stopping");
    this.spawned.stop();
  }

  private acceptOutput(chunk: Buffer | string): void {
    if (this.unavailable || this.closing) return;
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    this.outputBuffer = Buffer.concat([this.outputBuffer, bytes]);

    let newline = this.outputBuffer.indexOf(0x0a);
    while (newline >= 0) {
      if (newline > this.responseLineLimitBytes) {
        this.protocolFailure("bridge-protocol-error");
        return;
      }
      const line = this.outputBuffer.subarray(0, newline);
      this.outputBuffer = this.outputBuffer.subarray(newline + 1);
      this.acceptEnvelope(line);
      if (this.unavailable || this.closing) return;
      newline = this.outputBuffer.indexOf(0x0a);
    }

    if (this.outputBuffer.byteLength > this.responseLineLimitBytes) {
      this.protocolFailure("bridge-protocol-error");
    }
  }

  private acceptEnvelope(line: Buffer): void {
    let rawEnvelope: string;
    let envelope: unknown;
    try {
      rawEnvelope = strictUtf8Decoder.decode(line);
      envelope = JSON.parse(rawEnvelope) as unknown;
    } catch {
      this.protocolFailure("bridge-protocol-error");
      return;
    }

    if (!isRecord(envelope) || typeof envelope.id !== "string") {
      this.protocolFailure("bridge-protocol-error");
      return;
    }
    const pending = this.pending.get(envelope.id);
    if (pending === undefined) {
      this.protocolFailure("bridge-correlation-mismatch");
      return;
    }

    this.finishPending(pending);
    sendBridgeEnvelope(pending.response, rawEnvelope);
  }

  private finishPending(pending: PendingRequest): void {
    if (pending.timeout !== undefined) clearTimeout(pending.timeout);
    if (this.pending.get(pending.payload.id) === pending) {
      this.pending.delete(pending.payload.id);
    }
  }

  private failPending(status: number, code: string): void {
    for (const pending of [...this.pending.values()]) {
      this.finishPending(pending);
      sendJsonError(pending.response, status, code);
    }
  }

  private requestTimedOut(pending: PendingRequest): void {
    if (this.pending.get(pending.payload.id) !== pending) return;
    this.finishPending(pending);
    sendJsonError(pending.response, 504, "bridge-timeout");
    this.bridgeBecameUnavailable();
  }

  private protocolFailure(code: string): void {
    if (this.unavailable) return;
    this.unavailable = true;
    this.failPending(502, code);
    this.spawned.stop();
  }

  private bridgeBecameUnavailable(): void {
    if (this.unavailable || this.closing) return;
    this.unavailable = true;
    this.failPending(503, "bridge-unavailable");
    this.spawned.stop();
  }
}

function pathIsInsideOrEqual(candidate: string, root: string): boolean {
  const relativePath = relative(resolve(root), resolve(candidate));
  return (
    relativePath === "" ||
    (relativePath !== ".." &&
      !relativePath.startsWith(`..${sep}`) &&
      !isAbsolute(relativePath))
  );
}

function safeStartingLocation(candidate: string, protectedRoots: readonly string[]): string {
  let current = resolve(candidate);
  const isProtected = (path: string) =>
    protectedRoots.some((root) => pathIsInsideOrEqual(path, root));
  while (isProtected(current)) {
    const parent = resolve(current, "..");
    if (parent === current) break;
    current = parent;
  }
  return current;
}

function createBridgeEnvironment(missionControlRoot: string): NodeJS.ProcessEnv {
  const repositoryRoot = resolve(missionControlRoot, "..");
  const defaultStartingLocation = resolve(repositoryRoot, "..");
  const runtimeRoot = process.env.ALFREDO_RUNTIME_ROOT?.trim()
    ? process.env.ALFREDO_RUNTIME_ROOT
    : resolve(homedir(), ".alfredo", "runtime");
  const configuredStartingLocation = process.env.ALFREDO_STARTING_LOCATION?.trim()
    ? process.env.ALFREDO_STARTING_LOCATION
    : defaultStartingLocation;
  const installRoot = process.env.ALFREDO_INSTALL_ROOT?.trim() || missionControlRoot;
  const startingLocation = safeStartingLocation(configuredStartingLocation, [
    repositoryRoot,
    missionControlRoot,
    runtimeRoot,
    installRoot,
  ]);
  const environment: NodeJS.ProcessEnv = {
    ...process.env,
    ALBERT_BACKEND_ROOT: repositoryRoot,
    ALFREDO_INSTALL_ROOT: installRoot,
    ALFREDO_AGENT_CONFIG: resolve(repositoryRoot, ".albert", "agents.json"),
    ALFREDO_RUNTIME_ROOT: runtimeRoot,
    ALFREDO_STARTING_LOCATION: startingLocation,
  };
  // Match the managed launcher: a Starting Location must not inherit a stale
  // Coding Workspace or Mission binding from the parent shell.
  delete environment.ALFREDO_SELECTED_WORKSPACE;
  delete environment.ALBERT_MISSION_ID;
  return environment;
}

function requireLocalDevelopmentConfiguration(
  config: ResolvedConfig,
  allowAppleContainerPortForwarding: boolean,
): {
  readonly expectedHost: string;
  readonly expectedOrigin: string;
  readonly requireLoopbackPeer: boolean;
} {
  const requiredMode = allowAppleContainerPortForwarding
    ? "apple-container"
    : "localhost";
  const requiredBindHost = allowAppleContainerPortForwarding
    ? "0.0.0.0"
    : LOOPBACK_HOST;
  if (
    config.mode !== requiredMode ||
    config.server.host !== requiredBindHost ||
    config.server.strictPort !== true ||
    config.server.https
  ) {
    throw new Error(
      allowAppleContainerPortForwarding
        ? "Alfredo Apple container mode requires HTTP on 0.0.0.0 behind a loopback-only published port"
        : "Alfredo localhost mode requires HTTP on 127.0.0.1 with a strict, fixed port",
    );
  }
  const expectedHost = `${LOOPBACK_HOST}:${config.server.port}`;
  return {
    expectedHost,
    expectedOrigin: `http://${expectedHost}`,
    requireLoopbackPeer: !allowAppleContainerPortForwarding,
  };
}

function installGatewayMiddleware(
  server: ViteDevServer,
  gateway: () => BridgeGateway | undefined,
  expectedHost: string,
  expectedOrigin: string,
  token: string,
  requestBodyLimitBytes: number,
  requireLoopbackPeer: boolean,
): void {
  server.middlewares.use((request, response, next) => {
    if (request.url !== ALFREDO_LOCALHOST_ENDPOINT) {
      next();
      return;
    }

    if (requireLoopbackPeer && !isLoopbackPeer(request.socket.remoteAddress)) {
      sendJsonError(response, 403, "loopback-required");
      return;
    }
    if (
      readSingleHeader(request, "host") !== expectedHost ||
      readSingleHeader(request, "origin") !== expectedOrigin
    ) {
      sendJsonError(response, 403, "same-origin-required");
      return;
    }
    if (request.method !== "POST") {
      response.setHeader("allow", "POST");
      sendJsonError(response, 405, "method-not-allowed");
      return;
    }
    if (readSingleHeader(request, "content-type")?.toLowerCase() !== "application/json") {
      sendJsonError(response, 415, "application-json-required");
      return;
    }
    if (!tokenMatches(readSingleHeader(request, ALFREDO_LOCALHOST_TOKEN_HEADER), token)) {
      sendJsonError(response, 403, "invalid-bridge-token");
      return;
    }

    void readBoundedBody(request, requestBodyLimitBytes).then((result) => {
      if (result.kind === "too-large") {
        request.resume();
        sendJsonError(response, 413, "request-too-large");
        return;
      }
      if (result.kind === "invalid") {
        sendJsonError(response, 400, "invalid-request-body");
        return;
      }
      const payload = parseInvokeRequest(result.body);
      if (payload === undefined) {
        sendJsonError(response, 400, "invalid-invoke-request");
        return;
      }
      const activeGateway = gateway();
      if (activeGateway === undefined) {
        sendJsonError(response, 503, "bridge-unavailable");
        return;
      }
      activeGateway.enqueue(response, payload);
    });
  });
}

export function alfredoLocalhostBridgePlugin(
  options: LocalhostBridgePluginOptions = {},
): Plugin {
  const allowAppleContainerPortForwarding =
    options.allowAppleContainerPortForwarding ?? false;
  const token = options.token ?? randomBytes(32).toString("base64url");
  const root = resolve(options.missionControlRoot ?? missionControlDirectory);
  const requestBodyLimitBytes =
    options.requestBodyLimitBytes ?? DEFAULT_REQUEST_BODY_LIMIT_BYTES;
  const responseLineLimitBytes =
    options.responseLineLimitBytes ?? DEFAULT_RESPONSE_LINE_LIMIT_BYTES;
  const requestTimeoutMs = options.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS;
  const workstationSessionRunTimeoutMs =
    options.workstationSessionRunTimeoutMs ?? DEFAULT_WORKSTATION_SESSION_RUN_TIMEOUT_MS;
  const maxInFlightRequests =
    options.maxInFlightRequests ?? DEFAULT_MAX_IN_FLIGHT_REQUESTS;
  if (!Number.isSafeInteger(maxInFlightRequests) || maxInFlightRequests < 1) {
    throw new Error("Alfredo localhost bridge concurrency must be a positive integer");
  }
  const spawnBridge = options.spawnBridge ?? spawnDefaultBridge;
  let expectedHost: string | undefined;
  let expectedOrigin: string | undefined;
  let requireLoopbackPeer = true;
  let gateway: BridgeGateway | undefined;

  return {
    name: "alfredo-localhost-bridge",
    apply: "serve",
    enforce: "pre",
    configResolved(config) {
      ({ expectedHost, expectedOrigin, requireLoopbackPeer } =
        requireLocalDevelopmentConfiguration(
          config,
          allowAppleContainerPortForwarding,
        ));
    },
    transformIndexHtml() {
      const configuration = JSON.stringify({
        endpoint: ALFREDO_LOCALHOST_ENDPOINT,
        token,
      }).replaceAll("<", "\\u003c");
      return [
        {
          tag: "script",
          children:
            `Object.defineProperty(globalThis,"__ALFREDO_LOCALHOST_BRIDGE__",` +
            `{value:Object.freeze(${configuration}),writable:false,configurable:false});`,
          injectTo: "head-prepend",
        },
      ];
    },
    configureServer(server) {
      if (expectedHost === undefined || expectedOrigin === undefined) {
        throw new Error("Alfredo localhost bridge was not configured before server startup");
      }
      installGatewayMiddleware(
        server,
        () => gateway,
        expectedHost,
        expectedOrigin,
        token,
        requestBodyLimitBytes,
        requireLoopbackPeer,
      );
      let closed = false;
      const start = () => {
        if (closed || gateway !== undefined) return;
        try {
          const spawned = spawnBridge({
            missionControlRoot: root,
            environment: createBridgeEnvironment(root),
          });
          gateway = new BridgeGateway(
            spawned,
            responseLineLimitBytes,
            requestTimeoutMs,
            workstationSessionRunTimeoutMs,
            maxInFlightRequests,
          );
        } catch {
          gateway = undefined;
        }
      };
      const close = () => {
        if (closed) return;
        closed = true;
        server.httpServer?.removeListener("listening", start);
        gateway?.close();
      };
      server.httpServer?.once("listening", start);
      server.httpServer?.once("close", close);
      server.watcher.once("close", close);
    },
  };
}
