import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { configDefaults } from "vitest/config";

import { alfredoLocalhostBridgePlugin } from "./dev/localhost-bridge-plugin";

export default defineConfig(({ mode }) => {
  const isAppleContainer = mode === "apple-container";

  return {
    plugins: [
      react(),
      (mode === "localhost" || isAppleContainer) &&
        alfredoLocalhostBridgePlugin({
          allowAppleContainerPortForwarding: isAppleContainer,
        }),
      mode === "workspace-mission-journey" && {
        name: "workspace-mission-journey-prototype-entry",
        transformIndexHtml(html) {
          return html.replace(
            "/src/main.tsx",
            "/src/prototypes/workspace-mission-journey-main.tsx",
          );
        },
      },
      mode === "mission-execution-tree" && {
        name: "mission-execution-tree-prototype-entry",
        transformIndexHtml(html) {
          return html.replace(
            "/src/main.tsx",
            "/src/prototypes/mission-execution-tree-main.tsx",
          );
        },
      },
      mode === "rust-orchestrator-gui" && {
        name: "rust-orchestrator-gui-prototype-entry",
        transformIndexHtml(html) {
          return html.replace(
            "/src/main.tsx",
            "/src/prototypes/rust-orchestrator-main.tsx",
          );
        },
      },
    ],
    clearScreen: false,
    server: {
      host: isAppleContainer
        ? "0.0.0.0"
        : mode === "localhost" || mode === "tauri"
          ? "127.0.0.1"
          : undefined,
      port: mode === "tauri" ? 1422 : 1420,
      strictPort: true,
    },
    test: {
      environment: "jsdom",
      exclude: [...configDefaults.exclude, "tests/performance-*.test.js"],
      // App.test.tsx contains a large asynchronous UI matrix. Serial file
      // execution keeps it from competing with other jsdom workers and makes
      // baseline/post-change verification comparable on bounded cloud hosts.
      fileParallelism: false,
      globals: true,
      setupFiles: "./src/test-setup.ts",
    },
  };
});
