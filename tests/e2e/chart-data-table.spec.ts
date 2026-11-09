/**
 * T18 Chart Data Table in the browser (WebMockClient): a chart with its
 * own data (a pasted chart that reads no cells) opens AOO's Data Table
 * from the chart editor; a value edited with the keyboard and an inserted
 * row reach the chart as one `setChartDataTable` when the dialog closes,
 * and one undo puts the data back. Charts over cells have no data table.
 * Review 5: the table is read when the dialog opens (never in the chart
 * list), copy / paste still carries it, Move Series Right keeps a pending
 * cell edit, and a long table renders only the rows in view.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const chartNode = (page: Page) => page.locator('.tally-grid-object[data-object-id^="chart:"]');

/** The first series' bar values as drawn. */
const drawn = (page: Page, id: number) =>
  page.evaluate((id) => {
    const opt = window.__tallyCharts?.[id]?.getOption() as { series?: { data: unknown[] }[] } | undefined;
    return (opt?.series?.[0]?.data ?? []).map((d) => (Array.isArray(d) ? d[1] : typeof d === "object" && d !== null && "value" in d ? (d as { value: unknown }).value : d));
  }, id);

type Table = { categories: string[]; series: { source: number; name: string; values: (number | null)[] }[] };
const UNITS: Table = { categories: ["Q1", "Q2", "Q3"], series: [{ source: 0, name: "Units", values: [1, 2, 3] }] };

/** Inserts a chart over typed cells, then pastes a copy that carries its own data (default Q1..Q3: 1, 2, 3). */
async function ownDataChart(page: Page, table: Table = UNITS): Promise<{ id: number; cellChart: number }> {
  const run = (id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);
  await page.evaluate(async () => {
    const ctl = window.__tally!.controller;
    await ctl.apply("data", [
      { type: "setCellInput", sheet: ctl.sheet, r: 0, c: 0, input: "Q" },
      { type: "setCellInput", sheet: ctl.sheet, r: 0, c: 1, input: "Sales" },
      { type: "setCellInput", sheet: ctl.sheet, r: 1, c: 0, input: "Q1" },
      { type: "setCellInput", sheet: ctl.sheet, r: 1, c: 1, input: "5" },
    ]);
  });
  await run("insert.chart");
  await expect(chartNode(page)).toHaveCount(1);
  const cellChart = Number((await chartNode(page).first().getAttribute("data-object-id"))!.split(":")[1]);
  const id = await page.evaluate(async (table) => {
    const charts = window.__tally!.controller.w5.charts;
    const c = charts.selected!;
    const one = { ...c.series[0]!, values: "", name: null, xValues: null, sizes: null };
    const series = table.series.map(() => ({ ...one }));
    const copy = {
      ...c,
      id: 0,
      source: null,
      categories: null,
      series,
      anchor: { ...c.anchor, r: c.anchor.r + 12 },
      dataTable: table,
    };
    return await charts.addChart(copy, "Paste chart", true);
  }, table);
  expect(id).not.toBeNull();
  await expect(chartNode(page)).toHaveCount(2);
  if (table.categories.length <= 10) await expect.poll(() => drawn(page, id!)).toEqual(table.series[0]!.values);
  return { id: id!, cellChart };
}

test("the Data Table edits a chart's own data with the keyboard and applies it on close", async ({ page }) => {
  await ready(page);
  const { id } = await ownDataChart(page);
  await expect(page.getByTestId("chart-editor")).toBeVisible();
  await page.getByTestId("chart-data-table-open").click();
  const dlg = page.getByTestId("chart-data-table");
  await expect(dlg).toBeVisible();
  await expect(dlg.getByRole("grid")).toBeVisible();
  await expect(dlg.locator('th[role="columnheader"]')).toHaveText(["Categories", "Y-Values"]);
  await expect(dlg.getByTestId("dt-name-0")).toHaveValue("Units");
  // The cursor starts on the first category; → ↓ moves to Q2's value.
  await expect(dlg.getByTestId("dt-cell-0-0")).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowDown");
  await expect(dlg.getByTestId("dt-cell-1-1")).toBeFocused();
  // Text in a value cell is refused (AOO: "Numbers are required").
  await page.keyboard.press("Enter");
  await page.getByTestId("dt-edit").fill("lots");
  await page.keyboard.press("Enter");
  await expect(dlg.getByRole("alert")).toContainText("Numbers are required");
  await page.getByTestId("dt-edit").fill("42");
  await page.keyboard.press("Enter");
  await expect(dlg.getByTestId("dt-cell-1-1")).toHaveText("42");
  // Typing replaces a cell: the category of row 3.
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.type("Autumn");
  await page.keyboard.press("Enter");
  await expect(dlg.getByTestId("dt-cell-2-0")).toHaveText("Autumn");
  // Insert Row after the cursor row; Delete Row is on with several rows.
  await dlg.getByTestId("dt-insertRow").click();
  await expect(dlg.locator("tbody tr")).toHaveCount(4);
  await expect(dlg.getByTestId("dt-moveSeriesRight")).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(dlg).toBeHidden();
  const set = (await recorded(page)).filter((o) => o.type === "setChartDataTable");
  expect(set).toHaveLength(1);
  expect(set[0]).toMatchObject({ id, table: { categories: ["Q1", "Q2", "Autumn", ""], series: [{ name: "Units", values: [1, 42, 3, null] }] } });
  await expect.poll(() => drawn(page, id)).toEqual([1, 42, 3, null]);
  // One undo step ("Edit chart data") puts the data back.
  await page.evaluate(() => window.__tally!.controller.registry.run("edit.undo"));
  await expect.poll(() => drawn(page, id)).toEqual([1, 2, 3]);
});

test("axe: the Data Table dialog (light and dark)", async ({ page }) => {
  await ready(page);
  await ownDataChart(page);
  await page.getByTestId("chart-data-table-open").click();
  await expect(page.getByTestId("chart-data-table")).toBeVisible();
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
    const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).include("[data-testid=chart-data-table]").analyze();
    const bad = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => `${scheme} ${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`);
    expect(bad).toEqual([]);
  }
});

test("charts over cells have no data table (AOO edits the cells)", async ({ page }) => {
  await ready(page);
  const { cellChart } = await ownDataChart(page);
  const enabled = () => page.evaluate(() => window.__tally!.controller.registry.isEnabled("chart.dataTable"));
  expect(await enabled()).toBe(true);
  await page.evaluate((id) => window.__tally!.controller.grid?.objectLayer.select(`chart:${id}`, true), cellChart);
  await expect(page.getByTestId("chart-editor")).toBeVisible();
  await expect(page.getByTestId("chart-data-table-open")).toHaveCount(0);
  expect(await enabled()).toBe(false);
});

test("the chart list never carries the table; copy and paste bring it along", async ({ page }) => {
  await ready(page);
  const { id } = await ownDataChart(page);
  const listed = await page.evaluate(() => window.__tally!.controller.w5.charts.selected);
  expect(listed?.ownData).toBe(true);
  expect(listed?.dataTable ?? null).toBeNull();
  const pasted = await page.evaluate(async () => {
    const charts = window.__tally!.controller.w5.charts;
    await charts.copySelected();
    await charts.pasteChart();
    return charts.selected?.id ?? null;
  });
  await expect(chartNode(page)).toHaveCount(3);
  const adds = (await recorded(page)).filter((o) => o.type === "addChart");
  expect(adds.at(-1)).toMatchObject({ chart: { dataTable: { categories: ["Q1", "Q2", "Q3"], series: [{ name: "Units", values: [1, 2, 3] }] } } });
  const newest = await page.evaluate(() => Math.max(...[...document.querySelectorAll('.tally-grid-object[data-object-id^="chart:"]')].map((n) => Number(n.getAttribute("data-object-id")!.split(":")[1]))));
  expect(newest).not.toBe(id);
  void pasted;
  await expect.poll(() => drawn(page, newest)).toEqual([1, 2, 3]);
});

test("Move Series Right keeps a cell edit that is still open (review 5, M1)", async ({ page }) => {
  await ready(page);
  const { id } = await ownDataChart(page, {
    categories: ["Q1", "Q2"],
    series: [
      { source: 0, name: "A", values: [1, 2] },
      { source: 1, name: "B", values: [3, 4] },
    ],
  });
  await page.getByTestId("chart-data-table-open").click();
  const dlg = page.getByTestId("chart-data-table");
  await expect(dlg.getByTestId("dt-cell-0-0")).toBeFocused();
  // Series A's first value: type 42, no Enter, then click the tool.
  await page.keyboard.press("ArrowRight");
  await page.keyboard.type("42");
  await expect(page.getByTestId("dt-edit")).toHaveValue("42");
  await dlg.getByTestId("dt-moveSeriesRight").click();
  await expect(dlg.getByTestId("dt-name-1")).toHaveValue("A");
  await expect(dlg.getByTestId("dt-cell-0-2")).toHaveText("42");
  await dlg.getByTestId("dt-close").click();
  await expect(dlg).toBeHidden();
  const set = (await recorded(page)).filter((o) => o.type === "setChartDataTable");
  expect(set.at(-1)).toMatchObject({ id, table: { series: [{ name: "B", values: [3, 4] }, { name: "A", values: [42, 2] }] } });
});

test("a long table renders only the rows in view", async ({ page }) => {
  await ready(page);
  const n = 3000;
  const table: Table = { categories: Array.from({ length: n }, (_, i) => `R${i + 1}`), series: [{ source: 0, name: "V", values: Array.from({ length: n }, (_, i) => i) }] };
  await ownDataChart(page, table);
  await page.getByTestId("chart-data-table-open").click();
  const dlg = page.getByTestId("chart-data-table");
  await expect(dlg.getByTestId("dt-cell-0-0")).toBeFocused();
  expect(await dlg.locator("tbody tr[aria-rowindex]").count()).toBeLessThan(120);
  await expect(dlg.getByRole("grid")).toHaveAttribute("aria-rowcount", String(n + 2));
  // Ctrl+End reaches the last row, which is rendered and focused.
  await page.keyboard.press("Control+End");
  await expect(dlg.getByTestId(`dt-cell-${n - 1}-1`)).toBeFocused();
  await expect(dlg.getByTestId(`dt-cell-${n - 1}-1`)).toHaveText(String(n - 1));
  expect(await dlg.locator("tbody tr[aria-rowindex]").count()).toBeLessThan(120);
  // Scrolling back up renders the top rows again.
  await page.keyboard.press("Control+Home");
  await expect(dlg.getByTestId("dt-cell-0-0")).toBeFocused();
});
