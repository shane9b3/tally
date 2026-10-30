/**
 * References picked on other sheets while a formula is edited (AOO's
 * reference input across sheets): a tab click or ⌃PgDn shows the other
 * sheet with the edit still open, cell clicks insert `Sheet2.B2`, Shift+tab
 * builds a 3D reference, Enter commits into the original cell and returns,
 * Escape returns without a change. A plain-text edit is committed by a tab
 * click as before. Runs against the browser mock.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const COL_W = 100;

async function clickCell(page: Page, r: number, c: number, modifiers: "Shift"[] = []): Promise<void> {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const headerW = await page.evaluate(() => (window.__tally?.controller.grid as unknown as { headerW?: number } | null)?.headerW ?? 44);
  for (const m of modifiers) await page.keyboard.down(m);
  await page.mouse.click(box.x + headerW + c * COL_W + COL_W / 2, box.y + ROW_H + r * ROW_H + ROW_H / 2);
  for (const m of modifiers) await page.keyboard.up(m);
}

/** Three sheets (Sheet1..Sheet3), Sheet1 shown; returns their ids. */
async function threeSheets(page: Page, seed: { sheet: number; r: number; c: number; input: string }[] = []): Promise<number[]> {
  return page.evaluate(async (seed) => {
    const c = window.__tally!.controller;
    await c.addSheet();
    await c.addSheet();
    const ids = c.info!.sheets.map((s) => s.id);
    if (seed.length > 0) {
      await c.apply(
        "seed",
        seed.map((s) => ({ type: "setCellInput" as const, sheet: ids[s.sheet]!, r: s.r, c: s.c, input: s.input })),
      );
    }
    c.showSheet(ids[0]!, true);
    return ids;
  }, seed);
}

const tabs = (page: Page) => page.getByRole("tablist", { name: "Sheets" }).getByRole("tab");
const shown = (page: Page) => page.evaluate(() => window.__tally!.controller.sheet);
const editText = (page: Page) => page.evaluate(() => window.__tally!.controller.editText ?? null);

async function cell(page: Page, sheet: number, r: number, c: number): Promise<{ input: string; text: string }> {
  return page.evaluate(
    async ({ sheet, r, c }) => {
      const t = window.__tally!;
      const doc = t.controller.info!.doc;
      const info = await t.client.cellEditInfo(doc, sheet, r, c);
      const [block] = await t.client.cells(doc, sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      return { input: info.input, text: block?.cells.find((v) => v.r === r && v.c === c)?.text ?? "" };
    },
    { sheet, r, c },
  );
}

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
});

test("pick on Sheet2 and back on Sheet1; Enter commits into Sheet1.A1", async ({ page }) => {
  const ids = await threeSheets(page, [
    { sheet: 1, r: 1, c: 1, input: "5" },
    { sheet: 0, r: 2, c: 2, input: "7" },
  ]);
  await clickCell(page, 0, 0);
  await page.keyboard.type("=");
  await tabs(page).nth(1).click();
  await expect.poll(() => shown(page)).toBe(ids[1]);
  // The edit stays open, shown in the formula bar; the grid draws no editor or cursor of its own.
  await expect(page.getByTestId("grid")).toHaveAttribute("data-edit-elsewhere", "true");
  await expect(page.getByTestId("formula-input")).toHaveValue("=");
  await expect(page.getByTestId("name-box")).toHaveValue("Sheet1.A1");
  await clickCell(page, 1, 1);
  await expect(page.getByTestId("formula-input")).toHaveValue("=Sheet2.B2");
  await page.keyboard.type("+");
  await tabs(page).nth(0).click();
  await expect.poll(() => shown(page)).toBe(ids[0]);
  await expect(page.getByTestId("grid")).not.toHaveAttribute("data-edit-elsewhere", "true");
  await clickCell(page, 2, 2);
  await expect(page.getByTestId("formula-input")).toHaveValue("=Sheet2.B2+C3");
  await page.keyboard.press("Enter");
  await expect.poll(() => editText(page)).toBeNull();
  expect(await shown(page)).toBe(ids[0]);
  expect(await cell(page, ids[0]!, 0, 0)).toEqual({ input: "=Sheet2.B2+C3", text: "12" });
  await expect(page.getByTestId("name-box")).toHaveValue("A2");
});

test("Enter from the other sheet returns to the original cell and moves down", async ({ page }) => {
  const ids = await threeSheets(page, [{ sheet: 1, r: 0, c: 0, input: "3" }]);
  await clickCell(page, 4, 1);
  await page.keyboard.type("=");
  await tabs(page).nth(1).click();
  await clickCell(page, 0, 0);
  // Arrow keys move the picked reference on the shown sheet.
  await page.keyboard.press("ArrowDown");
  await expect(page.getByTestId("formula-input")).toHaveValue("=Sheet2.A2");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  await expect.poll(() => shown(page)).toBe(ids[0]);
  expect(await cell(page, ids[0]!, 4, 1)).toEqual({ input: "=Sheet2.A1", text: "3" });
  expect(await cell(page, ids[1]!, 0, 0)).toEqual({ input: "3", text: "3" });
  await expect(page.getByTestId("name-box")).toHaveValue("B6");
  // One undo step.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await cell(page, ids[0]!, 4, 1)).input).toBe("");
});

test("Escape on another sheet returns and leaves the cell alone", async ({ page }) => {
  const ids = await threeSheets(page, [{ sheet: 0, r: 3, c: 3, input: "keep" }]);
  await clickCell(page, 3, 3);
  await page.keyboard.type("=");
  await tabs(page).nth(2).click();
  await clickCell(page, 0, 0);
  await expect(page.getByTestId("formula-input")).toHaveValue("=Sheet3.A1");
  await page.keyboard.press("Escape");
  await expect.poll(() => shown(page)).toBe(ids[0]);
  await expect.poll(() => editText(page)).toBeNull();
  expect((await cell(page, ids[0]!, 3, 3)).input).toBe("keep");
  await expect(page.getByTestId("name-box")).toHaveValue("D4");
});

test("Shift-click on a tab extends the pick to a 3D reference", async ({ page }) => {
  const ids = await threeSheets(page, [
    { sheet: 0, r: 0, c: 0, input: "1" },
    { sheet: 1, r: 0, c: 0, input: "2" },
    { sheet: 2, r: 0, c: 0, input: "4" },
  ]);
  await clickCell(page, 5, 0);
  await page.keyboard.type("=SUM(");
  await clickCell(page, 0, 0);
  await tabs(page).nth(2).click({ modifiers: ["Shift"] });
  await expect.poll(() => shown(page)).toBe(ids[2]);
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUM(Sheet1.A1:Sheet3.A1");
  // The tab selection is untouched.
  expect(await page.evaluate(() => window.__tally!.controller.selectedSheets)).toEqual([ids[2]]);
  await page.keyboard.type(")");
  await page.keyboard.press("Enter");
  await expect.poll(() => shown(page)).toBe(ids[0]);
  expect(await cell(page, ids[0]!, 5, 0)).toEqual({ input: "=SUM(Sheet1.A1:Sheet3.A1)", text: "7" });
});

test("⌃PgDn while picking shows the next sheet and keeps the edit", async ({ page }) => {
  const ids = await threeSheets(page, [{ sheet: 1, r: 1, c: 0, input: "9" }]);
  await clickCell(page, 0, 0);
  await page.keyboard.type("=");
  await page.keyboard.press("Control+PageDown");
  await expect.poll(() => shown(page)).toBe(ids[1]);
  await clickCell(page, 1, 0);
  await page.keyboard.press("Control+PageDown");
  await expect.poll(() => shown(page)).toBe(ids[2]);
  await page.keyboard.press("Control+PageUp");
  await expect.poll(() => shown(page)).toBe(ids[1]);
  await expect(page.getByTestId("formula-input")).toHaveValue("=Sheet2.A2");
  await page.keyboard.press("Tab");
  await expect.poll(() => shown(page)).toBe(ids[0]);
  expect(await cell(page, ids[0]!, 0, 0)).toEqual({ input: "=Sheet2.A2", text: "9" });
  await expect(page.getByTestId("name-box")).toHaveValue("B1");
});

test("a plain-text edit is committed by a tab click, then the sheet switches", async ({ page }) => {
  const ids = await threeSheets(page);
  await clickCell(page, 0, 0);
  await page.keyboard.type("hello");
  await tabs(page).nth(1).click();
  await expect.poll(() => shown(page)).toBe(ids[1]);
  await expect.poll(() => editText(page)).toBeNull();
  expect((await cell(page, ids[0]!, 0, 0)).input).toBe("hello");
  expect((await cell(page, ids[1]!, 0, 0)).input).toBe("");
});

test("the formula bar path: pick on another sheet and commit with Enter", async ({ page }) => {
  const ids = await threeSheets(page, [{ sheet: 2, r: 2, c: 1, input: "6" }]);
  await clickCell(page, 1, 1);
  const bar = page.getByTestId("formula-input");
  await bar.click();
  await page.keyboard.type("=2*");
  await tabs(page).nth(2).click();
  await expect.poll(() => shown(page)).toBe(ids[2]);
  await expect(bar).toBeFocused();
  await clickCell(page, 2, 1);
  await expect(bar).toHaveValue("=2*Sheet3.B3");
  await bar.press("Enter");
  await expect.poll(() => shown(page)).toBe(ids[0]);
  expect(await cell(page, ids[0]!, 1, 1)).toEqual({ input: "=2*Sheet3.B3", text: "12" });
  await expect(page.getByTestId("name-box")).toHaveValue("B3");
});

test("Shift-click on a cell of another sheet extends the pick to 3D", async ({ page }) => {
  const ids = await threeSheets(page, [
    { sheet: 0, r: 0, c: 0, input: "1" },
    { sheet: 1, r: 1, c: 1, input: "2" },
  ]);
  await clickCell(page, 5, 0);
  await page.keyboard.type("=SUM(");
  await clickCell(page, 0, 0);
  await tabs(page).nth(1).click();
  await clickCell(page, 1, 1, ["Shift"]);
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUM(Sheet1.A1:Sheet2.B2");
  await page.keyboard.type(")");
  await page.keyboard.press("Enter");
  await expect.poll(() => shown(page)).toBe(ids[0]);
  expect(await cell(page, ids[0]!, 5, 0)).toEqual({ input: "=SUM(Sheet1.A1:Sheet2.B2)", text: "3" });
});

test("while picking: tab menu, rename and toolbar commands are off; a column header picks the column", async ({ page }) => {
  const ids = await threeSheets(page, [
    { sheet: 1, r: 0, c: 0, input: "4" },
    { sheet: 1, r: 1, c: 0, input: "5" },
  ]);
  await clickCell(page, 2, 0);
  await page.keyboard.type("=SUM(");
  await tabs(page).nth(1).click();
  await expect.poll(() => shown(page)).toBe(ids[1]);
  // The tab menu and double-click rename do nothing; the formula stays.
  await tabs(page).nth(2).click({ button: "right" });
  await tabs(page).nth(1).dblclick();
  await expect(page.getByTestId("sheet-rename")).toHaveCount(0);
  await expect(page.getByRole("menu")).toHaveCount(0);
  expect(await shown(page)).toBe(ids[1]);
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUM(");
  // Toolbar commands are disabled (Add up would write into this sheet).
  await expect(page.locator('[data-command="insert.addUp"]').first()).toHaveAttribute("aria-disabled", "true");
  await page.locator('[data-command="insert.addUp"]').first().click({ force: true });
  expect((await cell(page, ids[1]!, 2, 0)).input).toBe("");
  // A column header picks the whole column.
  const box = (await page.getByTestId("grid").boundingBox())!;
  const headerW = await page.evaluate(() => (window.__tally?.controller.grid as unknown as { headerW?: number } | null)?.headerW ?? 44);
  await page.mouse.click(box.x + headerW + 50, box.y + 12);
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUM(Sheet2.A:A");
  await page.keyboard.type(")");
  await page.keyboard.press("Enter");
  await expect.poll(() => shown(page)).toBe(ids[0]);
  // (The browser mock does not evaluate whole-column references; the input is what matters here.)
  expect((await cell(page, ids[0]!, 2, 0)).input).toBe("=SUM(Sheet2.A:A)");
});
