import { expect, test, type Locator, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { extname, resolve } from "node:path";

const workspacePath =
  "/workspace/a-very-long-project-name-that-must-remain-readable-without-overlapping-any-controls";
const reviewSessionId = "session-ISS-RESPONSIVE-03-10";
const reviewAgentName = "qwen2.5-coder-review-agent-with-a-long-readable-name";
const evidenceArtifactRef = `app-local://evidence/${reviewSessionId}`;

const snapshot = {
  schema_version: 1,
  revision: 4,
  workspace_session: {
    id: "workspace-responsive-layout",
    workspace_path: workspacePath,
    status: "ready",
  },
  active_mission: {
    id: "responsive-layout-mission",
    title: "Responsive Local Coding Agent Workstation",
    issue_count: 3,
  },
  conversation_scope: {
    kind: "working-directory",
    target_id: workspacePath,
    label: "responsive-layout-workspace",
  },
  operations_view: "mission-board",
  mission_board: {
    prd_title: "Responsive Local Coding Agent Workstation",
    issue_count: 3,
    ordered_issue_ids: ["ISS-RESPONSIVE-01", "ISS-RESPONSIVE-02", "ISS-RESPONSIVE-03"],
    ready_issue_ids: ["ISS-RESPONSIVE-02"],
    approved_issue_ids: ["ISS-RESPONSIVE-01"],
    issue_slices: [
      {
        issue_id: "ISS-RESPONSIVE-01",
        title: "Keep a long-running subagent status readable at every supported viewport",
        tracker_status: "in-progress",
        lifecycle: "Active",
        progress: "Local Agent is streaming a deliberately long implementation status without clipping",
        launch_eligible: false,
        blockers: [],
        accepted_boundary: {
          what_to_build: "Verify the responsive workstation layout.",
          acceptance_criteria: ["No panels or controls overlap."],
          evidence_requirements: ["Chromium geometry checks pass."],
          source_path: ".scratch/issues/ISS-RESPONSIVE-01.md",
        },
        sessions: [
          {
            session_id: "session-ISS-RESPONSIVE-01-10",
            assigned_agent: "gemma4-26b-local-agent-with-a-long-readable-name",
            role: "local-agent",
            provider: "ollama",
            model: "gemma4:26b",
            status: "running",
            stale: false,
            disconnected: false,
            operation_status: "streaming",
            failure: "",
          },
        ],
        provenance: { role: "local-agent", provider: "ollama", model: "gemma4:26b" },
        model_assignment: {
          agent_id: "gemma4-26b-local-agent-with-a-long-readable-name",
          role: "local-agent",
          provider: "ollama",
          model: "gemma4:26b",
          availability: "available",
          availability_reason: "",
          operation_status: "streaming",
          failure: "",
        },
        evidence: {
          state: "missing",
          changed_files: [],
          commands_run: [],
          test_results: "Evidence is not ready yet.",
          risks: "None recorded.",
          artifact_links: [],
        },
        working_context_sources: [],
      },
      {
        issue_id: "ISS-RESPONSIVE-02",
        title: "A workable ticket remains assignable from the Mission Work pane",
        tracker_status: "ready-for-agent",
        lifecycle: "Ready",
        progress: "Ready for assignment",
        launch_eligible: true,
        blockers: [],
        accepted_boundary: {
          what_to_build: "Keep a ticket actionable.",
          acceptance_criteria: ["Assignment actions remain visible."],
          evidence_requirements: ["Rendered control is reachable."],
          source_path: ".scratch/issues/ISS-RESPONSIVE-02.md",
        },
        sessions: [],
        provenance: { role: "local-agent", provider: "ollama", model: "gemma4:12b" },
        model_assignment: {
          agent_id: "",
          role: "local-agent",
          provider: "ollama",
          model: "gemma4:12b",
          availability: "available",
          availability_reason: "",
          operation_status: "idle",
          failure: "",
        },
        evidence: {
          state: "missing",
          changed_files: [],
          commands_run: [],
          test_results: "No evidence package recorded.",
          risks: "None recorded.",
          artifact_links: [],
        },
        working_context_sources: [],
      },
      {
        issue_id: "ISS-RESPONSIVE-03",
        title: "Review a bounded evidence package without leaving the workstation",
        tracker_status: "in-review",
        lifecycle: "Review Ready",
        progress: "Validated evidence is ready for a Mission Commander decision",
        launch_eligible: false,
        blockers: [],
        accepted_boundary: {
          what_to_build: "Keep review actions and bounded evidence readable.",
          acceptance_criteria: ["Review controls and artifact content remain reachable."],
          evidence_requirements: ["The registered artifact opens in the inline viewer."],
          source_path: ".scratch/issues/ISS-RESPONSIVE-03.md",
        },
        sessions: [
          {
            session_id: reviewSessionId,
            assigned_agent: reviewAgentName,
            role: "local-agent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
            status: "evidence-ready",
            stale: false,
            disconnected: false,
            operation_status: "evidence-ready",
            failure: "",
          },
        ],
        provenance: { role: "local-agent", provider: "ollama", model: "qwen2.5-coder:14b" },
        model_assignment: {
          agent_id: reviewAgentName,
          role: "local-agent",
          provider: "ollama",
          model: "qwen2.5-coder:14b",
          availability: "available",
          availability_reason: "",
          operation_status: "evidence-ready",
          failure: "",
        },
        evidence: {
          state: "ready",
          changed_files: [
            "mission-control/src/a-very-long-review-artifact-component-filename.tsx",
          ],
          commands_run: ["npm test -- --run responsive review geometry"],
          test_results: "Review geometry and bounded evidence validation passed.",
          risks: "Mission Commander review is still required.",
          artifact_links: [evidenceArtifactRef],
        },
        working_context_sources: [],
      },
    ],
  },
  missions: [
    {
      id: "responsive-layout-mission",
      title: "Responsive Local Coding Agent Workstation",
      issue_count: 3,
      is_active: true,
      sessions: [
        {
          session_id: "session-ISS-RESPONSIVE-01-10",
          issue_id: "ISS-RESPONSIVE-01",
          assigned_agent: "gemma4-26b-local-agent-with-a-long-readable-name",
          status: "running",
          role: "local-agent",
          provider: "ollama",
          model: "gemma4:26b",
          task_title: "Keep a long-running subagent status readable at every supported viewport",
          operation_status: "streaming",
          failure: "",
          changed_files: ["mission-control/src/a-very-long-component-filename-for-overflow-testing.tsx"],
          commands_run: ["npm test -- --run"],
          test_results: "Tests are still running.",
          risks: "None recorded.",
          artifact_links: [],
        },
        {
          session_id: reviewSessionId,
          issue_id: "ISS-RESPONSIVE-03",
          assigned_agent: reviewAgentName,
          status: "evidence-ready",
          role: "local-agent",
          provider: "ollama",
          model: "qwen2.5-coder:14b",
          task_title: "Review a bounded evidence package without leaving the workstation",
          operation_status: "evidence-ready",
          failure: "",
          changed_files: [
            "mission-control/src/a-very-long-review-artifact-component-filename.tsx",
          ],
          commands_run: ["npm test -- --run responsive review geometry"],
          test_results: "Review geometry and bounded evidence validation passed.",
          risks: "Mission Commander review is still required.",
          artifact_links: [evidenceArtifactRef],
        },
      ],
      attention: [],
    },
  ],
};

const history = {
  schema_version: 1,
  messages: [
    {
      message_id: "console-layout-000001",
      sequence: 1,
      role: "user",
      content:
        "Please investigate and fix the responsive workstation while keeping this deliberately long prompt readable.",
      scope: snapshot.conversation_scope,
      outcome: "proposed",
      source: "mission-commander",
    },
    {
      message_id: "console-layout-000002",
      sequence: 2,
      role: "assistant",
      content:
        "The task is delegated and remains visible beside the conversation without replacing the project discussion.",
      scope: snapshot.conversation_scope,
      outcome: "model-commentary",
      source: "frontier-model",
    },
  ],
};

async function installTauriFixture(page: Page): Promise<void> {
  await page.addInitScript(
    ({ fixtureSnapshot, fixtureHistory }) => {
      const responses: Record<string, unknown> = {
        performance_mark: { recorded: false },
        alfredo_launch_context: {
          schema_version: 1,
          selected_agent: "qwen3-14b",
          selected_model: "qwen3:14b",
          starting_location: "/workspace",
          coding_workspace: fixtureSnapshot.workspace_session.workspace_path,
          active_mission: fixtureSnapshot.active_mission.id,
          phase: "workspace-ready",
          runtime_root: "/tmp/alfredo-responsive-layout",
          recent_workspaces: [fixtureSnapshot.workspace_session.workspace_path],
        },
        agent_capabilities: {
          schema_version: 1,
          default_agent_id: "qwen3-14b",
          commands: [
            { name: "/help", usage: "/help", description: "Show commands.", category: "discovery" },
            { name: "/task", usage: "/task <request>", description: "Delegate coding work.", category: "execution" },
          ],
          skills: [
            {
              name: "diagnosing-bugs",
              description: "Reproduce and isolate hard bugs.",
              source: "/skills/diagnosing-bugs/SKILL.md",
              invocation: "/use diagnosing-bugs",
            },
          ],
          agents: [
            {
              id: "qwen3-14b",
              role: "frontier",
              provider: "ollama",
              runner: "ollama",
              model: "qwen3:14b",
              routing: "controller",
              availability: "available",
              availability_reason: "",
              assignable: false,
              delegate_only: false,
              requires_approval: false,
            },
            {
              id: "gemma4-12b",
              role: "local-agent",
              provider: "ollama",
              runner: "ollama",
              model: "gemma4:12b",
              routing: "worker",
              availability: "available",
              availability_reason: "",
              assignable: true,
              delegate_only: false,
              requires_approval: false,
            },
          ],
        },
        workspace_snapshot: fixtureSnapshot,
        agent_console_history: fixtureHistory,
        working_context: {
          schema_version: 1,
          revision: 1,
          scope: fixtureSnapshot.conversation_scope,
          sources: [],
          content_character_count: 0,
        },
        shell_terminal: {
          schema_version: 1,
          revision: 1,
          commands: [],
          grants: [],
          grant_denials: [],
        },
        session_artifact: {
          schema_version: 1,
          mission_id: fixtureSnapshot.active_mission.id,
          session_id: fixtureSnapshot.missions[0].sessions[1].session_id,
          artifact_id: "responsive-review-evidence",
          label: "Evidence Package",
          media_type: "application/json",
          content:
            '{"evidence_valid":true,"summary":"Bounded production geometry is readable without exposing a raw local path."}',
          byte_count: 112,
          content_limit_bytes: 128000,
          truncated: false,
        },
      };
      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: true,
        value: {
          invoke: async (command: string, args?: { afterRevision?: number }) => {
            if (command === "workspace_updates") {
              return {
                after_revision: args?.afterRevision ?? fixtureSnapshot.revision,
                current_revision: fixtureSnapshot.revision,
                events: [],
              };
            }
            if (!(command in responses)) throw new Error(`Unexpected fixture command: ${command}`);
            return structuredClone(responses[command]);
          },
        },
      });
    },
    { fixtureSnapshot: snapshot, fixtureHistory: history },
  );
}

async function serveProductionBundle(page: Page): Promise<void> {
  await page.route("http://alfredo.test/**", async (route) => {
    const url = new URL(route.request().url());
    const relativePath = url.pathname === "/" ? "index.html" : url.pathname.slice(1);
    if (!relativePath || relativePath.includes("..")) {
      await route.fulfill({ status: 404, body: "Not found" });
      return;
    }
    const contentTypes: Record<string, string> = {
      ".css": "text/css; charset=utf-8",
      ".html": "text/html; charset=utf-8",
      ".js": "text/javascript; charset=utf-8",
      ".woff2": "font/woff2",
    };
    try {
      const body = readFileSync(resolve(process.cwd(), "dist", relativePath));
      await route.fulfill({
        status: 200,
        body,
        contentType: contentTypes[extname(relativePath)] ?? "application/octet-stream",
      });
    } catch {
      await route.fulfill({ status: 404, body: "Not found" });
    }
  });
}

function rectanglesOverlap(
  first: { x: number; y: number; width: number; height: number },
  second: { x: number; y: number; width: number; height: number },
): boolean {
  return !(
    first.x + first.width <= second.x + 0.5 ||
    second.x + second.width <= first.x + 0.5 ||
    first.y + first.height <= second.y + 0.5 ||
    second.y + second.height <= first.y + 0.5
  );
}

async function expectNoHorizontalPageOverflow(page: Page): Promise<void> {
  const overflow = await page.evaluate(() => {
    const viewportWidth = document.documentElement.clientWidth;
    const offenders = Array.from(document.querySelectorAll<HTMLElement>("body *"))
      .map((element) => {
        const box = element.getBoundingClientRect();
        return {
          element: `${element.tagName.toLowerCase()}${element.className ? `.${String(element.className).trim().replace(/\s+/g, ".")}` : ""}`,
          left: Math.round(box.left),
          right: Math.round(box.right),
          width: Math.round(box.width),
          scrollWidth: element.scrollWidth,
          clientWidth: element.clientWidth,
        };
      })
      .filter(({ left, right }) => left < -1 || right > viewportWidth + 1)
      .sort((first, second) => second.right - first.right)
      .slice(0, 24);
    return {
      document: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      body: document.body.scrollWidth - document.body.clientWidth,
      offenders,
    };
  });
  expect(overflow.document, JSON.stringify(overflow.offenders, null, 2)).toBeLessThanOrEqual(1);
  expect(overflow.body, JSON.stringify(overflow.offenders, null, 2)).toBeLessThanOrEqual(1);
}

async function expectHorizontalViewportContainment(
  locator: Locator,
  viewportWidth: number,
): Promise<void> {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(-0.5);
  expect(box!.x + box!.width).toBeLessThanOrEqual(viewportWidth + 0.5);
}

async function expectFullViewportContainment(
  locator: Locator,
  viewport: { width: number; height: number },
): Promise<void> {
  await locator.scrollIntoViewIfNeeded();
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(-0.5);
  expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width + 0.5);
  expect(box!.y).toBeGreaterThanOrEqual(-0.5);
  expect(box!.y + box!.height).toBeLessThanOrEqual(viewport.height + 0.5);
}

async function expectLocatorsNotToOverlap(first: Locator, second: Locator): Promise<void> {
  const firstBox = await first.boundingBox();
  const secondBox = await second.boundingBox();
  expect(firstBox).not.toBeNull();
  expect(secondBox).not.toBeNull();
  expect(rectanglesOverlap(firstBox!, secondBox!)).toBe(false);
}

async function expectVisibleControlsNotToOverlap(scope: Locator): Promise<void> {
  const overlappingControls = await scope
    .locator("button, input, textarea, select")
    .evaluateAll((elements) => {
      const controls = elements
        .filter((element) => {
          const style = getComputedStyle(element);
          const box = element.getBoundingClientRect();
          return (
            style.display !== "none" &&
            style.visibility !== "hidden" &&
            box.width > 0 &&
            box.height > 0
          );
        })
        .map((element) => {
          const box = element.getBoundingClientRect();
          return {
            label:
              element.getAttribute("aria-label") ||
              element.textContent?.trim().replace(/\s+/g, " ") ||
              element.tagName.toLowerCase(),
            box: {
              left: box.left,
              right: box.right,
              top: box.top,
              bottom: box.bottom,
            },
          };
        });
      const overlaps: string[] = [];
      for (let firstIndex = 0; firstIndex < controls.length; firstIndex += 1) {
        const first = controls[firstIndex];
        for (let secondIndex = firstIndex + 1; secondIndex < controls.length; secondIndex += 1) {
          const second = controls[secondIndex];
          const separated =
            first.box.right <= second.box.left + 0.5 ||
            second.box.right <= first.box.left + 0.5 ||
            first.box.bottom <= second.box.top + 0.5 ||
            second.box.bottom <= first.box.top + 0.5;
          if (!separated) overlaps.push(`${first.label} <> ${second.label}`);
        }
      }
      return overlaps;
    });
  expect(overlappingControls).toEqual([]);
}

for (const viewport of [
  { name: "desktop", width: 1440, height: 900 },
  { name: "compact-desktop", width: 1100, height: 760 },
  { name: "tablet", width: 820, height: 900 },
  { name: "mobile", width: 390, height: 844 },
]) {
  test(`${viewport.name} has no panel or control overlap`, async ({ page }) => {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await serveProductionBundle(page);
    await installTauriFixture(page);
    await page.goto("http://alfredo.test/");
    await expect(page.getByRole("main", { name: "Prompt Workstation" })).toBeVisible();
    await expect(page.getByRole("complementary", { name: "Mission Work" })).toBeVisible();
    await expect(page.getByRole("table", { name: "Issue Assignment Board" })).toBeVisible();

    await expectNoHorizontalPageOverflow(page);

    const topbar = await page.locator(".topbar").boundingBox();
    const deck = await page.locator(".deck-grid").boundingBox();
    const prompt = await page.locator(".prompt-workspace").boundingBox();
    const missionWork = await page.locator(".agent-workstations").boundingBox();
    const transcript = await page.getByRole("region", { name: "Prompt Transcript" }).boundingBox();
    const composer = await page.locator(".composer").first().boundingBox();
    const textarea = await page.getByRole("textbox", { name: "Message Alfredo" }).boundingBox();
    const send = await page.getByRole("button", { name: "Send prompt" }).boundingBox();
    for (const box of [topbar, deck, prompt, missionWork, transcript, composer, textarea, send]) {
      expect(box).not.toBeNull();
      expect(box!.x).toBeGreaterThanOrEqual(-0.5);
      expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width + 0.5);
    }

    expect(rectanglesOverlap(topbar!, deck!)).toBe(false);
    expect(rectanglesOverlap(transcript!, composer!)).toBe(false);
    expect(rectanglesOverlap(textarea!, send!)).toBe(false);
    expect(rectanglesOverlap(prompt!, missionWork!)).toBe(false);
    if (viewport.width > 1040) {
      expect(prompt!.x + prompt!.width).toBeLessThanOrEqual(missionWork!.x + 1);
      const independentScroll = await page.evaluate(() => ({
        transcript: getComputedStyle(document.querySelector(".console-history")!).overflowY,
        mission: getComputedStyle(document.querySelector(".mission-work-scroll")!).overflowY,
        body: getComputedStyle(document.body).overflow,
      }));
      expect(independentScroll.transcript).toBe("auto");
      expect(independentScroll.mission).toBe("auto");
      expect(independentScroll.body).toBe("hidden");
    } else {
      expect(prompt!.y + prompt!.height).toBeLessThanOrEqual(missionWork!.y + 1);
    }

    const promptToolbar = page.locator(".prompt-toolbar");
    await page.getByRole("button", { name: "Browse commands and skills" }).click();
    const capabilityMenu = page.getByRole("region", { name: "Commands and skills" });
    await expect(capabilityMenu).toBeVisible();
    await expectFullViewportContainment(capabilityMenu, viewport);
    await expectLocatorsNotToOverlap(capabilityMenu, promptToolbar);
    await expectVisibleControlsNotToOverlap(capabilityMenu);
    await expectNoHorizontalPageOverflow(page);
    await page.getByRole("button", { name: "Close commands and skills" }).click();
    await expect(capabilityMenu).toBeHidden();

    const reviewCard = page.getByRole("article", {
      name: `${reviewAgentName} workstation card`,
    });
    const acceptReview = reviewCard.getByRole("button", {
      name: `Accept evidence ${reviewSessionId}`,
    });
    await acceptReview.scrollIntoViewIfNeeded();
    await expect(acceptReview).toBeVisible();
    await expect(acceptReview).toBeEnabled();
    await expectHorizontalViewportContainment(reviewCard, viewport.width);
    await expectVisibleControlsNotToOverlap(reviewCard);
    await expectNoHorizontalPageOverflow(page);

    await reviewCard.getByRole("button", { name: `Expand ${reviewAgentName}` }).click();
    const operationalDetail = reviewCard.getByRole("region", {
      name: `${reviewAgentName} operational detail`,
    });
    await expect(operationalDetail).toBeVisible();
    await expectHorizontalViewportContainment(operationalDetail, viewport.width);
    await expectVisibleControlsNotToOverlap(operationalDetail);
    await expectNoHorizontalPageOverflow(page);

    await operationalDetail
      .getByRole("button", { name: `Open evidence Evidence Package ${reviewSessionId}` })
      .click();
    const evidenceViewer = page.getByRole("region", { name: "Session evidence viewer" });
    await expect(evidenceViewer).toBeVisible();
    await expect(evidenceViewer.getByLabel("Evidence Package content")).toContainText(
      '"evidence_valid":true',
    );
    await expectFullViewportContainment(evidenceViewer, viewport);
    await expectLocatorsNotToOverlap(
      evidenceViewer.getByRole("heading", { name: "Evidence Package" }),
      evidenceViewer.getByRole("button", { name: "Close session evidence viewer" }),
    );
    await expectHorizontalViewportContainment(
      evidenceViewer.getByLabel("Evidence Package content"),
      viewport.width,
    );
    await expectVisibleControlsNotToOverlap(evidenceViewer);
    await expectNoHorizontalPageOverflow(page);

    const font = await page.locator("body").evaluate((element) => getComputedStyle(element).fontFamily);
    expect(font).toContain("Ubuntu Sans");
  });
}
