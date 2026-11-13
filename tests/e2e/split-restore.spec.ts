/**
 * Window ▸ Split restore against the mock engine: the split is stored per
 * sheet (`setSplit`, one undo step) and shown again when the sheet is shown
 * and after undo / redo.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;

async function ready(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function clickCell(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const geo = await page.evaluate(() => {
    const g = window.__tally!.controller.grid!.getGeometry();
    return { headerW: g.headerW, headerH: g.headerH };
  });
  await page.mouse.click(box.x + geo.headerW + 40 + c * 80, box.y + geo.headerH + r * ROW_H + ROW_H / 2);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

/** The split the engine stores for `sheet` (`view_options().split`). */
const stored = (page: Page, sheet: number) =>
  page.evaluate(async (sheet) => {
    const t = window.__tally!;
    const v = await (t.client as unknown as { viewOptions(doc: number, sheet: number): Promise<{ split?: { xPx: number; yPx: number } | null }> }).viewOptions(t.controller.info!.doc, sheet);
    return v.split ?? null;
  }, sheet);

const splitAxis = (page: Page) => page.getByTestId("grid").getAttribute("data-split");

async function dividerY(page: Page): Promise<number> {
  const box = await page.locator(".tally-split-divider").boundingBox();
  if (!box) throw new Error("no divider");
  return box.y;
}

async function twoSheets(page: Page): Promise<number[]> {
  return page.evaluate(async () => {
    const c = window.__tally!.controller;
    await c.addSheet();
    const ids = c.info!.sheets.map((s) => s.id);
    c.showSheet(ids[0]!, true);
    return ids;
  });
}

const show = (page: Page, sheet: number) => page.evaluate((s) => window.__tally!.controller.showSheet(s, true), sheet);

test("the split is shown again when its sheet is shown", async ({ page }) => {
  await ready(page);
  const [s1, s2] = await twoSheets(page);
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.sheet)).toBe(s1);
  await clickCell(page, 6, 1);
  await run(page, "view.split");
  await expect.poll(() => splitAxis(page)).toBe("horizontal");
  await expect.poll(async () => (await stored(page, s1!))?.yPx ?? 0).toBeGreaterThan(0);
  const y = await dividerY(page);

  await show(page, s2!);
  await expect.poll(() => splitAxis(page)).toBeNull();
  await expect(page.locator(".tally-split-divider")).toHaveCount(0);

  await show(page, s1!);
  await expect.poll(() => splitAxis(page)).toBe("horizontal");
  await expect.poll(async () => Math.abs((await dividerY(page)) - y)).toBeLessThanOrEqual(2);
});

test("undo and redo of Split Window and Remove Split restore the split", async ({ page }) => {
  await ready(page);
  const sheet = await page.evaluate(() => window.__tally!.controller.sheet);
  await clickCell(page, 6, 1);
  await run(page, "view.split");
  await expect.poll(() => splitAxis(page)).toBe("horizontal");
  await expect.poll(async () => (await stored(page, sheet))?.yPx ?? 0).toBeGreaterThan(0);
  const y = await dividerY(page);

  // Undo the split: the engine and the window have none.
  await run(page, "edit.undo");
  await expect.poll(() => stored(page, sheet)).toBeNull();
  await expect.poll(() => splitAxis(page)).toBeNull();

  // Redo: back at the same place.
  await run(page, "edit.redo");
  await expect.poll(() => splitAxis(page)).toBe("horizontal");
  await expect.poll(async () => Math.abs((await dividerY(page)) - y)).toBeLessThanOrEqual(2);

  // Remove Split (the toggle), then undo it.
  await run(page, "view.split");
  await expect.poll(() => splitAxis(page)).toBeNull();
  await expect.poll(() => stored(page, sheet)).toBeNull();
  await run(page, "edit.undo");
  await expect.poll(() => splitAxis(page)).toBe("horizontal");
  await expect.poll(async () => Math.abs((await dividerY(page)) - y)).toBeLessThanOrEqual(2);
});
