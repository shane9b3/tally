/**
 * View ▸ Toolbars / Visible Buttons in the browser build (prefs in
 * localStorage): hide Sort through the toolbar's context menu, a reload
 * keeps it hidden, Customize Toolbars ▸ Reset to Default brings it back;
 * whole bars toggle from the palette commands; a narrow window moves
 * buttons into "»".
 */
import { expect, test, type Page } from "@playwright/test";

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

test("hide Sort from the toolbar's context menu; reload keeps it; Reset to Default restores", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => localStorage.removeItem("tally.prefs"));
  await ready(page);
  const sort = page.getByTestId("toolbar-row").locator('[data-command="data.sort"]');
  await expect(sort).toBeVisible();
  await page.getByTestId("toolbar-row").click({ button: "right", position: { x: 5, y: 5 } });
  const menu = page.getByTestId("bar-menu");
  await expect(menu).toBeVisible();
  await menu.locator('[data-item="visible"]').click();
  const visible = page.getByTestId("visible-buttons");
  await expect(visible.locator('[data-item="sort"]')).toHaveAttribute("aria-checked", "true");
  await visible.locator('[data-item="sort"]').click();
  await expect(visible.locator('[data-item="sort"]')).toHaveAttribute("aria-checked", "false");
  await page.keyboard.press("Escape");
  await expect(sort).toHaveCount(0);
  // Still reachable from the palette (⌘K) and the commands.
  expect(await page.evaluate(() => window.__tally!.controller.registry.get("data.sort") !== undefined)).toBe(true);

  await page.reload();
  await expect(page.getByTestId("grid")).toBeVisible();
  await expect(page.getByTestId("toolbar-row").locator('[data-command="filter"], [data-command="data.filter"]')).toBeVisible();
  await expect(page.getByTestId("toolbar-row").locator('[data-command="data.sort"]')).toHaveCount(0);

  await run(page, "view.customizeToolbars");
  const dlg = page.getByTestId("customize-toolbars");
  await expect(dlg).toBeVisible();
  await expect(dlg.getByTestId("customize-toolbar-sort")).not.toBeChecked();
  await dlg.getByTestId("customize-formatting-percent").uncheck();
  await dlg.getByTestId("dialog-ok").click();
  await expect(page.getByTestId("format-bar").locator('[data-command="format.percent"]')).toHaveCount(0);
  await run(page, "view.customizeToolbars");
  await dlg.getByTestId("customize-reset").click();
  await expect(dlg.getByTestId("customize-toolbar-sort")).toBeChecked();
  await dlg.getByTestId("dialog-ok").click();
  await expect(page.getByTestId("toolbar-row").locator('[data-command="data.sort"]')).toBeVisible();
  await expect(page.getByTestId("format-bar").locator('[data-command="format.percent"]')).toBeVisible();
});

test("whole bars toggle; hiding the toolbar leaves a title strip", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => localStorage.removeItem("tally.prefs"));
  await ready(page);
  await run(page, "view.formattingBar");
  await expect(page.getByTestId("format-bar")).toHaveCount(0);
  await run(page, "view.toolbar");
  await expect(page.getByTestId("title-strip")).toContainText("Invoices");
  await expect(page.getByTestId("status-bar")).toBeVisible();
  await expect(page.getByTestId("formula-input")).toBeVisible();
  await run(page, "view.statusBar");
  await run(page, "view.formulaBar");
  await expect(page.getByTestId("status-bar")).toHaveCount(0);
  await expect(page.getByTestId("formula-input")).toHaveCount(0);
  await expect(page.getByTestId("grid")).toBeVisible();
  // ⇧⌘T shows the formula bar for the Name Box; it hides again afterwards.
  await page.getByTestId("grid").click({ position: { x: 200, y: 100 } });
  await page.keyboard.press("Meta+Shift+t");
  await expect(page.getByTestId("name-box")).toBeFocused();
  await page.keyboard.type("C5");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("name-box")).toHaveCount(0);
  expect(await page.evaluate(() => window.__tally!.controller.selection.active)).toEqual({ r: 4, c: 2 });
  // The title strip offers the bars again.
  await page.getByTestId("title-strip").click({ button: "right" });
  await page.getByTestId("bar-menu").locator('[data-item="bar-toolbar"]').click();
  await expect(page.getByTestId("toolbar-row")).toBeVisible();
  for (const id of ["view.formattingBar", "view.statusBar", "view.formulaBar"]) await run(page, id);
  await expect(page.getByTestId("format-bar")).toBeVisible();
});

test("the default window fits the format strip; narrower windows go compact first, then »", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => localStorage.removeItem("tally.prefs"));
  await ready(page);
  const row = page.getByTestId("format-row");
  // 1280 × 820 with the sidebar: everything shows, the two wide buttons icon-only.
  await expect(page.getByTestId("format-row-more")).toHaveCount(0);
  await expect(row).toHaveAttribute("data-compact", "true");
  const hl = page.getByTestId("format-bar").getByTestId("highlight-cells");
  await expect(hl).toBeVisible();
  await expect(hl).toHaveAttribute("aria-label", "Highlight cells");
  await expect(hl).not.toContainText("Highlight");
  // Wider: labels come back.
  await page.setViewportSize({ width: 1500, height: 820 });
  await expect(row).not.toHaveAttribute("data-compact", "true");
  await expect(hl).toContainText("Highlight");
  // Narrow: » holds the rest, nothing is clipped.
  await page.setViewportSize({ width: 820, height: 700 });
  const more = page.getByTestId("format-row-more");
  await expect(more).toBeVisible();
  await expect(page.getByTestId("format-bar").getByTestId("highlight-cells")).toHaveCount(0);
  const fits = await page.evaluate(() => {
    const bar = document.querySelector('[data-testid="format-bar"]')!.getBoundingClientRect();
    const slots = [...document.querySelectorAll('[data-testid="format-row"] > [data-slot]')];
    const last = slots.at(-1)!.getBoundingClientRect();
    const m = document.querySelector('[data-testid="format-row-more"]')!.getBoundingClientRect();
    return { lastInside: last.right <= m.left, moreInside: m.right <= bar.right };
  });
  expect(fits).toEqual({ lastInside: true, moreInside: true });
  // Keyboard opens »; Esc closes it and the grid has the focus again.
  await more.focus();
  await page.keyboard.press("Enter");
  const panel = page.getByTestId("format-row-overflow");
  await expect(panel).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => document.activeElement?.closest('[data-testid="grid"]') != null || document.activeElement?.tagName === "BODY")).toBe(true);
  await more.click();
  await expect(panel.getByTestId("highlight-cells")).toBeVisible();
  await panel.getByTestId("highlight-cells").click();
  await expect(page.getByTestId("highlight-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 1280, height: 820 });
  await expect(page.getByTestId("format-row-more")).toHaveCount(0);
});

test("in the » panel, Esc closes an open font list first", async ({ page }) => {
  // So narrow that the font size box goes into the panel.
  await page.setViewportSize({ width: 420, height: 700 });
  await ready(page);
  await page.evaluate(() => localStorage.removeItem("tally.prefs"));
  await ready(page);
  await page.getByTestId("format-row-more").click();
  const panel = page.getByTestId("format-row-overflow");
  await expect(panel).toBeVisible();
  const box = panel.getByTestId("font-size-input");
  if ((await box.count()) === 0) test.skip(true, "font size box fits at this width");
  await box.click();
  await page.keyboard.press("ArrowDown");
  await expect(box).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("Escape");
  await expect(box).toHaveAttribute("aria-expanded", "false");
  await expect(panel).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
});
