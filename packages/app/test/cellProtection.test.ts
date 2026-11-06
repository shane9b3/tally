/**
 * Format Cells ▸ Cell Protection (AOO `ScTabPageProtection`): the state
 * read from the selection's styles (indeterminate where they differ), the
 * enabling rules ("Hide all" disables "Protected" and "Hide formula"), the
 * per-box change, and the browser mock's "Hide all" / "Hide when printing"
 * model.
 */
import type { StyleDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { applyV3Op, emptyV3, isHiddenCell, isHidePrint, isLocked, protectionRefusal } from "../src/dev/mockV3";
import { protectionChange, protectionEnabled, protectionOf, selectionProtection, selectionStyles, stateFromStyle } from "../src/styles/formatCells";

const style = (id: number, protection: StyleDto["protection"]): StyleDto => ({ ...(stateFromStyle(undefined) as unknown as StyleDto), id, protection });
const DEF = { locked: true, hiddenFormula: false, hidden: false, hidePrint: false };

describe("Cell Protection tab", () => {
  it("reads the four attributes from the style (default: protected, nothing hidden)", () => {
    expect(protectionOf(undefined)).toEqual(DEF);
    expect(protectionOf(style(0, null))).toEqual(DEF);
    expect(protectionOf(style(3, { locked: false, hiddenFormula: true, hidden: true, hidePrint: true }))).toEqual({ locked: false, hiddenFormula: true, hidden: true, hidePrint: true });
  });

  it("a mixed selection gives indeterminate boxes for the flags that differ", () => {
    const a1 = style(1, { ...DEF, hidePrint: true });
    const a2 = style(0, null);
    expect(selectionProtection([a1, a2])).toEqual({ locked: true, hiddenFormula: false, hidden: false, hidePrint: null });
    expect(selectionProtection([a1, undefined, a1])).toEqual({ ...DEF, hidePrint: true });
    expect(selectionProtection([])).toEqual(DEF);
  });

  it("collects the styles of the loaded selected cells, plus the default for cells outside the used range", () => {
    const styles = new Map([[0, style(0, null)], [1, style(1, { ...DEF, hidePrint: true })]]);
    const cells = new Map([["0,0", 1]]);
    const source = { getCell: (r: number, c: number) => (cells.has(`${r},${c}`) ? { style: cells.get(`${r},${c}`)! } : undefined), getStyle: (id: number) => styles.get(id) };
    const used = { r0: 0, c0: 0, r1: 1, c1: 0 };
    // Active A2 (default), A1 has Hide when printing.
    expect(selectionProtection(selectionStyles(source, [{ r0: 0, c0: 0, r1: 1, c1: 0 }], { r: 1, c: 0 }, used)).hidePrint).toBeNull();
    expect(selectionProtection(selectionStyles(source, [{ r0: 0, c0: 0, r1: 0, c1: 0 }], { r: 0, c: 0 }, used)).hidePrint).toBe(true);
  });

  it("Hide all disables Protected and Hide formula, keeping their values; indeterminate leaves them enabled", () => {
    expect(protectionEnabled(DEF)).toEqual({ hidden: true, locked: true, hiddenFormula: true, hidePrint: true });
    expect(protectionEnabled({ ...DEF, hidden: true })).toEqual({ hidden: true, locked: false, hiddenFormula: false, hidePrint: true });
    expect(protectionEnabled({ ...DEF, hidden: null }).locked).toBe(true);
  });

  it("only the boxes the user changed get a value; the others leave each cell as it is", () => {
    const before = { locked: true, hiddenFormula: false, hidden: false, hidePrint: null };
    expect(protectionChange(before, { ...before })).toBeNull();
    // A1 has Hide when printing, A2 not: checking Hide formula keeps A1's flag.
    expect(protectionChange(before, { ...before, hiddenFormula: true })).toEqual({ locked: null, hiddenFormula: true, hidden: null, hidePrint: null });
    // Clicking the indeterminate box sets it everywhere.
    expect(protectionChange(before, { ...before, hidePrint: true })).toEqual({ locked: null, hiddenFormula: null, hidden: null, hidePrint: true });
    // Toggling a box back to where it was is no change.
    expect(protectionChange(DEF, { ...DEF })).toBeNull();
  });
});

describe("the mock's Hide all / Hide when printing", () => {
  it("hides only while the sheet is protected; hide-print regardless; hidden implies locked", () => {
    const x = emptyV3();
    const a1 = [{ r0: 0, c0: 0, r1: 0, c1: 0 }];
    applyV3Op(x, { type: "setCellProtection", sheet: 1, ranges: a1, locked: false, hiddenFormula: false, hidden: true, hidePrint: true });
    expect(isHiddenCell(x, 1, 0, 0)).toBe(false);
    expect(isHidePrint(x, 1, 0, 0)).toBe(true);
    expect(isLocked(x, 1, { r: 0, c: 0 })).toBe(true);
    applyV3Op(x, { type: "protectSheet", sheet: 1, password: null, options: null });
    expect(isHiddenCell(x, 1, 0, 0)).toBe(true);
    expect(isHiddenCell(x, 1, 0, 1)).toBe(false);
    expect(protectionRefusal(x, { type: "setCellInput", sheet: 1, r: 0, c: 0, input: "x" })).toMatch(/protected/);
    applyV3Op(x, { type: "unprotectSheet", sheet: 1, password: null });
    expect(isHiddenCell(x, 1, 0, 0)).toBe(false);
    // Absent / null fields leave the flags as they are.
    applyV3Op(x, { type: "setCellProtection", sheet: 1, ranges: a1, locked: null, hiddenFormula: true, hidden: null, hidePrint: null });
    expect(isHidePrint(x, 1, 0, 0)).toBe(true);
    applyV3Op(x, { type: "setCellProtection", sheet: 1, ranges: a1, locked: null, hiddenFormula: null, hidden: false, hidePrint: false });
    expect(isHidePrint(x, 1, 0, 0)).toBe(false);
    expect(isLocked(x, 1, { r: 0, c: 0 })).toBe(false);
  });
});
