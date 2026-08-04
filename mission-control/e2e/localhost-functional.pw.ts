import { expect, test } from "@playwright/test";
import { join } from "node:path";

test("localhost opens a functional Alfredo workstation", async ({ page }) => {
  const pageErrors: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.setViewportSize({ width: 640, height: 900 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("http://127.0.0.1:1421/");

  expect(await page.evaluate(() => "__TAURI_INTERNALS__" in window)).toBe(false);
  await expect(page.getByText("Alfredo workstation unavailable")).toHaveCount(0);
  await expect(page.getByRole("main", { name: "Agent Console" })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Choose or create a repository" }),
  ).toBeVisible();

  const startingLocation = await page.getByRole("banner").locator("strong").textContent();
  expect(startingLocation).toBeTruthy();
  const workspacePath = join(startingLocation!, "workspace");
  await page.getByLabel("Coding Workspace path").fill(workspacePath);
  const createRepository = page.getByRole("button", { name: "Create new repository" });
  await createRepository.focus();
  await expect(createRepository).toBeFocused();
  await createRepository.press("Enter");
  await expect(
    page.getByRole("heading", { name: "Mission selection required" }),
  ).toBeVisible();
  await page.getByLabel("New Mission title").fill("Localhost E2E");
  const startMission = page.getByRole("button", { name: "Start New Mission" });
  await startMission.focus();
  await expect(startMission).toBeFocused();
  await startMission.press("Enter");

  await expect(page.getByText("Alfredo workstation unavailable")).toHaveCount(0);
  await expect(page.getByRole("main", { name: "Agent Console" })).toBeVisible();
  await expect(page.getByText("Agent Console / localhost-e2e", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Message Alfredo")).toBeVisible();
  await expect(page.locator("body")).toHaveCSS("scroll-behavior", "auto");
  expect(pageErrors).toEqual([]);
});
