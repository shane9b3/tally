/**
 * W5-5 charts in the browser (WebMockClient emulates the chart ops and
 * `chart_data`): insert from the toolbar over the invoices template, an
 * ECharts canvas appears in the object layer, the editor renames the title,
 * typing in a series cell refreshes the data, drag + ⌘Z, delete + ⌘Z, range
 * picking, and the detective arrows.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;

interface Rec {
  type: string;
  [k: string]: unknown;
}

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

async function clickCell(page: Page, r: number, c: number, modifier?: "Shift") {
  const p = await cellXY(page, r, c);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(p.x, p.y);
  if (modifier) await page.keyboard.up(modifier);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const chartNode = (page: Page) => page.locator('.tally-grid-object[data-object-id^="chart:"]');

async function insertChart(page: Page, c1 = 2) {
  await clickCell(page, 0, 0);
  await clickCell(page, 5, c1, "Shift");
  await page.locator('[data-command="insert.chart"]').click();
  await expect(chartNode(page)).toHaveCount(1);
  await expect(chartNode(page).locator("canvas")).toHaveCount(1, { timeout: 15_000 });
  await expect(page.getByTestId("chart-editor")).toBeVisible();
  return Number((await chartNode(page).getAttribute("data-object-id"))!.split(":")[1]);
}

const seriesCount = (page: Page, id: number) =>
  page.evaluate((id) => {
    const inst = window.__tallyCharts?.[id];
    const opt = inst?.getOption() as { series?: unknown[] } | undefined;
    return opt?.series?.length ?? -1;
  }, id);

test("insert a column chart from the toolbar: an ECharts canvas in the object layer with two series", async ({ page }) => {
  await ready(page);
  const id = await insertChart(page);
  expect(id).toBeGreaterThan(0);
  await expect.poll(() => seriesCount(page, id)).toBe(2);
  const ops = await recorded(page);
  const add = ops.find((o) => o.type === "addChart") as { chart: { kind: string; source: { range: string } } } | undefined;
  expect(add?.chart.kind).toBe("column");
  expect(add?.chart.source.range).toMatch(/\$A\$1:\$C\$6$/);
  await expect(chartNode(page)).toHaveAttribute("data-selected", "true");
  await expect(chartNode(page).locator("[data-handle]")).toHaveCount(8);
});

test("the editor renames the title and switches the type (one updateChart each)", async ({ page }) => {
  await ready(page);
  const id = await insertChart(page);
  await page.getByTestId("chart-title").fill("Invoices by customer");
  await page.getByTestId("chart-title").press("Enter");
  await expect.poll(async () => (await recorded(page)).filter((o) => o.type === "updateChart").length).toBe(1);
  const up = (await recorded(page)).find((o) => o.type === "updateChart") as { chart: { title: string } };
  expect(up.chart.title).toBe("Invoices by customer");
  await expect.poll(() => page.evaluate((id) => (window.__tallyCharts?.[id]?.getOption() as { title?: { text: string }[] })?.title?.[0]?.text, id)).toBe("Invoices by customer");
  await page.locator('[data-kind="line"]').click();
  await expect.poll(async () => (await recorded(page)).filter((o) => o.type === "updateChart").length).toBe(2);
  await expect.poll(() => page.evaluate((id) => (window.__tallyCharts?.[id]?.getOption() as { series?: { type: string }[] })?.series?.[0]?.type, id)).toBe("line");
});

test("typing in a series cell refreshes the chart data", async ({ page }) => {
  await ready(page);
  // A1:E6: series B (Customer), C (Item), D (Qty), E (Price); typing goes into D2.
  const id = await insertChart(page, 4);
  const first = () => page.evaluate((id) => (window.__tallyCharts?.[id]?.getOption() as { series?: { data: (number | null)[] }[] })?.series?.[2]?.data?.[0] ?? null, id);
  await expect.poll(first).not.toBeNull();
  await clickCell(page, 1, 3);
  await page.keyboard.type("4242");
  await page.keyboard.press("Enter");
  await expect.poll(first).toBe(4242);
});

test("drag moves the chart with one op; ⌘Z restores the anchor", async ({ page }) => {
  await ready(page);
  await insertChart(page);
  const node = chartNode(page);
  const box = (await node.boundingBox())!;
  // Near the top-left corner: the chart's right part can sit under the editor panel.
  await page.mouse.move(box.x + 40, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 40 + 60, box.y + 20 + 30, { steps: 6 });
  await page.mouse.move(box.x + 40 + 120, box.y + 20 + 48, { steps: 6 });
  await page.mouse.up();
  await expect.poll(async () => (await recorded(page)).filter((o) => o.type === "moveResizeChart").length).toBe(1);
  const after = (await node.boundingBox())!;
  expect(after.x).toBeGreaterThan(box.x + 100);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await node.boundingBox())!.x).toBeCloseTo(box.x, 0);
});

test("Delete removes the chart; ⌘Z brings it back; the panel's Delete works too", async ({ page }) => {
  await ready(page);
  await insertChart(page);
  await chartNode(page).click({ position: { x: 40, y: 20 } });
  await page.keyboard.press("Delete");
  await expect(chartNode(page)).toHaveCount(0);
  await expect(page.getByTestId("chart-editor")).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+z");
  await expect(chartNode(page)).toHaveCount(1);
  await chartNode(page).dblclick({ position: { x: 40, y: 20 } });
  await expect(page.getByTestId("chart-editor")).toBeVisible();
  await page.getByTestId("chart-delete").click();
  await expect(chartNode(page)).toHaveCount(0);
});

test("pick a series range with point mode", async ({ page }) => {
  await ready(page);
  await insertChart(page);
  const values = page.getByTestId("chart-series-0-values");
  const before = await values.inputValue();
  await page.getByTestId("chart-series-0").getByRole("button", { name: "Pick range" }).click();
  await expect(page.locator(".tally-grid[data-range-pick]")).toHaveCount(1);
  // Column A (the chart sits over D:I).
  const a = await cellXY(page, 1, 0);
  const b = await cellXY(page, 6, 0);
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await page.mouse.move(b.x, b.y, { steps: 4 });
  await page.mouse.up();
  await expect(values).not.toHaveValue(before);
  await expect(values).toHaveValue(/\$A\$2:\$A\$7$/);
  await expect.poll(async () => ((await recorded(page)).at(-1) as { type: string }).type).toBe("updateChart");
});

test("trace precedents draws arrows; Remove All Traces clears them", async ({ page }) => {
  await ready(page);
  await clickCell(page, 1, 5);
  const traces = () => page.evaluate(() => (window.__tally!.controller as unknown as { w5: { state(): { traces: number } } }).w5.state().traces);
  await page.evaluate(() => window.__tally!.controller.registry.run("tools.tracePrecedents"));
  await expect.poll(traces).toBeGreaterThan(0);
  await page.evaluate(() => window.__tally!.controller.registry.run("tools.traceDependents"));
  await page.evaluate(() => window.__tally!.controller.registry.run("tools.removeTraces"));
  await expect.poll(traces).toBe(0);
});
