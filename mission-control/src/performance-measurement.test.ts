import {
  afterTwoAnimationFrames,
  markFrontendPerformance,
  markNativePerformance,
} from "./performance-measurement";
import type { PerformanceMarkRequest } from "./contracts";
import type { WorkspaceClient } from "./workspace-client";

test("frontend and native marks retain their distinct monotonic clock owners", async () => {
  const requests: PerformanceMarkRequest[] = [];
  const client: WorkspaceClient = {
    async loadSnapshot() {
      return { kind: "startup-failure", message: "unused", recoverable: false };
    },
    async recordPerformanceMark(request) {
      requests.push(request);
      return { recorded: true };
    },
  };

  await markFrontendPerformance(client, "S3", "start", { outcome: "pass" });
  await markNativePerformance(client, "S4", "start", { outcome: "pass" });

  expect(requests[0]).toMatchObject({
    stage: "S3",
    boundary: "start",
    clock: "frontend",
    detail: { outcome: "pass" },
  });
  expect(requests[0].clock_id).toMatch(/^react:/);
  expect(requests[0].monotonic_ns).toMatch(/^\d+$/);
  expect(requests[1]).toEqual({
    stage: "S4",
    boundary: "start",
    clock: "native",
    monotonic_ns: "",
    clock_id: "",
    detail: { outcome: "pass" },
  });
});

test("paint boundary waits for two browser animation frames", async () => {
  const callbacks: FrameRequestCallback[] = [];
  const animationFrame = vi
    .spyOn(window, "requestAnimationFrame")
    .mockImplementation((callback) => {
      callbacks.push(callback);
      return callbacks.length;
    });
  let finished = false;
  const paint = afterTwoAnimationFrames().then(() => {
    finished = true;
  });

  expect(callbacks).toHaveLength(1);
  callbacks.shift()!(1);
  expect(callbacks).toHaveLength(1);
  expect(finished).toBe(false);
  callbacks.shift()!(2);
  await paint;

  expect(finished).toBe(true);
  animationFrame.mockRestore();
});
