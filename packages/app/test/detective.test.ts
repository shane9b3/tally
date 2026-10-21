/**
 * W5-5: detective arrows from `precedents`/`dependents` results and the
 * "Mark invalid data" check.
 */
import type { CellView, ValidationDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { cellValid, invalidCells, traceArrows } from "../src/detective/detective";

const cell = (r: number, c: number, text: string, kind: CellView["kind"] = "text", num: number | null = null): CellView => ({ r, c, text, kind, num, style: 0, color: null, flags: 0 });

describe("trace arrows", () => {
  it("precedents point at the formula cell; dependents away from it; other sheets are counted", () => {
    const { arrows, otherSheets } = traceArrows({ r: 2, c: 2 }, [{ sheet: 1, range: { r0: 0, c0: 0, r1: 1, c1: 0 }, error: false }, { sheet: 2, range: { r0: 0, c0: 0, r1: 0, c1: 0 }, error: true }], 1, "precedent");
    expect(arrows).toEqual([{ from: { r: 0, c: 0 }, to: { r: 2, c: 2 }, kind: "precedent" }]);
    expect(otherSheets).toBe(1);
    const dep = traceArrows({ r: 0, c: 0 }, [{ sheet: 1, range: { r0: 5, c0: 1, r1: 5, c1: 1 }, error: true }], 1, "dependent");
    expect(dep.arrows).toEqual([{ from: { r: 0, c: 0 }, to: { r: 5, c: 1 }, kind: "error" }]);
  });
});

describe("mark invalid data", () => {
  const list: ValidationDto = { id: 1, ranges: [{ r0: 1, c0: 0, r1: 3, c1: 0 }], kind: { type: "list", source: { type: "values", items: ["Paid", "Due"] } }, allowBlank: true, showDropdown: true, inputHelp: null, error: null };
  const whole: ValidationDto = { id: 2, ranges: [{ r0: 0, c0: 1, r1: 2, c1: 1 }], kind: { type: "whole", op: "between", formulas: ["1", "10"] }, allowBlank: false, showDropdown: false, inputHelp: null, error: null };
  it("checks lists, whole numbers with bounds and blanks", () => {
    expect(cellValid(list, cell(1, 0, "Paid"))).toBe(true);
    expect(cellValid(list, cell(1, 0, "Maybe"))).toBe(false);
    expect(cellValid(list, undefined)).toBe(true);
    expect(cellValid(whole, undefined)).toBe(false);
    expect(cellValid(whole, cell(0, 1, "5", "number", 5))).toBe(true);
    expect(cellValid(whole, cell(0, 1, "5.5", "number", 5.5))).toBe(false);
    expect(cellValid(whole, cell(0, 1, "11", "number", 11))).toBe(false);
    expect(cellValid(whole, cell(0, 1, "x"))).toBe(false);
    expect(cellValid({ ...whole, kind: { type: "textLength", op: "le", formulas: ["3"] } }, cell(0, 1, "abcd"))).toBe(false);
    expect(cellValid({ ...whole, kind: { type: "custom", formula: "=A1>0" } }, cell(0, 1, "x"))).toBe(true);
  });
  it("lists the breaking cells within the validation ranges", () => {
    const cells = [cell(1, 0, "Paid"), cell(2, 0, "Nope"), cell(3, 0, ""), cell(0, 1, "3", "number", 3), cell(1, 1, "", "empty"), cell(2, 1, "12", "number", 12), cell(9, 9, "Nope")];
    expect(invalidCells([list, whole], cells)).toEqual([
      { r: 2, c: 0 },
      { r: 1, c: 1 },
      { r: 2, c: 1 },
    ]);
  });
});
