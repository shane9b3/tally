/**
 * W5-5 browser mock: chart ops in the side model (ids in `created`, series
 * derived from the wizard source, `chartsChanged` on edits, undo), chart
 * data (trend and error bars), `suggest_chart`, print layout and page
 * breaks, detective traces, through `WebMockClient`.
 */
import type { ChartDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { defaultChart } from "../src/charts/chartFeatures";
import { WebMockClient } from "../src/dev/webMock";
import { printPagesFor, refsIn, trend } from "../src/dev/mockW5";
import { defaultPageSetup } from "../src/v03";

async function invoices() {
  const client = new WebMockClient();
  const info = await client.newWorkbook("invoices");
  const name = info.sheets.find((s) => s.id === info.activeSheet)?.name ?? "Sheet1";
  return { client, doc: info.doc, sheet: info.activeSheet, name };
}

const tx = (ops: Parameters<WebMockClient["apply"]>[1]["ops"], label = "t") => ({ ops, label, selectionBefore: null, selectionAfter: null });

describe("charts in the browser mock", () => {
  it("addChart returns the id, derives series from the source and reports objectsChanged; undo removes it", async () => {
    const { client, doc, sheet, name } = await invoices();
    const chart = defaultChart(name, { r0: 0, c0: 0, r1: 4, c1: 2 });
    const res = await client.apply(doc, tx([{ type: "addChart", sheet, chart }], "Insert chart"));
    expect(res.errors).toEqual([]);
    expect(res.created).toEqual([{ opIndex: 0, id: 1 }]);
    expect(res.objectsChanged).toEqual([sheet]);
    const list = await client.charts(doc, sheet);
    expect(list).toHaveLength(1);
    expect(list[0]?.series.length).toBe(2);
    expect(list[0]?.categories).toBe(`$${name}.$A$2:$A$5`);
    expect(list[0]?.series[0]?.name).toEqual({ type: "ref", ref: `$${name}.$B$1` });
    const data = await client.chartData(doc, sheet, 1);
    expect(data.series).toHaveLength(2);
    expect(data.categories).toHaveLength(4);
    expect(data.series[0]?.values).toHaveLength(4);
    await client.undo(doc);
    expect(await client.charts(doc, sheet)).toEqual([]);
    await client.redo(doc);
    expect(await client.charts(doc, sheet)).toHaveLength(1);
  });

  it("updateChart keeps the id; an edit inside a series range reports chartsChanged; delete works", async () => {
    const { client, doc, sheet, name } = await invoices();
    // Columns A:C only: the template's formulas in F are recomputed (and reported) on every edit.
    const chart = defaultChart(name, { r0: 0, c0: 0, r1: 4, c1: 2 });
    await client.apply(doc, tx([{ type: "addChart", sheet, chart }]));
    const [stored] = await client.charts(doc, sheet);
    const edited: ChartDto = { ...stored!, title: "Totals" };
    const up = await client.apply(doc, tx([{ type: "updateChart", sheet, id: 1, chart: edited }], "Edit chart"));
    expect(up.errors).toEqual([]);
    expect((await client.charts(doc, sheet))[0]?.title).toBe("Totals");
    const typed = await client.apply(doc, tx([{ type: "setCellInput", sheet, r: 2, c: 1, input: "999" }], "Typing"));
    expect(typed.chartsChanged).toEqual([{ sheet, id: 1 }]);
    const elsewhere = await client.apply(doc, tx([{ type: "setCellInput", sheet, r: 40, c: 12, input: "x" }], "Typing"));
    expect(elsewhere.chartsChanged).toBeUndefined();
    const moved = await client.apply(doc, tx([{ type: "moveResizeChart", sheet, id: 1, anchor: { ...chart.anchor, r: 9 } }]));
    expect(moved.objectsChanged).toEqual([sheet]);
    expect((await client.charts(doc, sheet))[0]?.anchor.r).toBe(9);
    await client.apply(doc, tx([{ type: "deleteChart", sheet, id: 1 }]));
    expect(await client.charts(doc, sheet)).toEqual([]);
    const bad = await client.apply(doc, tx([{ type: "deleteChart", sheet, id: 1 }]));
    expect(bad.errors[0]?.code).toBe("not_found");
  });

  it("suggest_chart detects the region around one cell and the labels", async () => {
    const { client, doc, sheet, name } = await invoices();
    const c = await client.suggestChart(doc, sheet, { r0: 2, c0: 2, r1: 2, c1: 2 }, "column");
    expect(c.source?.range).toMatch(new RegExp(`^\\$${name}\\.\\$A\\$1:\\$G\\$\\d+$`));
    expect(c.source?.firstRowLabel).toBe(true);
    expect(c.anchor.widthMm).toBe(160);
    expect(c.anchor.c).toBe(7);
  });

  it("computes trend lines and error bars in chart data", async () => {
    const { client, doc, sheet, name } = await invoices();
    await client.apply(doc, tx([{ type: "setRangeInputs", sheet, r: 30, c: 0, rows: [["1"], ["2"], ["3"], ["4"]] }]));
    const chart: ChartDto = { ...defaultChart(name, { r0: 30, c0: 0, r1: 33, c1: 0 }), source: null, series: [{ name: { type: "text", text: "S" }, values: `$${name}.$A$31:$A$34`, xValues: null, sizes: null, kind: null, secondaryAxis: false, color: null, marker: null, labels: { value: false, percent: false, category: false, seriesName: false, position: "auto" }, trend: { kind: "linear", showEquation: true, showR2: true, name: null }, errorBars: { kind: { type: "stdDev", k: 1 }, direction: "both" }, meanLine: true }] };
    await client.apply(doc, tx([{ type: "addChart", sheet, chart }]));
    const [stored] = await client.charts(doc, sheet);
    const data = await client.chartData(doc, sheet, stored!.id);
    const s = data.series[0]!;
    expect(s.values).toEqual([1, 2, 3, 4]);
    expect(s.trend?.r2).toBe(1);
    expect(s.trend?.points).toEqual([[1, 1], [4, 4]]);
    expect(s.mean).toBe(2.5);
    expect(s.errors?.[0]?.[0]).toBeCloseTo(1.291, 3);
    expect(trend("exponential", [1, 2, 3], [2, 4, 8])?.equation).toContain("exp(");
    expect(trend("linear", [1, 1], [1, 2])).toBeNull();
  });

  it("images: add returns an id, the list omits the bytes, undo removes it", async () => {
    const { client, doc, sheet } = await invoices();
    const res = await client.apply(doc, tx([{ type: "addImage", sheet, image: { id: 0, name: "logo.png", anchor: { r: 1, c: 1, dxMm: 0, dyMm: 0, widthMm: 40, heightMm: 30 }, mime: "image/png", data: "iVBORw0KGgo=" } }]));
    expect(res.created?.[0]?.id).toBe(1);
    const [im] = await client.images(doc, sheet);
    expect(im?.name).toBe("logo.png");
    expect(im?.data).toBe("");
    await client.undo(doc);
    expect(await client.images(doc, sheet)).toEqual([]);
  });
});

describe("print and page breaks in the browser mock", () => {
  it("paginates the used area and prepares a job with pages", async () => {
    const { client, doc, sheet } = await invoices();
    const layout = await client.printLayout(doc, { sheets: { type: "active" }, selection: null, suppressEmptyPages: true, pageFrom: null, pageTo: null });
    expect(layout.pages.length).toBeGreaterThanOrEqual(1);
    expect(layout.pages[0]).toMatchObject({ index: 0, number: 1, sheet, paperMm: [210, 297] });
    const job = await client.preparePrintJob(doc, { sheets: { type: "active" }, selection: null, suppressEmptyPages: true, pageFrom: null, pageTo: null }, []);
    expect(job.job).toBe(1);
    expect(job.layout.pages.length).toBe(layout.pages.length);
    const png = await client.printJobPagePng(job.job, 0, 48);
    expect(png.length).toBeGreaterThan(20);
    await expect(client.printJobPagePng(99, 0, 48)).rejects.toMatchObject({ code: "not_found" });
    const breaks = await client.pageBreaks(doc, sheet);
    expect(breaks.area).not.toBeNull();
    await client.apply(doc, tx([{ type: "setPageBreak", sheet, axis: "row", at: 5, present: true }]));
    expect((await client.pageBreaks(doc, sheet)).rowBreaks).toContainEqual({ at: 5, manual: true });
  });

  it("printPagesFor orders pages top-to-bottom then right, or left-to-right", () => {
    const layout = { used: { r0: 0, c0: 0, r1: 199, c1: 19 }, merges: [], frozen: null, rowRuns: [], colRuns: [], defaultRowHeightPx: 24, defaultColWidthPx: 100 };
    const down = printPagesFor(1, defaultPageSetup(), layout as never, 0, 1);
    expect(down.length).toBeGreaterThan(2);
    expect(down[0]?.cols).toEqual(down[1]?.cols);
    const across = printPagesFor(1, { ...defaultPageSetup(), pageOrder: "leftToRight" }, layout as never, 0, 1);
    expect(across[0]?.rows).toEqual(across[1]?.rows);
    expect(across.length).toBe(down.length);
  });
});

describe("detective in the browser mock", () => {
  it("parses references and finds precedents and dependents", async () => {
    expect(refsIn("=SUM(A1:B2)+Sheet2.C3-LOG10(D4)", 1, (n) => (n === "sheet2" ? 2 : 1)).map((r) => [r.sheet, r.range.r0, r.range.c0, r.range.c1])).toEqual([
      [1, 0, 0, 1],
      [2, 2, 2, 2],
      [1, 3, 3, 3],
    ]);
    expect(refsIn("hello", 1, () => 1)).toEqual([]);
    const { client, doc, sheet } = await invoices();
    await client.apply(doc, tx([{ type: "setCellInput", sheet, r: 30, c: 0, input: "=B2+B3" }]));
    const pre = await client.precedents(doc, sheet, 30, 0);
    expect(pre.map((t) => t.range)).toEqual([
      { r0: 1, c0: 1, r1: 1, c1: 1 },
      { r0: 2, c0: 1, r1: 2, c1: 1 },
    ]);
    const dep = await client.dependents(doc, sheet, 1, 1);
    expect(dep.some((t) => t.range.r0 === 30 && t.range.c0 === 0)).toBe(true);
  });
});

describe("detective commands in the browser mock (engine detective.rs port)", () => {
  const pairs = (arrows: { from: { r0: number; c0: number } | null; to: { r: number; c: number } | null }[]) =>
    arrows.map((a) => `${a.from ? `${a.from.r0},${a.from.c0}` : "*"}>${a.to ? `${a.to.r},${a.to.c}` : "*"}`).sort();
  it("adds a level per trace, removes the outermost, and AutoRefresh replays after edits", async () => {
    const client = new WebMockClient();
    const info = await client.newWorkbook();
    const doc = info.doc;
    const sheet = info.sheets[0]!.id;
    const set = (r: number, c: number, input: string) => client.apply(doc, tx([{ type: "setCellInput", sheet, r, c, input }]));
    await set(0, 0, "1");
    await set(0, 1, "=A1");
    await set(0, 2, "=B1");
    let st = await client.detective(doc, sheet, 0, 2, "tracePrecedents");
    expect(st.done).toBe(true);
    expect(pairs(st.arrows)).toEqual(["0,1>0,2"]);
    st = await client.detective(doc, sheet, 0, 2, "tracePrecedents");
    expect(pairs(st.arrows)).toEqual(["0,0>0,1", "0,1>0,2"]);
    expect((await client.detective(doc, sheet, 0, 2, "tracePrecedents")).done).toBe(false);
    st = await client.detective(doc, sheet, 0, 2, "removePrecedents");
    expect(pairs(st.arrows)).toEqual(["0,1>0,2"]);
    // AutoRefresh: the formula now reads D1 as well.
    const res = await set(0, 2, "=B1+D1");
    expect(res.detectiveChanged).toBe(true);
    expect(pairs((await client.detectiveArrows(doc, sheet)).arrows)).toEqual(["0,1>0,2", "0,3>0,2"]);
    // Off: stale until Refresh Traces.
    await client.setDetectiveAuto(doc, false);
    await set(0, 2, "=B1");
    expect(pairs((await client.detectiveArrows(doc, sheet)).arrows)).toEqual(["0,1>0,2", "0,3>0,2"]);
    st = await client.detective(doc, sheet, 0, 0, "refresh");
    expect(pairs(st.arrows)).toEqual(["0,1>0,2"]);
    st = await client.detective(doc, sheet, 0, 0, "removeAll");
    expect(st.arrows).toEqual([]);
    expect(st.hasOps).toBe(false);
  });
});
