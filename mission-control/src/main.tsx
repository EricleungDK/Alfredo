import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { TauriWorkspaceClient } from "./workspace-client";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App client={new TauriWorkspaceClient()} />
  </StrictMode>,
);
