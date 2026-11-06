/**
 * Tools ▸ Detective at AOO 4.1 parity in the browser (WebMockClient ports
 * the engine's detective.rs): each Trace Precedents adds a level, an edit
 * redraws the arrows with AutoRefresh on, Refresh Traces shows a new
 * arrow with AutoRefresh off, and Fill Mode traces the clicked cell until
 * Esc.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const COL_W = 100;
const HEADER_H = 24;

async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const headerW = await page.evaluate(() => (window.__tally?.controller.grid as unknown as { headerW?: number } | null)?.headerW ?? 44);
  return { x: box.x + headerW + c * COL_W + COL_W / 2, y: box.y + HEADER_H + r * ROW_H + ROW_H / 2 };
}

async function clickCell(page: Page, r: number, c: number) {
  const p = await cellXY(page, r, c);
  await page.mouse.click(p.x, p.y);
}

async function typeInto(page: Page, r: number, c: number, text: string) {
  await clickCell(page, r, c);
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

async function ready(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

type W5 = { w5: { state(): { traces: number; detectiveFill: boolean } } };
const traces = (page: Page) => page.evaluate(() => (window.__tally!.controller as unknown as W5).w5.state().traces);
const fillMode = (page: Page) => page.evaluate(() => (window.__tally!.controller as unknown as W5).w5.state().detectiveFill);
const active = (page: Page) => page.evaluate(() => ({ ...window.__tally!.controller.selection.active }));

async function chain(page: Page) {
  await ready(page);
  await typeInto(page, 0, 0, "1");
  await typeInto(page, 0, 1, "=A1");
  await typeInto(page, 0, 2, "=B1");
}

test("Trace Precedents adds a level each time; Remove Precedents takes one away", async ({ page }) => {
  await chain(page);
  await clickCell(page, 0, 2);
  await run(page, "tools.tracePrecedents");
  await expect.poll(() => traces(page)).toBe(1);
  await run(page, "tools.tracePrecedents");
  await expect.poll(() => traces(page)).toBe(2);
  await run(page, "tools.removePrecedents");
  await expect.poll(() => traces(page)).toBe(1);
  await run(page, "tools.traceDependents");
  await run(page, "tools.removeTraces");
  await expect.poll(() => traces(page)).toBe(0);
});

test("AutoRefresh redraws after an edit; with it off, Refresh Traces shows the new arrow", async ({ page }) => {
  await chain(page);
  await clickCell(page, 0, 2);
  await run(page, "tools.tracePrecedents");
  await expect.poll(() => traces(page)).toBe(1);
  // AutoRefresh (on by default): C1 now also reads D1.
  await typeInto(page, 0, 2, "=B1+D1");
  await expect.poll(() => traces(page)).toBe(2);
  // Off: the edit leaves the arrows as they were until Refresh Traces.
  await run(page, "tools.autoRefreshTraces");
  await expect.poll(() => page.evaluate(() => (window.__tally!.controller as unknown as { w5: { detective: { autoRefresh: boolean } } }).w5.detective.autoRefresh)).toBe(false);
  await typeInto(page, 0, 2, "=B1+D1+E1");
  await page.waitForTimeout(300);
  expect(await traces(page)).toBe(2);
  await run(page, "tools.refreshTraces");
  await expect.poll(() => traces(page)).toBe(3);
  // Back on for the next tests (the option persists per viewer).
  await run(page, "tools.autoRefreshTraces");
});

test("Fill Mode traces the clicked cell and ends with Esc", async ({ page }) => {
  await chain(page);
  await typeInto(page, 4, 4, "=A1+B1");
  await run(page, "tools.detectiveFillMode");
  await expect.poll(() => fillMode(page)).toBe(true);
  await expect(page.locator(".tally-grid[data-detective-fill]")).toHaveCount(1);
  await clickCell(page, 4, 4);
  // The cursor moved there and its precedents were traced.
  await expect.poll(() => active(page)).toEqual({ r: 4, c: 4 });
  await expect.poll(() => traces(page)).toBe(2);
  // A second click on the same cell adds the next level (B1 reads A1).
  await clickCell(page, 4, 4);
  await expect.poll(() => traces(page)).toBe(3);
  await page.keyboard.press("Escape");
  await expect.poll(() => fillMode(page)).toBe(false);
  await expect(page.locator(".tally-grid[data-detective-fill]")).toHaveCount(0);
  // Clicks select again and trace nothing.
  await clickCell(page, 0, 2);
  await expect.poll(() => active(page)).toEqual({ r: 0, c: 2 });
  expect(await traces(page)).toBe(3);
});

const inputAt = (page: Page, r: number, c: number) =>
  page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      return (await t.client.cellEditInfo(t.controller.info!.doc, t.controller.sheet, r, c)).input;
    },
    [r, c] as const,
  );

test("Fill Mode blocks double-click editing, fill-handle AutoFill and the context menu; right clicks keep it on", async ({ page }) => {
  await chain(page);
  await clickCell(page, 0, 1);
  await run(page, "tools.detectiveFillMode");
  await expect.poll(() => fillMode(page)).toBe(true);
  // Double-click on a cell: no editor (typing does not change the formula).
  const p = await cellXY(page, 0, 2);
  await page.mouse.dblclick(p.x, p.y);
  // Opening the editor would have ended Fill Mode (editing switches it off).
  await page.waitForTimeout(200);
  expect(await fillMode(page)).toBe(true);
  expect(await inputAt(page, 0, 2)).toBe("=B1");
  // Double-click on the fill handle of B1 (bottom-right corner): no AutoFill.
  const b = await cellXY(page, 0, 1);
  await page.mouse.dblclick(b.x + COL_W / 2 - 1, b.y + ROW_H / 2 - 1);
  expect(await inputAt(page, 1, 1)).toBe("");
  // Right click: no context menu, the mode stays on (AOO: only a plain left click acts).
  await page.mouse.click(p.x, p.y, { button: "right" });
  await expect(page.getByTestId("context-menu")).toHaveCount(0);
  expect(await fillMode(page)).toBe(true);
});

test("Fill Mode marks the document modified, survives a sheet switch, and ends on a range pick or a new document", async ({ page }) => {
  await chain(page);
  // As if just saved: the store says clean.
  await page.evaluate(() => {
    (window.__tally!.controller as unknown as { info: { dirty: boolean } }).info.dirty = false;
  });
  await run(page, "tools.detectiveFillMode");
  await clickCell(page, 0, 2);
  await expect.poll(() => traces(page)).toBe(1);
  // The trace is a modification (AOO SetDocumentModified): the close guard asks.
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.info?.dirty)).toBe(true);
  // A sheet switch keeps the mode (AOO leaves the auditing shell on).
  const first = await page.evaluate(() => window.__tally!.controller.sheet);
  await page.evaluate(() => window.__tally!.controller.addSheet());
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.sheet)).not.toBe(first);
  expect(await fillMode(page)).toBe(true);
  // A dialog's range pick takes the clicks and ends the mode.
  await page.evaluate(() => {
    void (window.__tally!.controller.grid as unknown as { pickRange(): Promise<unknown> }).pickRange();
  });
  await expect.poll(() => fillMode(page)).toBe(false);
  await page.keyboard.press("Escape");
  // A new document in the window ends it too.
  await run(page, "tools.detectiveFillMode");
  await expect.poll(() => fillMode(page)).toBe(true);
  await page.evaluate(async () => {
    const t = window.__tally!;
    t.controller.load(await t.client.newWorkbook());
  });
  await expect.poll(() => fillMode(page)).toBe(false);
});
