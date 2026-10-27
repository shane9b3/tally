/**
 * Automated accessibility check (1.0 a11y pass): axe-core over the main
 * window and each major dialog. Fails on serious or critical violations.
 * The keyboard-only walk through the same dialogs is in keyboard.spec.ts;
 * the VoiceOver checklist is docs/qa/a11y.md.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

/** Rules reported as annotations but not failed (none: colour contrast is gated too). */
const REPORT_ONLY: string[] = [];

/** Serious and critical violations, formatted one per line (empty when clean). */
async function violations(page: Page, include?: string): Promise<string[]> {
  // Dialogs fade in; measuring contrast mid-animation reports blended colours.
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
  let builder = new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]);
  if (include) builder = builder.include(include);
  const result = await builder.analyze();
  const fmt = (v: (typeof result.violations)[number]) => `${v.impact} ${v.id}: ${v.help}\n${v.nodes.map((n) => `    ${n.target.join(" ")}  ${n.failureSummary?.split("\n").slice(1).join("; ") ?? ""}`).join("\n")}`;
  const serious = result.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  for (const v of serious.filter((v) => REPORT_ONLY.includes(v.id))) test.info().annotations.push({ type: "a11y (report only)", description: fmt(v) });
  return serious.filter((v) => !REPORT_ONLY.includes(v.id)).map(fmt);
}

/** Commands that open a modal dialog, by name. */
export const DIALOG_COMMANDS: readonly { name: string; command: string }[] = [
  { name: "Format Cells", command: "format.cells" },
  { name: "Page Setup", command: "file.pageSetup" },
  { name: "Options", command: "tools.options" },
  { name: "Customize Toolbars", command: "view.customizeToolbars" },
  { name: "Function Wizard", command: "insert.function" },
  { name: "Sort", command: "data.sort" },
  { name: "Standard Filter", command: "data.standardFilter" },
  { name: "Find & Replace", command: "edit.findReplace" },
  { name: "Paste Special", command: "edit.pasteSpecial" },
  { name: "Define Name", command: "data.defineName" },
  { name: "Manage Names", command: "data.manageNames" },
  { name: "Conditional formatting", command: "format.conditional" },
  { name: "Validity", command: "data.validation" },
  { name: "Hyperlink", command: "insert.hyperlink" },
  { name: "AutoFormat", command: "format.autoFormat" },
  { name: "DataPilot", command: "data.dataPilot" },
  { name: "Subtotals", command: "data.subtotals" },
  { name: "Goal Seek", command: "tools.goalSeek" },
  { name: "Properties", command: "file.properties" },
  { name: "Keyboard shortcuts", command: "help.shortcuts" },
  { name: "Text to Columns", command: "data.textToColumns" },
  { name: "Remove Duplicates", command: "data.removeDuplicates" },
  { name: "Delete Contents", command: "edit.deleteContents" },
  { name: "Solver", command: "tools.solver" },
  { name: "Scenarios", command: "tools.scenarios" },
  { name: "Consolidate", command: "data.consolidate" },
  { name: "Protect Sheet", command: "tools.protectSheet" },
  { name: "Protect Document", command: "tools.protectDocument" },
  { name: "Document Statistics", command: "file.documentStats" },
  { name: "Save as Template", command: "file.saveAsTemplate" },
  { name: "Sort Options", command: "data.sortOptions" },
  { name: "Define Database Range", command: "data.defineDatabaseRange" },
  { name: "Custom number format", command: "format.number.custom" },
];

test("the main window has no serious or critical axe violations", async ({ page }) => {
  await ready(page);
  expect(await violations(page)).toEqual([]);
});

for (const d of DIALOG_COMMANDS) {
  test(`axe: ${d.name} dialog`, async ({ page }) => {
    await ready(page);
    await run(page, d.command);
    const dialog = page.locator("[role=dialog][aria-modal=true], [role=alertdialog]").first();
    await expect(dialog).toBeVisible();
    expect(await violations(page, "[role=dialog][aria-modal=true]")).toEqual([]);
  });
}

test("axe: print preview and the chart editor", async ({ page }) => {
  await ready(page);
  await run(page, "file.printPreview");
  await expect(page.getByTestId("print-preview")).toBeVisible();
  expect(await violations(page, "[data-testid=print-preview]")).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("print-preview")).toBeHidden();

  await run(page, "insert.chart");
  const editor = page.getByTestId("chart-editor");
  await expect(editor).toBeVisible();
  expect(await violations(page, "[data-testid=chart-editor]")).toEqual([]);
});

/** Dark theme: the same checks (contrast differs most here) for the main window and a few dialogs. */
test.describe("dark theme", () => {
  test.use({ colorScheme: "dark" });
  test("axe: main window (dark)", async ({ page }) => {
    await ready(page);
    expect(await violations(page)).toEqual([]);
  });
  for (const d of DIALOG_COMMANDS.filter((d) => ["Format Cells", "Function Wizard", "Remove Duplicates", "Options", "Sort"].includes(d.name))) {
    test(`axe: ${d.name} dialog (dark)`, async ({ page }) => {
      await ready(page);
      await run(page, d.command);
      await expect(page.locator("[role=dialog][aria-modal=true], [role=alertdialog]").first()).toBeVisible();
      expect(await violations(page, "[role=dialog][aria-modal=true]")).toEqual([]);
    });
  }
});
