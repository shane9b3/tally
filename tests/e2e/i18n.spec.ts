/**
 * W5-6E: switching the UI language to German relabels the chrome ("Datei",
 * "Bearbeiten" in the command palette groups, "Rückgängig" on the toolbar),
 * the desktop shell is asked to rebuild the native menu (recorded by the
 * mock), and a German document locale shows localized function names in the
 * formula bar while the engine receives the canonical ones.
 */
import { expect, test, type Page } from "@playwright/test";

async function ready(page: Page) {
  await page.goto("/?template=invoices");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

test("language switch to German shows Datei / Bearbeiten", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("Meta+,");
  const dlg = page.getByTestId("options-dialog");
  await dlg.getByTestId("tab-language").click();
  await dlg.getByTestId("opt-ui-language").selectOption("de");
  await dlg.getByTestId("dialog-ok").click();
  await expect(page.getByRole("button", { name: "Rückgängig" })).toBeVisible();
  await page.keyboard.press("Meta+k");
  await page.getByTestId("palette-input").fill("Speichern");
  const palette = page.locator('[cmdk-list]');
  await expect(palette).toContainText("Datei");
  await expect(palette).toContainText("Speichern");
  await page.getByTestId("palette-input").fill("Rückgängig");
  await expect(palette).toContainText("Bearbeiten");
  await page.keyboard.press("Escape");
  const calls = await page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recordedCalls: unknown[] }).recordedCalls)) as { type: string; lang?: string }[]);
  expect(calls.some((c) => c.type === "setUiLanguage" && c.lang === "de")).toBe(true);
  // Back to English.
  await page.keyboard.press("Meta+,");
  await dlg.getByTestId("tab-language").click();
  await dlg.getByTestId("opt-ui-language").selectOption("en");
  await dlg.getByTestId("dialog-ok").click();
  await expect(page.getByRole("button", { name: "Undo" })).toBeVisible();
});

test("German document locale shows SUMME in the editor and commits SUM", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.__tally!.controller.v3.setLocale("de-DE"));
  await page.evaluate(() => window.__tally!.controller.startEdit({ r: 20, c: 5 }, "=SUMME(F2:F3)", "grid"));
  await page.evaluate(() => window.__tally!.controller.commitEdit("none"));
  const last = await page.evaluate(() => {
    const ops = (window.__tally!.client as unknown as { recorded: { type: string; input?: string }[] }).recorded;
    return ops[ops.length - 1];
  });
  expect(last).toMatchObject({ type: "setCellInput", input: "=SUM(F2:F3)" });
  await page.evaluate(() => window.__tally!.controller.grid!.setSelection({ range: { r0: 20, c0: 5, r1: 20, c1: 5 }, active: { r: 20, c: 5 }, anchor: { r: 20, c: 5 } }));
  await page.evaluate(() => window.__tally!.controller.onSelectionChange({ range: { r0: 20, c0: 5, r1: 20, c1: 5 }, active: { r: 20, c: 5 }, anchor: { r: 20, c: 5 } }));
  await expect(page.getByTestId("formula-input")).toHaveValue(/SUMME/);
});
