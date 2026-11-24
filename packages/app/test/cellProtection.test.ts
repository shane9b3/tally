/**
 * Format Cells ▸ Cell Protection (AOO `ScTabPageProtection`): the state
 * read from the selection's styles (indeterminate where they differ), the
 * enabling rules ("Hide all" disables "Protected" and "Hide formula"), the
 * per-box change, and the browser mock's "Hide all" / "Hide when printing"
 * model.
 */
import { PROTECTED_MESSAGE, type Op, type StyleDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { WebMockClient } from "../src/dev/webMock";
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

describe("the browser mock's Clear formats / Clear all (the engine's Op::Clear)", () => {
  const a1a4 = { r0: 0, c0: 0, r1: 3, c1: 0 };
  async function book() {
    const c = new WebMockClient();
    const info = await c.newWorkbook();
    const doc = info.doc;
    const sheet = info.activeSheet;
    const tx = (ops: Op[]) => c.apply(doc, { ops, label: null, selectionBefore: null, selectionAfter: null });
    const res = await tx([
      { type: "setCellInput", sheet, r: 0, c: 0, input: "7" },
      { type: "addConditionalFormat", sheet, ranges: [a1a4], rules: [{ kind: { type: "cellValue", op: "gt", formulas: ["5"] }, style: { bold: true, italic: null, underline: null, strike: null, color: null, fill: null, borders: null, numberFormat: null, display: null }, stopIfTrue: false }] },
      { type: "addValidation", sheet, validation: { id: 0, ranges: [{ r0: 0, c0: 1, r1: 3, c1: 1 }], kind: { type: "list", source: { type: "values", items: ["a", "b"] } }, allowBlank: true, showDropdown: true, sortAscending: false, inputHelp: null, error: null } },
      { type: "mergeCells", sheet, range: { r0: 2, c0: 0, r1: 3, c1: 0 } },
    ]);
    expect(res.errors).toEqual([]);
    return { c, doc, sheet, tx };
  }

  it("Clear formats removes conditional formats and validations of the cells and keeps merges; undo brings them back", async () => {
    const { c, doc, sheet, tx } = await book();
    const res = await tx([{ type: "clear", sheet, ranges: [{ r0: 1, c0: 0, r1: 3, c1: 1 }], what: "formats" }]);
    expect(res.errors).toEqual([]);
    expect((await c.conditionalFormats(doc, sheet)).map((cf) => cf.ranges)).toEqual([[{ r0: 0, c0: 0, r1: 0, c1: 0 }]]);
    expect((await c.validations(doc, sheet)).map((v) => v.ranges)).toEqual([[{ r0: 0, c0: 1, r1: 0, c1: 1 }]]);
    expect((await c.sheetLayout(doc, sheet)).merges).toEqual([{ r0: 2, c0: 0, r1: 3, c1: 0 }]);
    await c.undo(doc);
    expect((await c.conditionalFormats(doc, sheet)).map((cf) => cf.ranges)).toEqual([[a1a4]]);
    expect(await c.validations(doc, sheet)).toHaveLength(1);
    // Clear all does the same; Clear contents takes neither.
    await tx([{ type: "clear", sheet, ranges: [{ r0: 0, c0: 0, r1: 3, c1: 1 }], what: "contents" }]);
    expect(await c.conditionalFormats(doc, sheet)).toHaveLength(1);
    await tx([{ type: "clear", sheet, ranges: [{ r0: 0, c0: 0, r1: 3, c1: 1 }], what: "all" }]);
    expect(await c.conditionalFormats(doc, sheet)).toEqual([]);
    expect(await c.validations(doc, sheet)).toEqual([]);
    expect((await c.sheetLayout(doc, sheet)).merges).toHaveLength(1);
  });

  it("a protected sheet refuses both where a conditional format or validation is in the range, even on unlocked cells", async () => {
    const { c, doc, sheet, tx } = await book();
    const all = { r0: 0, c0: 0, r1: 9, c1: 4 };
    await tx([{ type: "setCellProtection", sheet, ranges: [all], locked: false, hiddenFormula: null, hidden: null, hidePrint: null }]);
    await tx([{ type: "protectSheet", sheet, password: null, options: null }]);
    for (const what of ["formats", "all"] as const) {
      for (const col of [0, 1]) {
        const res = await tx([{ type: "clear", sheet, ranges: [{ r0: 0, c0: col, r1: 0, c1: col }], what }]);
        expect(res.errors).toMatchObject([{ opIndex: 0, code: "protected", message: PROTECTED_MESSAGE }]);
      }
    }
    expect(await c.conditionalFormats(doc, sheet)).toHaveLength(1);
    expect(await c.validations(doc, sheet)).toHaveLength(1);
    // Unlocked cells with neither, and Clear contents, still work.
    expect((await tx([{ type: "clear", sheet, ranges: [{ r0: 0, c0: 3, r1: 2, c1: 4 }], what: "all" }])).errors).toEqual([]);
    expect((await tx([{ type: "clear", sheet, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], what: "contents" }])).errors).toEqual([]);
    const [b] = await c.cells(doc, sheet, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]);
    expect(b?.cells.find((v) => v.text !== "")).toBeUndefined();
  });

  it("a Clear with a range outside the grid is refused and takes nothing; one that takes a format reports the layout", async () => {
    const { c, doc, sheet, tx } = await book();
    for (const bad of [{ r0: 0, c0: 0, r1: 2_000_000, c1: 1 }, { r0: 3, c0: 0, r1: 0, c1: 1 }, { r0: -1, c0: 0, r1: 3, c1: 1 }]) {
      // A good range first: nothing of the op may be applied.
      const res = await tx([{ type: "clear", sheet, ranges: [a1a4, bad], what: "formats" }]);
      expect(res.errors).toMatchObject([{ opIndex: 0, code: "invalid_argument" }]);
      expect((await c.conditionalFormats(doc, sheet)).map((cf) => cf.ranges)).toEqual([[a1a4]]);
      expect(await c.validations(doc, sheet)).toHaveLength(1);
    }
    // No format in the range: cells only. With one: the layout changed too (as the engine reports).
    expect((await tx([{ type: "clear", sheet, ranges: [{ r0: 0, c0: 5, r1: 0, c1: 5 }], what: "formats" }])).layoutChanged).not.toContain(sheet);
    expect((await tx([{ type: "clear", sheet, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], what: "formats" }])).layoutChanged).toContain(sheet);
  });

  // The engine's `ops::check_protection` for `Op::Clear`: `check_edit` for
  // Contents / All, `check_format` (`format_cells || all_unlocked`) for Formats / All.
  it("with locked cells, Clear formats needs the sheet's Format cells permission; Clear contents and Clear all need unlocked cells", async () => {
    const options = { selectLocked: true, selectUnlocked: true, insertRows: false, insertCols: false, deleteRows: false, deleteCols: false, formatCells: true, sort: false, autoFilter: false };
    const plain = { r0: 0, c0: 3, r1: 0, c1: 3 };
    const { c, doc, sheet, tx } = await book();
    await tx([{ type: "setCellInput", sheet, r: 0, c: 3, input: "x" }]);
    await tx([{ type: "protectSheet", sheet, password: null, options }]);
    expect((await tx([{ type: "clear", sheet, ranges: [plain], what: "formats" }])).errors).toEqual([]);
    expect((await tx([{ type: "clear", sheet, ranges: [plain], what: "contents" }])).errors).toMatchObject([{ code: "protected" }]);
    expect((await tx([{ type: "clear", sheet, ranges: [plain], what: "all" }])).errors).toMatchObject([{ code: "protected" }]);
    // The permission does not reach conditional formats and validations.
    expect((await tx([{ type: "clear", sheet, ranges: [a1a4], what: "formats" }])).errors).toMatchObject([{ code: "protected", message: PROTECTED_MESSAGE }]);
    expect(await c.conditionalFormats(doc, sheet)).toHaveLength(1);
    // Without the permission, locked cells refuse Clear formats too.
    await tx([{ type: "unprotectSheet", sheet, password: null }]);
    await tx([{ type: "protectSheet", sheet, password: null, options: { ...options, formatCells: false } }]);
    expect((await tx([{ type: "clear", sheet, ranges: [plain], what: "formats" }])).errors).toMatchObject([{ code: "protected" }]);
    const [b] = await c.cells(doc, sheet, [plain]);
    expect(b?.cells[0]?.text).toBe("x");
  });
});
