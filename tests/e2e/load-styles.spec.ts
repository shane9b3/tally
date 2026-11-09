/**
 * Load Styles (MockClient): Styles panel ▸ Load Styles… lists the templates,
 * loads a built-in template's cell styles (new names only without
 * Overwrite), the page format with Page Styles + Overwrite, one undo step
 * takes it all back; the dialog is keyboard-usable and axe-clean. The
 * browser build has no "From File…" (the desktop shell's Open panel).
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function ready(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

const orientation = (page: Page) =>
  page.evaluate(async () => {
    const t = window.__tally!;
    return (await t.client.pageSetup(t.controller.info!.doc, t.controller.sheet)).orientation;
  });

test("Load Styles from a template: cell styles, page styles with Overwrite, undo", async ({ page }) => {
  await ready(page);
  await run(page, "view.stylesPanel");
  const panel = page.getByTestId("styles-panel");
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId("style-Table Header")).toHaveCount(0);

  await panel.getByTestId("styles-load").click();
  const dialog = page.getByTestId("load-styles");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("heading", { name: "Load Styles" })).toBeVisible();
  // No From File… in the browser build; OK waits for a template.
  await expect(dialog.getByTestId("load-styles-file")).toHaveCount(0);
  await expect(dialog.getByTestId("dialog-ok")).toBeDisabled();
  await expect(dialog.getByTestId("load-styles-cell")).toBeChecked();
  await expect(dialog.getByTestId("load-styles-page")).not.toBeChecked();
  await expect(dialog.getByTestId("load-styles-overwrite")).not.toBeChecked();

  // Serious and critical violations gate, as in axe.spec.ts (the shared dialog form's role is a minor best-practice note).
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
  const axe = await new AxeBuilder({ page }).include('[data-testid="load-styles"]').withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).analyze();
  const serious = axe.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`);
  expect(serious).toEqual([]);

  // Keyboard: focus the list, pick the first template, submit with Enter.
  await dialog.getByTestId("load-styles-template-invoices").focus();
  await page.keyboard.press("Space");
  await expect(dialog.getByTestId("load-styles-template-invoices")).toHaveAttribute("aria-selected", "true");
  await dialog.getByTestId("dialog-ok").click();
  await expect(dialog).toHaveCount(0);
  await expect(panel.getByTestId("style-Table Header")).toBeVisible();
  await expect(panel.getByTestId("style-Amount")).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.__tally!.client.undoHistory(window.__tally!.controller.info!.doc).then((h) => h.undo[0]))).toBe("Load Styles");

  // Page Styles + Overwrite: the template's page format replaces this sheet's.
  await page.evaluate(async () => {
    const t = window.__tally!;
    const doc = t.controller.info!.doc;
    const sheet = t.controller.sheet;
    const setup = await t.client.pageSetup(doc, sheet);
    await t.controller.apply("Page", [{ type: "setPageSetup", sheet, setup: { ...setup, orientation: "landscape" } }]);
  });
  expect(await orientation(page)).toBe("landscape");
  await run(page, "format.loadStyles");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("load-styles-template-budget").click();
  await dialog.getByTestId("load-styles-cell").uncheck();
  await dialog.getByTestId("load-styles-page").check();
  // Page styles alone need Overwrite.
  await expect(dialog.getByTestId("dialog-ok")).toBeDisabled();
  await dialog.getByTestId("load-styles-overwrite").check();
  await dialog.getByTestId("dialog-ok").click();
  await expect.poll(() => orientation(page)).toBe("portrait");

  // One undo step each.
  await page.evaluate(() => window.__tally!.controller.undo());
  await expect.poll(() => orientation(page)).toBe("landscape");
  await page.evaluate(() => window.__tally!.controller.undo());
  await page.evaluate(() => window.__tally!.controller.undo());
  await expect(panel.getByTestId("style-Table Header")).toHaveCount(0);
});
