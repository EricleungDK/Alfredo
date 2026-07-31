import { createRoot } from "react-dom/client";
import { App } from "../App";
import { TauriWorkspaceClient } from "../workspace-client";
import { RustOrchestratorGuiPrototype } from "./RustOrchestratorGuiPrototype";

const client = new TauriWorkspaceClient();

createRoot(document.getElementById("root")!).render(
  <RustOrchestratorGuiPrototype>
    <App client={client} />
  </RustOrchestratorGuiPrototype>,
);
