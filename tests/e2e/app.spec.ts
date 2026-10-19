/**
 * WP7 acceptance in the browser (MockClient): values and a formula, the
 * formula bar + hint, ⌘B, ⌘Z, add/rename a sheet, status bar Sum, ⌘K.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const COL_W = 100;

/** Clicks a cell by 0-based row/col using the grid's geometry. */
async function clickCell(page: Page, r: number, c: number): Promise<void> {
  const grid = page.getByTestId("grid");
  const box = await grid.boundingBox();
  if (!box) throw new Error("grid not visible");
  // Row header width is dynamic (grid.ts `headerW`); the column header is 24 px tall.
  const headerW = await page.evaluate(() => {
    const g = window.__tally?.controller.grid as unknown as { headerW?: number } | null | undefined;
    return g?.headerW ?? 44;
  });
  await page.mouse.click(box.x + headerW + c * COL_W + COL_W / 2, box.y + ROW_H + r * ROW_H + ROW_H / 2);
}

async function typeInto(page: Page, r: number, c: number, text: string): Promise<void> {
  await clickCell(page, r, c);
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
});

test("type values and a formula; formula bar, hint, bold, undo, sum", async ({ page }) => {
  await typeInto(page, 0, 0, "2");
  await typeInto(page, 0, 1, "3");
  await typeInto(page, 0, 2, "=A1*B1");

  // C1 shows 6 (read through the engine client).
  const c1 = await page.evaluate(async () => {
    const t = window.__tally;
    if (!t) return null;
    const info = t.controller.info;
    if (!info) return null;
    const [block] = await t.client.cells(info.doc, t.controller.sheet, [{ r0: 0, c0: 2, r1: 0, c1: 2 }]);
    return block?.cells[0]?.text ?? null;
  });
  expect(c1).toBe("6");

  // Formula bar: prettified formula and the plain-English hint.
  await clickCell(page, 0, 2);
  await expect(page.getByTestId("formula-input")).toHaveValue("=A1 × B1");
  await expect(page.getByTestId("formula-hint")).toHaveText("A1 × B1");

  // ⌘B bolds the active cell; the toolbar reflects it.
  await page.keyboard.press("ControlOrMeta+b");
  const bold = page.locator('[data-command="format.bold"]');
  await expect(bold).toHaveAttribute("aria-pressed", "true");

  // ⌘Z undoes the bold.
  await page.keyboard.press("ControlOrMeta+z");
  await expect(bold).toHaveAttribute("aria-pressed", "false");

  // Status bar: Sum = 11 for A1:C1.
  await clickCell(page, 0, 0);
  await page.keyboard.press("Shift+ArrowRight");
  await page.keyboard.press("Shift+ArrowRight");
  await expect(page.getByTestId("name-box")).toHaveValue("A1:C1");
  await expect(page.getByTestId("stat-sum")).toContainText("11");
  await expect(page.getByTestId("stat-count")).toContainText("3");
});

test("add a sheet and rename it", async ({ page }) => {
  await page.getByRole("button", { name: "Add sheet" }).click();
  const tab = page.getByRole("tab", { name: "Sheet2" });
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await tab.dblclick();
  const input = page.getByTestId("sheet-rename");
  await input.fill("Orders");
  await input.press("Enter");
  await expect(page.getByRole("tab", { name: "Orders" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Sheet2" })).toHaveCount(0);
});

test("⌘K lists commands and runs one", async ({ page }) => {
  await clickCell(page, 0, 0);
  await page.keyboard.press("ControlOrMeta+k");
  const input = page.getByTestId("palette-input");
  await expect(input).toBeFocused();
  await expect(page.locator("[cmdk-item]").filter({ hasText: "Bold" }).first()).toBeVisible();
  await expect(page.locator("[cmdk-item]").filter({ hasText: "Insert sheet" })).toBeVisible();
  await input.fill("insert sheet");
  await page.keyboard.press("Enter");
  await expect(input).toHaveCount(0);
  await expect(page.getByRole("tab", { name: "Sheet2" })).toBeVisible();
});

test("Σ Add up sums the numbers above", async ({ page }) => {
  await typeInto(page, 0, 0, "4");
  await typeInto(page, 1, 0, "5");
  await clickCell(page, 2, 0);
  await page.getByRole("button", { name: "Add up" }).click();
  await clickCell(page, 2, 0);
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUM(A1:A2)");
});
