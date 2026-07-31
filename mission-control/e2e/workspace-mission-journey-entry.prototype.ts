import { expect, test } from "@playwright/test";

// Temporary regression for the throwaway Wayfinder prototype entry.
test("prototype server root opens the Mission journey without Tauri", async ({
  page,
}) => {
  await page.goto("http://127.0.0.1:14873/");

  await expect(page.getByText("Alfredo workstation unavailable")).toHaveCount(0);
  await expect(
    page.getByText("Cannot read properties of undefined (reading 'invoke')"),
  ).toHaveCount(0);
  await expect(
    page.getByText("Conversational workstation", { exact: true }),
  ).toBeVisible();
});

test("Mission Execution Tree status lights contain their labels", async ({ page }) => {
  await page.setViewportSize({ width: 947, height: 845 });
  await page.goto("http://127.0.0.1:14873/");

  const decisionStatus = page.getByText("Decision needed", { exact: true });
  await expect(decisionStatus).toBeVisible();

  const dimensions = await decisionStatus.evaluate((element) => ({
    clientHeight: element.clientHeight,
    clientWidth: element.clientWidth,
    scrollHeight: element.scrollHeight,
    scrollWidth: element.scrollWidth,
  }));

  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
  expect(dimensions.scrollHeight).toBeLessThanOrEqual(dimensions.clientHeight);
});

test("Agent Console follows a newly submitted conversation turn", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("http://127.0.0.1:14873/");

  const transcript = page.locator(".journey-transcript");
  await transcript.evaluate((element) => {
    element.scrollTop = 0;
  });

  await page.getByLabel("Message Alfredo").fill("Keep the agent status visible.");
  await page.getByRole("button", { name: "Send" }).click();

  await expect
    .poll(() =>
      transcript.evaluate(
        (element) => element.scrollHeight - element.scrollTop - element.clientHeight,
      ),
    )
    .toBeLessThanOrEqual(1);
});

test("Agent Console provides shell history and prompt completion", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("http://127.0.0.1:14873/");

  const composer = page.getByLabel("Message Alfredo");
  await composer.fill("first local prompt");
  await composer.press("Enter");
  await composer.fill("second local prompt");
  await composer.press("Enter");
  await composer.fill("unsent draft");

  await composer.press("ArrowUp");
  await expect(composer).toHaveValue("second local prompt");
  await composer.press("ArrowUp");
  await expect(composer).toHaveValue("first local prompt");
  await composer.press("ArrowDown");
  await expect(composer).toHaveValue("second local prompt");
  await composer.press("ArrowDown");
  await expect(composer).toHaveValue("unsent draft");

  await composer.fill("/st");
  await expect(page.getByRole("option", { name: /\/status/ })).toBeVisible();
  await composer.press("Enter");
  await expect(composer).toHaveValue("/status ");
  await composer.press("Enter");
  await expect(
    page.getByText(/Mission Work on the right is the canonical live state/),
  ).toBeVisible();

  await composer.fill("Ask @way");
  await expect(page.getByRole("option", { name: /@wayfinder/ })).toBeVisible();
  await composer.press("Tab");
  await expect(composer).toHaveValue("Ask @wayfinder ");
});
