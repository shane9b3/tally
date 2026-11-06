/**
 * T18 chart axes in the browser (WebMockClient emulates `chart_data`'s
 * date axis): a line chart over date cells gets AOO's automatic date axis
 * (an ECharts `time` axis, points placed by date with a gap for the
 * missing day, labels in the cells' date format); "Cross other axis at"
 * value 50 moves the category axis up to y = 50 after layout.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;
const DAY_MS = 86_400_000;

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page) {
  await page.goto("/");
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

async function typeInto(page: Page, r: number, c: number, text: string) {
  await clickCell(page, r, c);
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const chartNode = (page: Page) => page.locator('.tally-grid-object[data-object-id^="chart:"]');

/** 2026-01-05, -06, -08 and -09 (serials; the 7th is missing) with 20..80, formatted as dates. */
async function datesSheet(page: Page) {
  await typeInto(page, 0, 0, "Day");
  await typeInto(page, 0, 1, "Sales");
  const serials = [46027, 46028, 46030, 46031];
  for (const [i, s] of serials.entries()) {
    await typeInto(page, i + 1, 0, String(s));
    await typeInto(page, i + 1, 1, String((i + 1) * 20));
  }
  await clickCell(page, 1, 0);
  await clickCell(page, 4, 0, "Shift");
  await run(page, "format.number.date");
}

async function insertLineChart(page: Page): Promise<number> {
  await clickCell(page, 0, 0);
  await clickCell(page, 4, 1, "Shift");
  await page.locator('[data-command="insert.chart"]').click();
  await expect(chartNode(page)).toHaveCount(1);
  await expect(chartNode(page).locator("canvas")).toHaveCount(1, { timeout: 15_000 });
  await expect(page.getByTestId("chart-editor")).toBeVisible();
  await page.locator('[data-kind="line"]').click();
  return Number((await chartNode(page).getAttribute("data-object-id"))!.split(":")[1]);
}

interface AxisState {
  type: string;
  labels: string[];
  data: unknown[];
  onZero: boolean | undefined;
  offset: number | undefined;
}

const xAxisState = (page: Page, id: number): Promise<AxisState | null> =>
  page.evaluate((id) => {
    const opt = window.__tallyCharts?.[id]?.getOption() as
      | { xAxis: { type: string; offset?: number; axisLine?: { onZero?: boolean }; axisLabel?: { customValues?: number[]; formatter?: (v: number) => string } }[]; series: { type: string; data: unknown[] }[] }
      | undefined;
    const x = opt?.xAxis?.[0];
    if (!x) return null;
    const f = x.axisLabel?.formatter;
    return {
      type: x.type,
      labels: typeof f === "function" ? (x.axisLabel?.customValues ?? []).map((v) => f(v)) : [],
      data: opt.series?.[0]?.type === "line" ? (opt.series[0].data ?? []) : [],
      onZero: x.axisLine?.onZero,
      offset: x.offset,
    };
  }, id);

test("a line chart over dates gets AOO's automatic date axis: dated points, a gap, labels in the date format", async ({ page }) => {
  await ready(page);
  await datesSheet(page);
  const id = await insertLineChart(page);
  await expect.poll(async () => (await xAxisState(page, id))?.type).toBe("time");
  const s = (await xAxisState(page, id))!;
  // One label per day, the missing 7th included, in the cells' MM/DD/YYYY format.
  expect(s.labels).toEqual(["01/05/2026", "01/06/2026", "01/07/2026", "01/08/2026", "01/09/2026"]);
  const day = (d: number) => Date.UTC(2026, 0, d);
  expect(s.data).toEqual([
    [day(5), 20],
    [day(6), 40],
    [day(8), 60],
    [day(9), 80],
  ]);
  expect(day(9) - day(8)).toBe(DAY_MS);
  // Scale ▸ Type: Text gives back evenly spaced categories.
  await page.getByTestId("chart-section-axes").locator("summary").click();
  await page.getByRole("tab", { name: "X axis" }).click();
  await page.getByTestId("chart-axis-type").selectOption("text");
  await expect.poll(async () => (await xAxisState(page, id))?.type).toBe("category");
  const up = (await recorded(page)).filter((o) => o.type === "updateChart").at(-1) as { chart: { xAxis: { axisType: string } } };
  expect(up.chart.xAxis.axisType).toBe("text");
});

test("Cross other axis at value 50 moves the X axis up to 50 on the Y axis", async ({ page }) => {
  await ready(page);
  await datesSheet(page);
  const id = await insertLineChart(page);
  await expect.poll(async () => (await xAxisState(page, id))?.type).toBe("time");
  const before = (await xAxisState(page, id))!;
  expect(before.offset ?? 0).toBe(0);
  await page.getByTestId("chart-section-axes").locator("summary").click();
  await page.getByRole("tab", { name: "X axis" }).click();
  await expect(page.getByTestId("chart-axis-cross")).toHaveValue("value");
  await page.getByTestId("chart-axis-cross-value").fill("50");
  await page.getByTestId("chart-axis-cross-value").press("Enter");
  await expect
    .poll(async () => ((await recorded(page)).filter((o) => o.type === "updateChart").at(-1) as { chart: { xAxis: { crosses: unknown } } } | undefined)?.chart.xAxis.crosses)
    .toEqual({ type: "value", value: 50 });
  // The axis leaves its "on zero" place and is offset upwards to the pixel of y = 50.
  await expect.poll(async () => (await xAxisState(page, id))?.offset ?? 0).toBeLessThan(-5);
  const s = (await xAxisState(page, id))!;
  expect(s.onZero).toBe(false);
  const px = await page.evaluate((id) => {
    const inst = window.__tallyCharts![id]!;
    const y50 = inst.convertToPixel({ yAxisIndex: 0 }, 50) as number;
    const yMin = inst.convertToPixel({ yAxisIndex: 0 }, 0) as number;
    return { y50, yMin };
  }, id);
  // The Y axis runs from 0 here (values 20..80): its bottom edge is y = 0.
  expect(Math.abs(px.y50 - px.yMin - s.offset!)).toBeLessThan(2);
  // Start puts it back at the bottom edge, no offset needed.
  await page.getByTestId("chart-axis-cross").selectOption("start");
  await expect.poll(async () => (await xAxisState(page, id))?.offset ?? 0).toBe(0);
});

test("an out-of-range X minimum stays a number field; min ≥ max is flagged inline and not sent", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await ready(page);
  await datesSheet(page);
  const id = await insertLineChart(page);
  await expect.poll(async () => (await xAxisState(page, id))?.type).toBe("time");
  // X Min 1e9 on a scatter chart (a value axis), then back to a line over dates.
  await page.locator('[data-kind="scatter"]').click();
  await page.getByTestId("chart-section-axes").locator("summary").click();
  await page.getByRole("tab", { name: "X axis" }).click();
  await page.getByTestId("chart-axis-min").fill("1000000000");
  await page.getByTestId("chart-axis-min").press("Enter");
  await expect.poll(async () => ((await recorded(page)).filter((o) => o.type === "updateChart").at(-1) as { chart: { xAxis: { min: number | null } } } | undefined)?.chart.xAxis.min).toBe(1e9);
  await page.locator('[data-kind="line"]').click();
  await page.getByRole("tab", { name: "X axis" }).click();
  await page.getByTestId("chart-axis-type").selectOption("date");
  // The panel survives; the limit shows as a number, the maximum as a date field.
  await expect(page.getByTestId("chart-editor")).toBeVisible();
  await expect(page.getByTestId("chart-axis-min")).toHaveValue("1000000000");
  await expect(page.getByTestId("chart-axis-max-date")).toBeVisible();
  expect(errors).toEqual([]);
  // A maximum below the minimum: AOO's warning, no update sent.
  const sent = (await recorded(page)).filter((o) => o.type === "updateChart").length;
  await page.getByTestId("chart-axis-max-date").fill("2026-01-09");
  await page.getByTestId("chart-axis-max-date").blur();
  await expect(page.getByTestId("chart-axis-range-error")).toHaveText("The minimum must be lower than the maximum. Check your input.");
  expect((await recorded(page)).filter((o) => o.type === "updateChart").length).toBe(sent);
  // Clearing the minimum fixes it and commits (the field becomes a date field).
  await page.getByTestId("chart-axis-min").fill("");
  await expect(page.getByTestId("chart-axis-min-date")).toHaveValue("");
  await expect(page.getByTestId("chart-axis-range-error")).toHaveCount(0);
  await expect.poll(async () => ((await recorded(page)).filter((o) => o.type === "updateChart").at(-1) as { chart: { xAxis: { min: number | null; max: number | null } } }).chart.xAxis).toMatchObject({ min: null, max: 46031 });
});
