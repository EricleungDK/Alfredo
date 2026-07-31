import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { configDefaults } from "vitest/config";

export default defineConfig(({ mode }) => ({
  plugins: [
    react(),
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
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: "jsdom",
    exclude: [...configDefaults.exclude, "tests/performance-*.test.js"],
    globals: true,
    setupFiles: "./src/test-setup.ts",
  },
}));
