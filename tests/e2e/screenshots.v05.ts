/**
 * v0.5 (W5-6) screenshots for docs/qa (not part of `pnpm e2e`). Run:
 *
 *   pnpm --filter tally-desktop exec playwright test -c ../../tests/e2e/screenshots.v05.config.ts
 *
 * Writes docs/qa/screenshots/v05-*.png against the browser build.
 */
import { expect, test, type Page } from "@playwright/test";

const OUT = process.env.TALLY_SHOTS ?? "../../docs/qa/screenshots";

async function ready(page: Page) {
  await page.goto("/?template=invoices");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(150);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

test("DataPilot dialog", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.__tally!.controller.grid!.setSelection({ range: { r0: 2, c0: 1, r1: 2, c1: 1 }, active: { r: 2, c: 1 }, anchor: { r: 2, c: 1 } }, true));
  await page.evaluate(() => window.__tally!.controller.onSelectionChange({ range: { r0: 2, c0: 1, r1: 2, c1: 1 }, active: { r: 2, c: 1 }, anchor: { r: 2, c: 1 } }));
  await run(page, "data.dataPilot");
  const dlg = page.getByTestId("datapilot-dialog");
  await dlg.getByTestId("pivot-add-row-Customer").click();
  await dlg.getByTestId("pivot-add-col-Status").click();
  await dlg.getByTestId("pivot-add-data-Total").click();
  await dlg.getByTestId("pivot-field-Sum - Total").click();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${OUT}/v05-datapilot-dialog.png` });
});

test("Styles panel", async ({ page }) => {
  await ready(page);
  await run(page, "view.stylesPanel");
  await expect(page.getByTestId("styles-panel")).toBeVisible();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${OUT}/v05-styles-panel.png` });
});

test("Format Cells dialog", async ({ page }) => {
  await ready(page);
  await run(page, "format.cells");
  const dlg = page.getByTestId("format-cells");
  await dlg.getByTestId("tab-borders").click();
  await dlg.getByTestId("cells-borders-outer").click();
  await dlg.getByTestId("cells-border-diagonalDown").click();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${OUT}/v05-format-cells.png` });
});

test("Options dialog", async ({ page }) => {
  await ready(page);
  await run(page, "tools.options");
  await expect(page.getByTestId("options-dialog")).toBeVisible();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${OUT}/v05-options.png` });
});

test("Navigator", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.__tally!.controller.defineName("Totals", null, "$Sheet1.$F$2:$F$15", null));
  await run(page, "view.navigator");
  await expect(page.getByTestId("navigator-panel")).toContainText("Totals");
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${OUT}/v05-navigator.png` });
});

test("German chrome", async ({ page }) => {
  await ready(page);
  await run(page, "tools.options");
  const dlg = page.getByTestId("options-dialog");
  await dlg.getByTestId("tab-language").click();
  await dlg.getByTestId("opt-ui-language").selectOption("de");
  await dlg.getByTestId("dialog-ok").click();
  await expect(page.getByRole("button", { name: "Rückgängig" })).toBeVisible();
  await page.keyboard.press("Meta+k");
  await page.getByTestId("palette-input").fill("Datei");
  await page.waitForTimeout(250);
  await page.screenshot({ path: `${OUT}/v05-german-chrome.png` });
});
