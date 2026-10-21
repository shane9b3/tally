/**
 * v0.4 screenshots for docs/qa (not part of `pnpm e2e`: the file name does
 * not end in `.spec.ts`). Run against the browser build:
 *
 *   pnpm --filter tally-desktop exec playwright test -c ../../tests/e2e/screenshots.v04.config.ts
 *
 * Writes docs/qa/screenshots/v04-*.png: a chart over the grid, the chart
 * editor, the print preview and Page Break Preview (WebMockClient emulates
 * the chart ops, chart data and the print layout).
 */
import { expect, test, type Page } from "@playwright/test";

const OUT = process.env.TALLY_SHOTS ?? "../../docs/qa/screenshots";
const ROW_H = 24;
const HEADER_H = 24;

async function ready(page: Page) {
  await page.goto("/?template=invoices");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(150);
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

/** A column chart of Qty and Price by date (columns D:E over A), added through the controller. */
async function insertChart(page: Page) {
  await page.evaluate(async () => {
    const ctl = window.__tally!.controller as unknown as { sheetName(): string; w5: { charts: { addChart(c: unknown, label?: string, open?: boolean): Promise<number | null> } } };
    const sheet = ctl.sheetName();
    const ref = (a: string, b: string) => `$${sheet}.$${a}:$${b}`;
    const series = (name: string, values: string) => ({ name: { type: "ref", ref: `$${sheet}.$${name}` }, values, xValues: null, sizes: null, kind: null, secondaryAxis: false, color: null, marker: null, labels: { value: false, percent: false, category: false, seriesName: false, position: "auto" }, trend: null, errorBars: null, meanLine: false });
    const axis = { visible: true, title: null, min: null, max: null, major: null, minorCount: null, log: false, reverse: false, majorGrid: true, minorGrid: false, numberFormat: null };
    await ctl.w5.charts.addChart({
      id: 0, name: "", anchor: { r: 1, c: 7, dxMm: 4, dyMm: 2, widthMm: 150, heightMm: 85 }, kind: "column", stacking: "none", lines: true, markers: false, smooth: false, hole: null, exploded: false, filled: false, stock: null, threeD: false,
      source: null, categories: ref("A$2", "A$15"), series: [series("D$1", ref("D$2", "D$15")), { ...series("E$1", ref("E$2", "E$15")), kind: "line", secondaryAxis: true }],
      title: "Quantity and unit price by invoice", subtitle: null, legend: "bottom", xAxis: { ...axis, majorGrid: false }, yAxis: { ...axis, title: "Qty" }, y2Axis: { ...axis, title: "Price", majorGrid: false }, background: null, plotBackground: null, includeHidden: false,
    }, "Insert chart", false);
  });
  await page.evaluate(() => window.__tally!.controller.grid!.scrollTo({ r: 0, c: 3 }));
  await expect(page.locator('.tally-grid-object[data-object-id^="chart:"] canvas')).toHaveCount(1, { timeout: 15_000 });
  await page.waitForTimeout(500);
}

test("chart over the grid", async ({ page }) => {
  await ready(page);
  await insertChart(page);
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${OUT}/v04-chart-grid.png` });
});

test("chart editor", async ({ page }) => {
  await ready(page);
  await insertChart(page);
  await page.evaluate(() => window.__tally!.controller.grid!.scrollTo({ r: 0, c: 5 }));
  await page.evaluate(() => (window.__tally!.controller as unknown as { w5: { charts: { openEditor(): void } } }).w5.charts.openEditor());
  await expect(page.getByTestId("chart-editor")).toBeVisible();
  await page.getByTestId("chart-section-axes").locator("summary").click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${OUT}/v04-chart-editor.png` });
});

test("print preview", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.__tally!.controller.registry.run("file.printPreview"));
  await expect(page.getByTestId("print-page")).toBeVisible();
  await page.waitForTimeout(500);
  await page.screenshot({ path: `${OUT}/v04-print-preview.png` });
});

test("page break preview", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => {
    const t = window.__tally!;
    const sel = { range: { r0: 8, c0: 0, r1: 8, c1: 0 }, active: { r: 8, c: 0 }, anchor: { r: 8, c: 0 } };
    (t.controller as unknown as { onSelectionChange(s: unknown): void }).onSelectionChange(sel);
  });
  await page.evaluate(() => window.__tally!.controller.registry.run("insert.pageBreak"));
  await page.evaluate(() => window.__tally!.controller.registry.run("view.pageBreakPreview"));
  await expect(page.getByTestId("grid")).toHaveAttribute("data-page-break-preview", "true");
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${OUT}/v04-page-break-preview.png` });
});
