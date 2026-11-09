/**
 * Tools ▸ Options ▸ View (AOO 4.1 `ScTpContentOptions`, U18) against the
 * mock engine:
 * - document options (`setViewOptions`, undoable): sheet tabs, outline
 *   symbols, comment indicator, grid colour;
 * - app prefs (AOO configuration): scroll bars, chart Show / Hide, handles,
 *   anchor, guides while moving.
 */
import { expect, test, type Page } from "@playwright/test";

interface Rec {
  type: string;
  [k: string]: unknown;
}

const ROW_H = 24;

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, headerH, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    const geo = ctl.grid!.getGeometry();
    return { headerW: geo.headerW, headerH: geo.headerH, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  return { x: box.x + headerW + xs + w / 2, y: box.y + headerH + r * ROW_H + ROW_H / 2 };
}

async function clickCell(page: Page, r: number, c: number, modifier?: "Shift") {
  const p = await cellXY(page, r, c);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(p.x, p.y);
  if (modifier) await page.keyboard.up(modifier);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

async function openView(page: Page) {
  await page.keyboard.press("Meta+,");
  const dlg = page.getByTestId("options-dialog");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("tab-view").click();
  return dlg;
}

const chartNode = (page: Page) => page.locator('.tally-grid-object[data-object-id^="chart:"]');

test("document view options: sheet tabs, outline symbols, comment indicator and grid colour (undoable)", async ({ page }) => {
  await ready(page);
  // A row group gives the outline gutter something to show.
  await clickCell(page, 2, 0);
  await clickCell(page, 4, 0, "Shift");
  await run(page, "data.group");
  const gutter = () => page.evaluate(() => window.__tally!.controller.grid!.getGeometry().gutterW);
  await expect.poll(gutter).toBeGreaterThan(0);
  const tabs = page.getByRole("tablist", { name: "Sheets" });
  await expect(tabs).toBeVisible();

  let dlg = await openView(page);
  // AOO's groups, every control labelled.
  for (const g of ["Visual aids", "Display", "Objects", "Window"]) await expect(dlg.getByRole("group", { name: g })).toBeVisible();
  await expect(dlg.getByTestId("opt-grid-color")).toHaveValue("auto");
  await dlg.getByTestId("opt-sheet-tabs").uncheck();
  await dlg.getByTestId("opt-outline-symbols").uncheck();
  await dlg.getByTestId("opt-comment-indicator").uncheck();
  await dlg.getByTestId("opt-grid-color").selectOption("#000080");
  await dlg.getByTestId("dialog-ok").click();
  await expect(dlg).toBeHidden();

  const op = (await recorded(page)).filter((o) => o.type === "setViewOptions").at(-1) as { options: Record<string, unknown> } | undefined;
  expect(op?.options).toMatchObject({ showSheetTabs: false, showOutlineSymbols: false, showNotes: false, gridColor: 0x000080 });
  await expect(tabs).toHaveCount(0);
  await expect.poll(gutter).toBe(0);
  // The canvas draws the grid in the chosen colour.
  await expect.poll(() => page.evaluate(() => (window.__tally!.controller.grid as unknown as { props: { theme: { colors: { gridLine: string } } } }).props.theme.colors.gridLine)).toBe("#000080");

  // The dialog shows what the document holds.
  dlg = await openView(page);
  await expect(dlg.getByTestId("opt-sheet-tabs")).not.toBeChecked();
  await expect(dlg.getByTestId("opt-grid-color")).toHaveValue("#000080");
  await page.keyboard.press("Escape");
  await expect(dlg).toBeHidden();

  // One undo step brings everything back.
  await page.keyboard.press("Meta+z");
  await expect(tabs).toBeVisible();
  await expect.poll(gutter).toBeGreaterThan(0);
});

test("app view prefs: scroll bars, charts Show/Hide, handles, anchor and guides while moving", async ({ page }) => {
  await ready(page);
  await clickCell(page, 0, 0);
  await clickCell(page, 5, 2, "Shift");
  await page.locator('[data-command="insert.chart"]').click();
  await expect(chartNode(page)).toHaveCount(1);
  await page.keyboard.press("Escape");

  const root = page.locator('[data-testid="grid"] .tally-grid').first();
  await expect(root).not.toHaveAttribute("data-scrollbars", /.*/);
  let dlg = await openView(page);
  await expect(dlg.getByTestId("opt-big-handles")).toBeChecked();
  await expect(dlg.getByTestId("opt-anchor")).toBeChecked();
  await dlg.getByTestId("opt-hscroll").uncheck();
  await dlg.getByTestId("opt-help-lines").check();
  await dlg.getByTestId("opt-simple-handles").check();
  await dlg.getByTestId("opt-big-handles").uncheck();
  await dlg.getByTestId("dialog-ok").click();
  await expect(dlg).toBeHidden();
  // Not a document change: kept in the app prefs.
  expect((await recorded(page)).some((o) => o.type === "setViewOptions")).toBe(false);
  const prefs = await page.evaluate(() => JSON.parse(localStorage.getItem("tally.prefs") ?? "{}") as { view?: Record<string, unknown> });
  expect(prefs.view).toMatchObject({ hScroll: false, helpLines: true, simpleHandles: true, bigHandles: false });
  await expect(root).toHaveAttribute("data-scrollbars", "v");

  // Selecting the chart: small simple handles and the anchor marker on its cell.
  const node = chartNode(page);
  const box = (await node.boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await expect(node).toHaveAttribute("data-selected", "true");
  const handle = node.locator('[data-handle="se"]');
  await expect(handle).toHaveAttribute("data-handle-style", "simple");
  expect((await handle.boundingBox())!.width).toBeCloseTo(7, 0);
  await expect(page.locator(".tally-grid-anchor")).toHaveCount(1);

  // Guides while moving: four lines while the drag lasts.
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 30, box.y + box.height / 2 + 20, { steps: 4 });
  await expect(page.locator(".tally-grid-guide")).toHaveCount(4);
  await page.mouse.up();
  await expect(page.locator(".tally-grid-guide")).toHaveCount(0);

  // Charts: Hide removes them from the sheet; Show brings them back.
  dlg = await openView(page);
  await dlg.getByTestId("opt-obj-charts").selectOption("hide");
  await dlg.getByTestId("dialog-ok").click();
  await expect(chartNode(page)).toHaveCount(0);
  dlg = await openView(page);
  await dlg.getByTestId("opt-obj-charts").selectOption("show");
  await dlg.getByTestId("opt-hscroll").check();
  await dlg.getByTestId("dialog-ok").click();
  await expect(chartNode(page)).toHaveCount(1);
  await expect(root).not.toHaveAttribute("data-scrollbars", /.*/);
});
