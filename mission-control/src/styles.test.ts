import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, test } from "vitest";

const css = readFileSync(resolve(process.cwd(), "src/styles.css"), "utf8");
const html = readFileSync(resolve(process.cwd(), "index.html"), "utf8");

describe("workstation visual safety rails", () => {
  test("uses offline-readable typography without micro text", () => {
    expect(css).not.toMatch(/@import\s+url/i);
    expect(css).toMatch(/font-family:\s*"Ubuntu Sans"/);
    expect(css).not.toMatch(/font-size:\s*0\.[0-6](?:\d+)?rem/);
    expect(html).toContain("<title>Alfredo Workstation</title>");
    expect(html).toContain('name="theme-color" content="#0d0f0f"');
  });

  test("bounds each pane to one scroll owner before desktop columns can collide", () => {
    expect(css).toMatch(/\.agent-workstations\s*\{[^}]*grid-template-rows:\s*auto\s+minmax\(0,\s*1fr\)/s);
    expect(css).toMatch(/\.mission-work-scroll\s*\{[^}]*overflow:\s*auto/s);
    expect(css).toMatch(/@media\s*\(max-width:\s*(?:9[6-9]\d|1\d{3,})px\)/);
    expect(css).not.toMatch(/grid-template-columns:\s*minmax\(520px/);
    expect(css).toMatch(
      /@media\s*\(max-width:\s*680px\)[\s\S]*?\.command-deck\s*\{[^}]*grid-template-rows:\s*auto\s+minmax\(0,\s*1fr\)/,
    );
    expect(css).toMatch(/\.capability-menu\s*\{[^}]*max-height:\s*min\([^;]*dvh\)/s);
    expect(css).toMatch(/\.context-inspector\s*\{[^}]*max-height:\s*min\([^;]*dvh/s);
  });

  test("lets long session and command headers shrink and wrap instead of overlapping", () => {
    expect(css).toMatch(/\.workstation-card header\s*\{[^}]*min-width:\s*0[^}]*flex-wrap:\s*wrap/s);
    expect(css).toMatch(/\.command-console-card header,[\s\S]*?\{[^}]*min-width:\s*0[^}]*flex-wrap:\s*wrap/s);
    expect(css).toMatch(/\.command-console-card header code,[\s\S]*?overflow-wrap:\s*anywhere/s);
  });

  test("keeps bounded evidence text inside the Mission Work pane", () => {
    expect(css).toMatch(/\.session-artifact-viewer\s*\{[^}]*min-width:\s*0[^}]*max-width:\s*100%/s);
    expect(css).toMatch(
      /\.session-artifact-viewer pre\s*\{[^}]*max-width:\s*100%[^}]*max-height:[^;}]+[^}]*overflow:\s*auto/s,
    );
    expect(css).toMatch(/\.session-artifact-viewer > header\s*\{[^}]*flex-wrap:\s*wrap/s);
  });
});
