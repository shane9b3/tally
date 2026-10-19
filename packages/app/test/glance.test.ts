import type { CellView } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { CURRENCY_FORMAT } from "../src/formats";
import { analyzeGlance, cappedRange, currentRegion } from "../src/glance";

function sheet(rows: (string | number | null)[][], at = { r: 0, c: 0 }) {
  const cells = new Map<string, CellView>();
  rows.forEach((row, i) =>
    row.forEach((v, j) => {
      if (v === null) return;
      const r = at.r + i;
      const c = at.c + j;
      cells.set(`${r},${c}`, {
        r,
        c,
        text: String(v),
        kind: typeof v === "number" ? "number" : "text",
        num: typeof v === "number" ? v : null,
        style: 0,
        color: null,
        flags: 0,
      });
    }),
  );
  return (r: number, c: number) => cells.get(`${r},${c}`);
}

const INVOICES = [
  ["Customer", "Qty", "Total", "Status"],
  ["A", 2, 10, "Paid"],
  ["B", 1, 5.5, "Due"],
  ["C", 3, 20, "Paid"],
  ["D", 1, 4.5, "Overdue"],
  ["E", 2, 10, "Paid"],
  ["F", 1, 1, "Due"],
];

describe("currentRegion", () => {
  it("grows to the connected block", () => {
    const get = sheet(INVOICES, { r: 2, c: 1 });
    expect(currentRegion(get, { r: 4, c: 2 }, { r0: 0, c0: 0, r1: 50, c1: 20 })).toEqual({ r0: 2, c0: 1, r1: 8, c1: 4 });
    expect(currentRegion(get, { r: 20, c: 10 }, { r0: 0, c0: 0, r1: 50, c1: 20 })).toBeNull();
  });
});

describe("analyzeGlance", () => {
  it("sums numeric columns with their format, counts the category, charts the Total", () => {
    const get = sheet(INVOICES);
    const g = analyzeGlance(get, { r0: 0, c0: 0, r1: 6, c1: 3 }, (c) => (c === 2 ? CURRENCY_FORMAT : "General"));
    expect(g.hasHeader).toBe(true);
    expect(g.rows).toBe(6);
    expect(g.sums.map((s) => [s.header, s.text])).toEqual([
      ["Qty", "10"],
      ["Total", "$51.00"],
    ]);
    expect(g.category?.header).toBe("Status");
    expect(g.category?.counts).toEqual([
      { value: "Paid", count: 3 },
      { value: "Due", count: 2 },
      { value: "Overdue", count: 1 },
    ]);
    expect(g.chart?.valueHeader).toBe("Total");
    expect(g.chart?.bars.map((b) => [b.label, b.text])).toEqual([
      ["Paid", "$40.00"],
      ["Due", "$6.50"],
      ["Overdue", "$4.50"],
    ]);
  });

  it("skips date-formatted columns and works without a header", () => {
    const get = sheet([
      [46000, 1],
      [46001, 2],
    ]);
    const g = analyzeGlance(get, { r0: 0, c0: 0, r1: 1, c1: 1 }, (c) => (c === 0 ? "MM/DD/YYYY" : "General"));
    expect(g.hasHeader).toBe(false);
    expect(g.sums.map((s) => [s.header, s.text])).toEqual([["Column B", "3"]]);
  });
});

describe("cappedRange", () => {
  it("caps the fetched rows", () => {
    expect(cappedRange({ r0: 0, c0: 0, r1: 9, c1: 3 }, 100)).toEqual({ range: { r0: 0, c0: 0, r1: 9, c1: 3 }, sampled: false });
    expect(cappedRange({ r0: 0, c0: 0, r1: 99, c1: 3 }, 100)).toEqual({ range: { r0: 0, c0: 0, r1: 24, c1: 3 }, sampled: true });
  });
});
