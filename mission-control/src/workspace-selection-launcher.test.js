import { readFileSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");
const packageJson = JSON.parse(readFileSync(resolve(projectRoot, "package.json"), "utf8"));
const alfredoBinPath = resolve(projectRoot, packageJson.bin.alfredo);

test("launcher treats the invocation directory only as a Starting Location", () => {
  const startingLocation = mkdtempSync(resolve(tmpdir(), "alfredo-starting-location-"));
  const canonicalStartingLocation = realpathSync(startingLocation);

  try {
    const result = spawnSync(
      process.execPath,
      [alfredoBinPath, "--agent", "qwen3.6-27b"],
      {
        cwd: startingLocation,
        encoding: "utf8",
        env: {
          ...process.env,
          ALFREDO_DESKTOP_DRY_RUN: "1",
        },
      },
    );

    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    const plan = JSON.parse(result.stdout);
    expect(plan.project_root).toBe(projectRoot);
    expect(plan.backend_root).toBe(repositoryRoot);
    expect(plan.starting_location).toBe(canonicalStartingLocation);
    expect(plan.workspace_selection).toEqual({
      schema_version: 1,
      phase: "selection-required",
      starting_location: canonicalStartingLocation,
      coding_workspace: null,
      active_mission: null,
    });
    expect(plan.recent_workspaces).toEqual([]);
    expect(plan).not.toHaveProperty("selected_workspace");
    expect(plan).not.toHaveProperty("tracker_dir");
    expect(plan).not.toHaveProperty("issues_dir");
    expect(plan).not.toHaveProperty("mission_id");
  } finally {
    rmSync(startingLocation, { recursive: true, force: true });
  }
});

test("launcher relocates an Alfredo-root Starting Location before exposing workspace selection", () => {
  const result = spawnSync(
    process.execPath,
    [alfredoBinPath, "--agent", "qwen3.6-27b"],
    {
      cwd: repositoryRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        ALFREDO_DESKTOP_DRY_RUN: "1",
        ALFREDO_STARTING_LOCATION: "",
      },
    },
  );

  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  const plan = JSON.parse(result.stdout);
  expect(plan.starting_location).toBe(dirname(repositoryRoot));
  expect(plan.workspace_selection.starting_location).toBe(dirname(repositoryRoot));
});
