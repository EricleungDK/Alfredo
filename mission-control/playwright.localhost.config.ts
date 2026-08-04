import { defineConfig } from "@playwright/test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const runtimeRoot = mkdtempSync(join(tmpdir(), "alfredo-localhost-e2e-"));
const startingLocation = mkdtempSync(join(tmpdir(), "alfredo-localhost-start-"));
const repositoryRoot = resolve("..");

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/localhost-functional.pw.ts",
  timeout: 180_000,
  expect: { timeout: 90_000 },
  workers: 1,
  reporter: "line",
  outputDir: "/tmp/alfredo-localhost-playwright-results",
  use: {
    headless: true,
    serviceWorkers: "block",
  },
  webServer: {
    command: "npm run dev -- --host 127.0.0.1 --port 1421 --strictPort",
    url: "http://127.0.0.1:1421/",
    reuseExistingServer: false,
    timeout: 180_000,
    env: {
      ...process.env,
      ALBERT_BACKEND_ROOT: repositoryRoot,
      ALFREDO_AGENT_CONFIG: join(repositoryRoot, ".albert", "agents.json"),
      ALFREDO_RUNTIME_ROOT: runtimeRoot,
      ALFREDO_STARTING_LOCATION: startingLocation,
    },
  },
});
