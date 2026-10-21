/**
 * Accessibility (wave 5, §4 item 46; W5-0b): the grid's ARIA mirror
 * (`role=grid`, row/column counts, the active-cell descendant, the live
 * region announcing address + value + formula), modal dialogs that trap
 * focus, and the range-pick seam a dialog uses to ask for a range.
 * VoiceOver walkthrough: docs/qa/a11y.md.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    return { headerW: (ctl.grid as unknown as { headerW: number }).headerW, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  return { x: box.x + headerW + xs + w / 2, y: box.y + HEADER_H + r * ROW_H + ROW_H / 2 };
}

test("the grid exposes an ARIA mirror and announces the active cell", async ({ page }) => {
  await ready(page);
  const grid = page.locator(".tally-grid[role='grid']");
  await expect(grid).toHaveAttribute("aria-rowcount", "1048576");
  await expect(grid).toHaveAttribute("aria-colcount", "16384");
  await expect(grid).toHaveAttribute("aria-multiselectable", "true");
  await expect(grid).toHaveAttribute("aria-label", /^Sheet /);

  const editor = grid.locator("textarea");
  const activeId = await editor.getAttribute("aria-activedescendant");
  expect(activeId).toBeTruthy();
  const active = page.locator(`#${activeId}`);
  await expect(active).toHaveAttribute("role", "gridcell");
  await expect(active).toHaveAttribute("aria-rowindex", "1");
  await expect(active).toHaveAttribute("aria-colindex", "1");
  await expect(active.locator("xpath=..")).toHaveAttribute("role", "row");

  // Move to B2: the descendant and the live region follow with address + value.
  await editor.focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowRight");
  await expect(active).toHaveAttribute("aria-rowindex", "2");
  await expect(active).toHaveAttribute("aria-colindex", "2");
  const live = page.getByTestId("aria-live");
  await expect(live).toHaveAttribute("aria-live", "polite");
  await expect(live).toHaveText(/^B2: .+/);
  await expect(live).not.toHaveText(/empty/);

  // A formula cell announces its formula (the invoices template's totals are formulas).
  const formulaCell = await page.evaluate(async () => {
    const t = window.__tally!;
    const info = t.controller.info!;
    const [block] = await t.client.cells(info.doc, t.controller.sheet, [{ r0: 0, c0: 0, r1: 20, c1: 8 }]);
    const f = block?.cells.find((c) => (c.flags & 1) !== 0);
    return f ? { r: f.r, c: f.c } : null;
  });
  expect(formulaCell).not.toBeNull();
  const p = await cellXY(page, formulaCell!.r, formulaCell!.c);
  await page.mouse.click(p.x, p.y);
  await expect(live).toHaveText(/, formula =/);

  // Accessibility tree: grid › row › gridcell, plus the status region (the
  // objects layer, group "Objects", sits next to the grid root since W5-5).
  await expect(grid).toMatchAriaSnapshot(`
    - grid /^Sheet /:
      - textbox "Cell editor"
      - row /, formula =/:
        - gridcell /, formula =/ [selected]
      - status
  `);

  // A multi-cell selection announces its extent.
  await page.keyboard.press("Shift+ArrowDown");
  await expect(live).toHaveText(/selected [A-Z]+\d+ to [A-Z]+\d+ \(2 rows, 1 columns\)/);
});

test("dialogs are modal and keep Tab inside; the Function Wizard inserts NAME(", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("ControlOrMeta+F2");
  const dialog = page.getByTestId("function-wizard");
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveAttribute("role", "dialog");
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  // Focus starts in the dialog and stays there through a full Tab cycle.
  await expect(page.getByTestId("wizard-search")).toBeFocused();
  for (let i = 0; i < 8; i++) {
    await page.keyboard.press("Tab");
    const inside = await page.evaluate(() => !!document.activeElement?.closest("[data-testid='function-wizard']"));
    expect(inside, `Tab #${i + 1}`).toBe(true);
  }
  // Shift+Tab from the first control wraps to the last (the Insert button).
  await page.getByTestId("wizard-category").focus();
  await page.keyboard.press("Shift+Tab");
  await expect(page.getByTestId("dialog-ok")).toBeFocused();

  // The list is a listbox; searching narrows it and Enter inserts.
  await expect(page.getByTestId("wizard-list")).toHaveAttribute("role", "listbox");
  // (The browser mock lists a few functions; the engine lists all 400+.)
  await page.getByTestId("wizard-search").fill("su");
  await expect(page.getByTestId("wizard-list").getByRole("option", { selected: true })).toHaveText("SUM");
  await expect(page.getByTestId("wizard-description")).toContainText("SUM(");
  await page.keyboard.press("Enter");
  await expect(dialog).toBeHidden();
  await expect(page.getByTestId("formula-input")).toHaveValue(/^=SUM\(/);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("formula-input")).not.toHaveValue(/SUM/);
});

test("a dialog can ask the grid for a range (pickRange seam)", async ({ page }) => {
  await ready(page);
  const a = await cellXY(page, 1, 1);
  const b = await cellXY(page, 3, 2);
  const picked = page.evaluate(() => window.__tally!.controller.grid!.pickRange());
  await expect(page.locator(".tally-grid")).toHaveAttribute("data-range-pick", "true");
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await page.mouse.move(b.x, b.y, { steps: 4 });
  await page.mouse.up();
  expect(await picked).toEqual({ r0: 1, c0: 1, r1: 3, c1: 2 });
  await expect(page.locator(".tally-grid")).not.toHaveAttribute("data-range-pick", "true");
  // The selection itself did not move while picking.
  const active = await page.evaluate(() => window.__tally!.controller.selection.active);
  expect(active).toEqual({ r: 0, c: 0 });
});
