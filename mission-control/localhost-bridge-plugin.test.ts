// @vitest-environment node

import { EventEmitter } from "node:events";
import { readFileSync } from "node:fs";
import type { IncomingMessage, ServerResponse } from "node:http";
import { homedir } from "node:os";
import { dirname, resolve } from "node:path";
import { PassThrough } from "node:stream";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";

import type { ChildProcess } from "node:child_process";
import type { ResolvedConfig, ViteDevServer } from "vite";
import { afterEach, describe, expect, test, vi } from "vitest";

import {
  ALFREDO_LOCALHOST_ENDPOINT,
  ALFREDO_LOCALHOST_TOKEN_HEADER,
  alfredoLocalhostBridgePlugin,
  isLoopbackPeer,
  type BridgeSpawnContext,
  type SpawnedBridge,
} from "./dev/localhost-bridge-plugin";

const testDirectory = dirname(fileURLToPath(import.meta.url));
const TOKEN = "fixed-test-token";

type Middleware = (
  request: IncomingMessage,
  response: ServerResponse,
  next: () => void,
) => void;

class FakeBridgeChild extends EventEmitter {
  readonly stdin = new PassThrough();
  readonly stdout = new PassThrough();
  readonly stderr = null;
  readonly stdio = [this.stdin, this.stdout, this.stderr];
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;
  killed = false;
  pid = undefined;

  kill(): boolean {
    this.killed = true;
    return true;
  }
}

interface Harness {
  readonly child: FakeBridgeChild;
  readonly closeServer: () => void;
  readonly listenServer: () => void;
  readonly middleware: Middleware;
  readonly spawnBridge: ReturnType<typeof vi.fn>;
  readonly spawnContext: () => BridgeSpawnContext | undefined;
  readonly stop: ReturnType<typeof vi.fn>;
}

class FakeResponse extends EventEmitter {
  readonly headers: Record<string, string> = {};
  body = "";
  destroyed = false;
  statusCode = 200;
  writableEnded = false;

  setHeader(name: string, value: string): this {
    this.headers[name.toLowerCase()] = value;
    return this;
  }

  end(chunk?: string | Buffer): this {
    if (chunk !== undefined) this.body += chunk.toString();
    this.writableEnded = true;
    this.emit("finish");
    return this;
  }
}

function invokePluginHook<T extends (...args: never[]) => unknown>(
  hook: T | { handler: T } | undefined,
  ...args: Parameters<T>
): ReturnType<T> {
  if (hook === undefined) throw new Error("Expected plugin hook");
  return (typeof hook === "function" ? hook : hook.handler)(...args) as ReturnType<T>;
}

async function startHarness(options: {
  readonly appleContainerPortForwarding?: boolean;
  readonly bodyLimit?: number;
  readonly maxInFlightRequests?: number;
  readonly maxRetiredCorrelations?: number;
  readonly onRequest?: (request: Record<string, unknown>, child: FakeBridgeChild) => void;
  readonly requestTimeoutMs?: number;
  readonly startListening?: boolean;
  readonly workstationSessionRunTimeoutMs?: number;
} = {}): Promise<Harness> {
  let middleware: Middleware | undefined;
  const serverLifecycle = new EventEmitter();

  const child = new FakeBridgeChild();
  const stop = vi.fn();
  let spawnContext: BridgeSpawnContext | undefined;
  const spawnBridge = vi.fn((context: BridgeSpawnContext): SpawnedBridge => {
    spawnContext = context;
    return {
      child: child as unknown as ChildProcess,
      stop,
    };
  });
  let input = "";
  child.stdin.setEncoding("utf8");
  child.stdin.on("data", (chunk: string) => {
    input += chunk;
    let newline = input.indexOf("\n");
    while (newline >= 0) {
      const line = input.slice(0, newline);
      input = input.slice(newline + 1);
      const parsed = JSON.parse(line) as Record<string, unknown>;
      options.onRequest?.(parsed, child);
      newline = input.indexOf("\n");
    }
  });

  const plugin = alfredoLocalhostBridgePlugin({
    allowAppleContainerPortForwarding: options.appleContainerPortForwarding,
    missionControlRoot: "/tmp/alfredo-plugin-test/mission-control",
    maxInFlightRequests: options.maxInFlightRequests,
    requestBodyLimitBytes: options.bodyLimit,
    requestTimeoutMs: options.requestTimeoutMs,
    maxRetiredCorrelations: options.maxRetiredCorrelations,
    spawnBridge,
    token: TOKEN,
    workstationSessionRunTimeoutMs: options.workstationSessionRunTimeoutMs,
  });
  invokePluginHook(
    plugin.configResolved,
    {
      mode: options.appleContainerPortForwarding ? "apple-container" : "localhost",
      server: {
        host: options.appleContainerPortForwarding ? "0.0.0.0" : "127.0.0.1",
        https: false,
        port: 1420,
        strictPort: true,
      },
    } as unknown as ResolvedConfig,
  );
  invokePluginHook(plugin.configureServer, {
    httpServer: serverLifecycle,
    middlewares: {
      use(handler: Middleware) {
        middleware = handler;
      },
    },
    watcher: new EventEmitter(),
  } as unknown as ViteDevServer);
  if (middleware === undefined) throw new Error("Gateway middleware was not installed");
  const listenServer = () => serverLifecycle.emit("listening");
  if (options.startListening !== false) listenServer();
  if (options.startListening !== false && spawnContext === undefined) {
    throw new Error("Bridge was not spawned after the server started listening");
  }
  return {
    child,
    closeServer: () => serverLifecycle.emit("close"),
    listenServer,
    middleware,
    spawnBridge,
    spawnContext: () => spawnContext,
    stop,
  };
}

function requireSpawnContext(harness: Harness): BridgeSpawnContext {
  const context = harness.spawnContext();
  if (context === undefined) throw new Error("Bridge was not spawned");
  return context;
}

interface RequestOptions {
  readonly body?: string;
  readonly contentType?: string;
  readonly host?: string;
  readonly method?: string;
  readonly origin?: string;
  readonly path?: string;
  readonly remoteAddress?: string;
  readonly token?: string;
}

function sendRequest(harness: Harness, options: RequestOptions = {}): Promise<{
  readonly body: string;
  readonly headers: Record<string, string>;
  readonly status: number;
}> {
  const body = options.body ?? JSON.stringify({ id: "request-1", command: "workspace_snapshot", args: {} });
  return new Promise((resolveResponse, rejectResponse) => {
    const request = new PassThrough();
    Object.assign(request, {
      headers: {
        "content-length": String(Buffer.byteLength(body)),
        "content-type": options.contentType ?? "application/json",
        host: options.host ?? "127.0.0.1:1420",
        origin: options.origin ?? "http://127.0.0.1:1420",
        [ALFREDO_LOCALHOST_TOKEN_HEADER]: options.token ?? TOKEN,
      },
      method: options.method ?? "POST",
      socket: { remoteAddress: options.remoteAddress ?? "127.0.0.1" },
      url: options.path ?? ALFREDO_LOCALHOST_ENDPOINT,
    });
    const response = new FakeResponse();
    response.once("finish", () => {
      resolveResponse({
        body: response.body,
        headers: response.headers,
        status: response.statusCode,
      });
    });
    request.once("error", rejectResponse);
    harness.middleware(
      request as unknown as IncomingMessage,
      response as unknown as ServerResponse,
      () => {
        response.statusCode = 404;
        response.end();
      },
    );
    request.end(body);
  });
}

async function closeHarness(harness: Harness): Promise<void> {
  harness.closeServer();
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
});

describe("Alfredo localhost bridge plugin", () => {
  test("installs middleware but never spawns when Vite closes before listening", async () => {
    const harness = await startHarness({ startListening: false });

    expect(harness.spawnBridge).not.toHaveBeenCalled();
    const beforeListening = await sendRequest(harness);
    expect(beforeListening.status).toBe(503);

    await closeHarness(harness);
    harness.listenServer();
    expect(harness.spawnBridge).not.toHaveBeenCalled();
    expect(harness.stop).not.toHaveBeenCalled();
  });

  test("honors an explicit Starting Location without inheriting a workspace binding", async () => {
    vi.stubEnv("ALFREDO_STARTING_LOCATION", "/tmp/alfredo-explicit-start");
    vi.stubEnv("ALFREDO_SELECTED_WORKSPACE", "/tmp/stale-workspace");
    vi.stubEnv("ALBERT_MISSION_ID", "stale-mission");
    const harness = await startHarness();
    const spawnContext = requireSpawnContext(harness);

    expect(spawnContext.environment.ALFREDO_STARTING_LOCATION).toBe(
      "/tmp/alfredo-explicit-start",
    );
    expect(spawnContext.environment.ALFREDO_SELECTED_WORKSPACE).toBeUndefined();
    expect(spawnContext.environment.ALBERT_MISSION_ID).toBeUndefined();

    await closeHarness(harness);
  });

  test("moves an explicit backend-root Starting Location to its safe parent", async () => {
    vi.stubEnv("ALFREDO_STARTING_LOCATION", "/tmp/alfredo-plugin-test");
    const harness = await startHarness();
    const spawnContext = requireSpawnContext(harness);

    expect(spawnContext.environment.ALFREDO_STARTING_LOCATION).toBe("/tmp");
    expect(spawnContext.environment.ALFREDO_INSTALL_ROOT).toBe(
      "/tmp/alfredo-plugin-test/mission-control",
    );

    await closeHarness(harness);
  });

  test("injects an immutable per-process capability and forwards the unchanged correlated envelope", async () => {
    const rawEnvelope =
      '{"id":"request-1","ok":true,"value":{"preserved":"without reserialization"}}';
    const harness = await startHarness({
      onRequest(request, child) {
        expect(request).toEqual({
          id: "request-1",
          command: "workspace_snapshot",
          args: {},
        });
        child.stdout.write(`${rawEnvelope}\n`);
      },
    });

    const response = await sendRequest(harness);
    const spawnContext = requireSpawnContext(harness);
    expect(response.status).toBe(200);
    expect(response.body).toBe(rawEnvelope);
    expect(response.headers["access-control-allow-origin"]).toBeUndefined();
    expect(spawnContext.missionControlRoot).toBe(
      "/tmp/alfredo-plugin-test/mission-control",
    );
    expect(spawnContext.environment).toMatchObject({
      ALBERT_BACKEND_ROOT: "/tmp/alfredo-plugin-test",
      ALFREDO_AGENT_CONFIG: "/tmp/alfredo-plugin-test/.albert/agents.json",
      ALFREDO_STARTING_LOCATION: "/tmp",
    });
    expect(spawnContext.environment.ALFREDO_STARTING_LOCATION).not.toBe(
      spawnContext.environment.ALBERT_BACKEND_ROOT,
    );
    expect(spawnContext.environment.ALFREDO_RUNTIME_ROOT).toBe(
      process.env.ALFREDO_RUNTIME_ROOT?.trim()
        ? process.env.ALFREDO_RUNTIME_ROOT
        : resolve(homedir(), ".alfredo", "runtime"),
    );

    const plugin = alfredoLocalhostBridgePlugin({ token: TOKEN });
    const transformIndexHtml = plugin.transformIndexHtml as unknown as (
      html: string,
      context: unknown,
    ) => Array<{ children: string }>;
    const tags = transformIndexHtml("<html></html>", {});
    const sandbox: Record<string, unknown> = {};
    runInNewContext(tags[0].children, sandbox);
    const descriptor = Object.getOwnPropertyDescriptor(
      sandbox,
      "__ALFREDO_LOCALHOST_BRIDGE__",
    );
    expect(descriptor?.writable).toBe(false);
    expect(descriptor?.configurable).toBe(false);
    expect(Object.isFrozen(descriptor?.value)).toBe(true);
    expect(descriptor?.value).toEqual({
      endpoint: ALFREDO_LOCALHOST_ENDPOINT,
      token: TOKEN,
    });

    await closeHarness(harness);
    expect(harness.stop).toHaveBeenCalledTimes(1);
  });

  test.each([
    ["non-loopback peer", { remoteAddress: "192.168.1.20" }, 403],
    ["wrong host", { host: "localhost:1420" }, 403],
    ["wrong origin", { origin: "http://localhost:1420" }, 403],
    ["wrong method", { method: "PUT" }, 405],
    ["wrong media type", { contentType: "text/plain" }, 415],
    ["missing capability", { token: "" }, 403],
    ["extra request field", {
      body: JSON.stringify({
        id: "request-1",
        command: "workspace_snapshot",
        args: {},
        rawArgv: ["python3"],
      }),
    }, 400],
  ] as const)("rejects %s before forwarding", async (_name, requestOptions, status) => {
    const forwarded = vi.fn();
    const harness = await startHarness({ onRequest: forwarded });

    const response = await sendRequest(harness, requestOptions);
    expect(response.status).toBe(status);
    expect(response.headers["access-control-allow-origin"]).toBeUndefined();
    expect(forwarded).not.toHaveBeenCalled();

    await closeHarness(harness);
  });

  test("accepts an Apple container forwarded peer without weakening origin or token checks", async () => {
    const forwardedPeer = "192.168.64.1";
    const harness = await startHarness({
      appleContainerPortForwarding: true,
      onRequest(request, child) {
        child.stdout.write(
          `${JSON.stringify({ id: request.id, ok: true, value: { forwarded: true } })}\n`,
        );
      },
    });

    const accepted = await sendRequest(harness, { remoteAddress: forwardedPeer });
    expect(accepted.status).toBe(200);
    expect(JSON.parse(accepted.body)).toMatchObject({
      id: "request-1",
      ok: true,
      value: { forwarded: true },
    });

    const wrongOrigin = await sendRequest(harness, {
      body: JSON.stringify({ id: "request-2", command: "workspace_snapshot", args: {} }),
      origin: "http://192.168.64.2:1420",
      remoteAddress: forwardedPeer,
    });
    expect(wrongOrigin.status).toBe(403);

    const wrongToken = await sendRequest(harness, {
      body: JSON.stringify({ id: "request-3", command: "workspace_snapshot", args: {} }),
      remoteAddress: forwardedPeer,
      token: "wrong-token",
    });
    expect(wrongToken.status).toBe(403);

    await closeHarness(harness);
  });

  test("bounds request bodies and rejects a mismatched Rust response id", async () => {
    const oversizedHarness = await startHarness({ bodyLimit: 64, onRequest: vi.fn() });
    const oversized = await sendRequest(oversizedHarness, {
      body: JSON.stringify({
        id: "request-1",
        command: "workspace_snapshot",
        args: { padding: "x".repeat(128) },
      }),
    });
    expect(oversized.status).toBe(413);
    await closeHarness(oversizedHarness);

    const mismatchHarness = await startHarness({
      onRequest(_request, child) {
        child.stdout.write('{"id":"different-id","ok":true,"value":{}}\n');
      },
    });
    const mismatch = await sendRequest(mismatchHarness);
    expect(mismatch.status).toBe(502);
    expect(mismatch.body).toContain("bridge-correlation-mismatch");
    expect(mismatchHarness.stop).toHaveBeenCalledTimes(1);
    await closeHarness(mismatchHarness);
  });

  test("bounds concurrent work and correlates out-of-order Rust envelopes", async () => {
    const firstEnvelope = '{"id":"request-1","ok":true,"value":{"order":1}}';
    const secondEnvelope = '{"id":"request-2","ok":true,"value":{"order":2}}';
    const forwarded: string[] = [];
    let twoRequestsForwarded: (() => void) | undefined;
    const bothForwarded = new Promise<void>((resolveForwarded) => {
      twoRequestsForwarded = resolveForwarded;
    });
    const harness = await startHarness({
      maxInFlightRequests: 2,
      onRequest(request) {
        forwarded.push(String(request.id));
        if (forwarded.length === 2) twoRequestsForwarded?.();
      },
    });
    const completionOrder: string[] = [];
    const first = sendRequest(harness, {
      body: JSON.stringify({ id: "request-1", command: "workstation_session_run", args: {} }),
    }).then((response) => {
      completionOrder.push("request-1");
      return response;
    });
    const second = sendRequest(harness, {
      body: JSON.stringify({ id: "request-2", command: "workspace_snapshot", args: {} }),
    }).then((response) => {
      completionOrder.push("request-2");
      return response;
    });
    await bothForwarded;

    const overCapacity = await sendRequest(harness, {
      body: JSON.stringify({ id: "request-3", command: "workspace_snapshot", args: {} }),
    });
    expect(overCapacity.status).toBe(429);
    expect(forwarded).toEqual(["request-1", "request-2"]);

    harness.child.stdout.write(`${secondEnvelope}\n${firstEnvelope}\n`);
    const [firstResponse, secondResponse] = await Promise.all([first, second]);
    expect(firstResponse.body).toBe(firstEnvelope);
    expect(secondResponse.body).toBe(secondEnvelope);
    expect(completionOrder).toEqual(["request-2", "request-1"]);

    await closeHarness(harness);
  });

  test("keeps a legal workstation runner alive beyond the ordinary five-minute timeout", async () => {
    vi.useFakeTimers();
    const runnerEnvelope = '{"id":"runner-1","ok":true,"value":{"status":"completed"}}';
    const harness = await startHarness();
    let settled = false;
    const runner = sendRequest(harness, {
      body: JSON.stringify({
        id: "runner-1",
        command: "workstation_session_run",
        args: { request: { sessionId: "session-1" } },
      }),
    }).then((response) => {
      settled = true;
      return response;
    });

    await vi.advanceTimersByTimeAsync(600_001);
    expect(settled).toBe(false);
    expect(harness.stop).not.toHaveBeenCalled();

    harness.child.stdout.write(`${runnerEnvelope}\n`);
    const response = await runner;
    expect(response.status).toBe(200);
    expect(response.body).toBe(runnerEnvelope);
    await closeHarness(harness);
  });

  test("times out one HTTP waiter without stopping the bridge or rejecting a late response", async () => {
    vi.useFakeTimers();
    const harness = await startHarness({
      requestTimeoutMs: 100,
      onRequest(request, child) {
        if (request.id === "control-1") {
          child.stdout.write('{"id":"control-1","ok":true,"value":{"ready":true}}\n');
        }
      },
    });

    const timedOut = sendRequest(harness, {
      body: JSON.stringify({ id: "timed-out-1", command: "workspace_snapshot", args: {} }),
    });
    await vi.advanceTimersByTimeAsync(101);
    await expect(timedOut).resolves.toMatchObject({
      status: 504,
      body: '{"error":"bridge-timeout"}',
    });
    expect(harness.stop).not.toHaveBeenCalled();

    const control = await sendRequest(harness, {
      body: JSON.stringify({ id: "control-1", command: "workspace_snapshot", args: {} }),
    });
    expect(control.status).toBe(200);
    expect(control.body).toBe('{"id":"control-1","ok":true,"value":{"ready":true}}');
    expect(harness.stop).not.toHaveBeenCalled();

    harness.child.stdout.write(
      '{"id":"timed-out-1","ok":true,"value":{"late":true}}\n',
    );
    expect(harness.stop).not.toHaveBeenCalled();

    await closeHarness(harness);
  });

  test("fails closed when a retired correlation receives a malformed late envelope", async () => {
    vi.useFakeTimers();
    const harness = await startHarness({ requestTimeoutMs: 100 });

    const timedOut = sendRequest(harness, {
      body: JSON.stringify({ id: "malformed-late-1", command: "workspace_snapshot", args: {} }),
    });
    await vi.advanceTimersByTimeAsync(101);
    await expect(timedOut).resolves.toMatchObject({
      status: 504,
      body: '{"error":"bridge-timeout"}',
    });

    harness.child.stdout.write('{"id":"malformed-late-1","ok":true}\n');
    expect(harness.stop).toHaveBeenCalledTimes(1);

    await closeHarness(harness);
  });

  test("fails closed when a write pipe reports an error after its waiter timed out", async () => {
    vi.useFakeTimers();
    const harness = await startHarness({ requestTimeoutMs: 100 });

    const timedOut = sendRequest(harness, {
      body: JSON.stringify({ id: "pipe-failure-1", command: "workspace_snapshot", args: {} }),
    });
    await vi.advanceTimersByTimeAsync(101);
    await expect(timedOut).resolves.toMatchObject({
      status: 504,
      body: '{"error":"bridge-timeout"}',
    });

    harness.child.stdin.on("error", () => undefined);
    harness.child.stdin.destroy(new Error("pipe closed"));
    const afterPipeFailure = sendRequest(harness, {
      body: JSON.stringify({ id: "pipe-failure-2", command: "workspace_snapshot", args: {} }),
    });
    await expect(afterPipeFailure).resolves.toMatchObject({
      status: 503,
      body: '{"error":"bridge-unavailable"}',
    });
    expect(harness.stop).toHaveBeenCalledTimes(1);

    await closeHarness(harness);
  });

  test("keeps an accepted workstation run recoverable through canonical polling after its waiter times out", async () => {
    vi.useFakeTimers();
    const harness = await startHarness({
      workstationSessionRunTimeoutMs: 100,
      onRequest(request, child) {
        if (request.id === "poll-running") {
          child.stdout.write(
            '{"id":"poll-running","ok":true,"value":{"status":"running","session_id":"session-1"}}\n',
          );
        }
        if (request.id === "poll-complete") {
          child.stdout.write(
            '{"id":"poll-complete","ok":true,"value":{"status":"completed","session_id":"session-1"}}\n',
          );
        }
      },
    });

    const run = sendRequest(harness, {
      body: JSON.stringify({
        id: "run-1",
        command: "workstation_session_run",
        args: { request: { sessionId: "session-1" } },
      }),
    });
    await vi.advanceTimersByTimeAsync(101);
    await expect(run).resolves.toMatchObject({
      status: 504,
      body: '{"error":"bridge-timeout"}',
    });
    expect(harness.stop).not.toHaveBeenCalled();

    const running = await sendRequest(harness, {
      body: JSON.stringify({
        id: "poll-running",
        command: "workspace_snapshot",
        args: { session_id: "session-1" },
      }),
    });
    expect(running.status).toBe(200);
    expect(running.body).toContain('"status":"running"');

    harness.child.stdout.write(
      '{"id":"run-1","ok":true,"value":{"status":"completed","session_id":"session-1"}}\n',
    );
    expect(harness.stop).not.toHaveBeenCalled();

    const complete = await sendRequest(harness, {
      body: JSON.stringify({
        id: "poll-complete",
        command: "workspace_snapshot",
        args: { session_id: "session-1" },
      }),
    });
    expect(complete.status).toBe(200);
    expect(complete.body).toContain('"status":"completed"');
    expect(harness.stop).not.toHaveBeenCalled();

    await closeHarness(harness);
  });

  test("bounds retired correlations and prevents unsafe duplicate request-id reuse", async () => {
    vi.useFakeTimers();
    const harness = await startHarness({
      maxRetiredCorrelations: 1,
      requestTimeoutMs: 100,
      onRequest(request, child) {
        if (request.id === "after-late-response") {
          child.stdout.write(
            '{"id":"after-late-response","ok":true,"value":{"accepted":true}}\n',
          );
        }
      },
    });

    const timedOut = sendRequest(harness, {
      body: JSON.stringify({ id: "retired-1", command: "workspace_snapshot", args: {} }),
    });
    await vi.advanceTimersByTimeAsync(101);
    await expect(timedOut).resolves.toMatchObject({
      status: 504,
      body: '{"error":"bridge-timeout"}',
    });

    const duplicate = await sendRequest(harness, {
      body: JSON.stringify({ id: "retired-1", command: "workspace_snapshot", args: {} }),
    });
    expect(duplicate.status).toBe(409);
    expect(duplicate.body).toContain("duplicate-request-id");

    const atCapacity = await sendRequest(harness, {
      body: JSON.stringify({ id: "retired-2", command: "workspace_snapshot", args: {} }),
    });
    expect(atCapacity.status).toBe(429);
    expect(atCapacity.body).toContain("bridge-correlation-capacity-exceeded");
    expect(harness.stop).not.toHaveBeenCalled();

    harness.child.stdout.write(
      '{"id":"retired-1","ok":true,"value":{"late":true}}\n',
    );
    const afterLateResponse = await sendRequest(harness, {
      body: JSON.stringify({
        id: "after-late-response",
        command: "workspace_snapshot",
        args: {},
      }),
    });
    expect(afterLateResponse.status).toBe(200);
    expect(harness.stop).not.toHaveBeenCalled();

    await closeHarness(harness);
  });

  test("stops the bridge process group when its transport ends abnormally", async () => {
    const harness = await startHarness();

    const ended = new Promise<void>((resolveEnded) => {
      harness.child.stdout.once("end", resolveEnded);
    });
    harness.child.stdout.end();
    await ended;
    expect(harness.stop).toHaveBeenCalledTimes(1);

    await closeHarness(harness);
  });

  test("recognizes only IPv4 and IPv6 loopback peers", () => {
    expect(isLoopbackPeer("127.0.0.1")).toBe(true);
    expect(isLoopbackPeer("::1")).toBe(true);
    expect(isLoopbackPeer("::ffff:127.0.0.1")).toBe(true);
    expect(isLoopbackPeer("192.168.1.20")).toBe(false);
    expect(isLoopbackPeer(undefined)).toBe(false);
  });

  test("keeps browser and native development on explicit, isolated loopback modes", () => {
    const packageJson = JSON.parse(
      readFileSync(resolve(testDirectory, "package.json"), "utf8"),
    ) as { scripts: Record<string, string> };
    const tauriConfig = JSON.parse(
      readFileSync(resolve(testDirectory, "src-tauri/tauri.conf.json"), "utf8"),
    ) as { build: { beforeDevCommand: string; devUrl: string } };

    expect(packageJson.scripts.dev).toBe(
      "vite --mode localhost --host 127.0.0.1 --port 1420 --strictPort",
    );
    expect(packageJson.scripts["dev:container"]).toBe(
      "vite --mode apple-container --host 0.0.0.0 --port 1420 --strictPort",
    );
    expect(packageJson.scripts["dev:tauri"]).toBe(
      "vite --mode tauri --host 127.0.0.1 --port 1422 --strictPort",
    );
    expect(tauriConfig.build).toMatchObject({
      beforeDevCommand: "npm run dev:tauri",
      devUrl: "http://127.0.0.1:1422",
    });
  });
});
