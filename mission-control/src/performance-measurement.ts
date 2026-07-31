import type {
  PerformanceMarkAcknowledgement,
  PerformanceStage,
} from "./contracts";
import type { WorkspaceClient } from "./workspace-client";

const frontendClockId = `react:${Math.round(performance.timeOrigin * 1000)}`;

function frontendMonotonicNanoseconds(): string {
  return Math.round(performance.now() * 1_000_000).toString();
}

async function record(
  client: WorkspaceClient,
  stage: PerformanceStage,
  boundary: "start" | "end",
  clock: "native" | "frontend",
  detail: Readonly<Record<string, unknown>>,
): Promise<PerformanceMarkAcknowledgement> {
  if (!client.recordPerformanceMark) return { recorded: false };
  return client.recordPerformanceMark({
    stage,
    boundary,
    clock,
    monotonic_ns: clock === "frontend" ? frontendMonotonicNanoseconds() : "",
    clock_id: clock === "frontend" ? frontendClockId : "",
    detail,
  });
}

export function markFrontendPerformance(
  client: WorkspaceClient,
  stage: PerformanceStage,
  boundary: "start" | "end",
  detail: Readonly<Record<string, unknown>> = {},
): Promise<PerformanceMarkAcknowledgement> {
  return record(client, stage, boundary, "frontend", detail);
}

export function markNativePerformance(
  client: WorkspaceClient,
  stage: PerformanceStage,
  boundary: "start" | "end",
  detail: Readonly<Record<string, unknown>> = {},
): Promise<PerformanceMarkAcknowledgement> {
  return record(client, stage, boundary, "native", detail);
}

export function afterTwoAnimationFrames(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
}
