import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { MissionExecutionTreePrototype } from "./MissionExecutionTreePrototype";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <MissionExecutionTreePrototype />
  </StrictMode>,
);
