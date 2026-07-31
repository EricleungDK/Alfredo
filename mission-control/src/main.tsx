import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import {
  markFrontendPerformance,
  markNativePerformance,
} from "./performance-measurement";
import { TauriWorkspaceClient } from "./workspace-client";

const client = new TauriWorkspaceClient();
void markNativePerformance(client, "S2", "end", { outcome: "pass" });
void markFrontendPerformance(client, "S3", "start", { outcome: "pass" });

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App client={client} />
  </StrictMode>,
);
