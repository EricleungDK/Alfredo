import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { WorkspaceMissionJourneyPrototype } from "./WorkspaceMissionJourneyPrototype";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <WorkspaceMissionJourneyPrototype />
  </StrictMode>,
);
