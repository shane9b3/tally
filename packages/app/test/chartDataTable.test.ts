/** The Chart Data Table model (T18; AOO DataBrowserModel) and the mock's `setChartDataTable`. */
import type { ChartDataTableDto, ChartDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { defaultChart } from "../src/charts/chartFeatures";
import { cellValue, dataColumns, deleteRow, deleteSeries, formatCell, headerGroups, insertRow, insertSeries, mayDeleteRow, mayMoveRowDown, moveRowDown, moveSeriesRight, parseCell, setCell, setSeriesName, showsCategories } from "../src/charts/dataTable";
import { applyW5Op, chartDataFor, emptyW5, isOwnData } from "../src/dev/mockW5";

const table = (): ChartDataTableDto => ({
  categories: ["Q1", "Q2", "Q3"],
  series: [
    { source: 0, name: "North", values: [1, 2, 3] },
    { source: 1, name: "South", values: [4, null, 6] },
  ],
});

function own(kind: ChartDto["kind"], t: ChartDataTableDto | null): ChartDto {
  const c = defaultChart("Sheet1", { r0: 0, c0: 0, r1: 0, c1: 0 }, kind);
  return { ...c, source: null, series: [], dataTable: t };
}

describe("columns", () => {
  it("category charts show the categories, then one Y column per series", () => {
    const c = own("column", table());
    expect(dataColumns(c, table())).toEqual([
      { series: -1, role: "categories" },
      { series: 0, role: "y" },
      { series: 1, role: "y" },
    ]);
    expect(headerGroups(dataColumns(c, table()))).toEqual([
      { series: -1, start: 0, span: 1 },
      { series: 0, start: 1, span: 1 },
      { series: 1, start: 2, span: 1 },
    ]);
  });

  it("XY charts have no category column; bubbles group X, Y and sizes", () => {
    const t: ChartDataTableDto = { categories: ["", ""], series: [{ source: 0, name: "B", values: [1, 2], x: [3, 4], sizes: [5, 6] }] };
    const c = own("bubble", t);
    expect(showsCategories(c)).toBe(false);
    expect(dataColumns(c, t).map((x) => x.role)).toEqual(["x", "y", "size"]);
    expect(headerGroups(dataColumns(c, t))).toEqual([{ series: 0, start: 0, span: 3 }]);
  });

  it("stock series are labelled with their roles", () => {
    const t: ChartDataTableDto = { categories: ["d"], series: ["l", "h", "c"].map((n, i) => ({ source: i, name: n, values: [i] })) };
    const c = { ...own("stock", t), stock: "hlc" as const };
    expect(dataColumns(c, t).map((x) => x.role)).toEqual(["categories", "low", "high", "close"]);
  });
});

describe("edits", () => {
  it("Insert Row adds an empty row after the cursor row; Delete Row keeps one row", () => {
    const t = insertRow(table(), 0);
    expect(t.categories).toEqual(["Q1", "", "Q2", "Q3"]);
    expect(t.series[1]?.values).toEqual([4, null, null, 6]);
    expect(deleteRow(t, 1)).toEqual(table());
    const one: ChartDataTableDto = { categories: ["a"], series: [{ name: "", values: [1] }] };
    expect(mayDeleteRow(one, 0)).toBe(false);
    expect(deleteRow(one, 0)).toBe(one);
    expect(insertRow({ categories: [], series: [{ name: "", values: [] }] }, -1).categories).toEqual([""]);
  });

  it("Move Row Down swaps with the next row, not past the last", () => {
    const t = moveRowDown(table(), 0);
    expect(t.categories).toEqual(["Q2", "Q1", "Q3"]);
    expect(t.series[0]?.values).toEqual([2, 1, 3]);
    expect(mayMoveRowDown(table(), 2)).toBe(false);
  });

  it("Insert / Delete / Move Series keep the sources", () => {
    const c = own("column", table());
    const t = insertSeries(table(), c, 0);
    expect(t.series.map((s) => s.source ?? null)).toEqual([0, null, 1]);
    expect(t.series[1]?.values).toEqual([null, null, null]);
    expect(insertSeries(table(), c, -1).series[0]?.source).toBeNull();
    expect(deleteSeries(table(), 0).series.map((s) => s.name)).toEqual(["South"]);
    expect(moveSeriesRight(table(), 0).series.map((s) => s.source)).toEqual([1, 0]);
    expect(moveSeriesRight(table(), 1)).toEqual(table());
  });

  it("an XY series inserted copies its neighbour's X values (AOO shares the X sequence)", () => {
    const t: ChartDataTableDto = { categories: ["", ""], series: [{ source: 0, name: "", values: [1, 2], x: [10, 20] }] };
    const n = insertSeries(t, own("scatter", t), 0);
    expect(n.series[1]?.x).toEqual([10, 20]);
    expect(n.series[1]?.values).toEqual([null, null]);
  });

  it("cells and names", () => {
    const cols = dataColumns(own("column", table()), table());
    let t = setCell(table(), cols[0]!, 1, "Spring");
    t = setCell(t, cols[2]!, 1, 5.5);
    t = setSeriesName(t, 0, "N");
    expect(cellValue(t, cols[0]!, 1)).toBe("Spring");
    expect(cellValue(t, cols[2]!, 1)).toBe(5.5);
    expect(t.series[0]?.name).toBe("N");
    // The input table is not changed.
    expect(table().categories[1]).toBe("Q2");
  });
});

describe("numbers", () => {
  it("parses numbers, empties and the locale's decimal separator", () => {
    expect(parseCell("12.5")).toBe(12.5);
    expect(parseCell(" -3 ")).toBe(-3);
    expect(parseCell("1,234.5")).toBe(1234.5);
    expect(parseCell("50%")).toBe(0.5);
    expect(parseCell("")).toBeNull();
    expect(parseCell("abc")).toBeUndefined();
    expect(parseCell("1.234,5", ",")).toBe(1234.5);
    expect(formatCell(1.5, ",")).toBe("1,5");
    expect(formatCell(null)).toBe("");
  });
});

describe("mock", () => {
  const cells = async () => [];
  const sheets = () => 0;

  it("adds an own-data chart, edits its table and refuses charts over cells", async () => {
    const x = emptyW5();
    const chart = own("column", table());
    chart.series = [0, 1].map(() => ({ name: null, values: "", xValues: null, sizes: null, kind: null, secondaryAxis: false, color: null, marker: null, labels: { value: false, percent: false, category: false, seriesName: false, position: "auto" }, trend: null, errorBars: null, meanLine: false }));
    chart.series[0]!.color = 0xff0000ff;
    const added = await applyW5Op(x, { type: "addChart", sheet: 0, chart }, cells, sheets);
    expect(added.kind).toBe("handled");
    const c = x.charts[0]![0]!;
    expect(isOwnData(c)).toBe(true);
    const next = moveSeriesRight(insertRow(c.dataTable!, 2), 0);
    expect((await applyW5Op(x, { type: "setChartDataTable", sheet: 0, id: c.id, table: next }, cells, sheets)).kind).toBe("handled");
    const after = x.charts[0]![0]!;
    expect(after.series[1]?.color).toBe(0xff0000ff);
    const data = await chartDataFor(0, after, 1, cells, sheets);
    expect(data.categories).toEqual(["Q1", "Q2", "Q3", ""]);
    expect(data.series[0]?.name).toBe("South");
    expect(data.series[1]?.values).toEqual([1, 2, 3, null]);
    const cellChart = defaultChart("Sheet1", { r0: 0, c0: 0, r1: 2, c1: 1 });
    await applyW5Op(x, { type: "addChart", sheet: 0, chart: { ...cellChart, series: [{ ...chart.series[0]!, values: "$Sheet1.$B$1:$B$3" }] } }, cells, sheets);
    const ranged = x.charts[0]![1]!;
    expect(ranged.dataTable).toBeNull();
    const refused = await applyW5Op(x, { type: "setChartDataTable", sheet: 0, id: ranged.id, table: table() }, cells, sheets);
    expect(refused.kind).toBe("error");
  });
});
