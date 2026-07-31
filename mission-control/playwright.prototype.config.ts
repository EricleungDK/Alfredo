import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.prototype.ts",
  timeout: 30_000,
  expect: { timeout: 8_000 },
  workers: 1,
  reporter: "line",
  outputDir: "/tmp/alfredo-prototype-playwright-results",
  use: {
    headless: true,
    serviceWorkers: "block",
  },
  webServer: {
    command: "npm run prototype:journey",
    url: "http://127.0.0.1:14873/",
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
