import { describe, expect, it } from "vitest";
import { ALL_KINDS, DEFAULT_PASTE_SPECIAL, pasteAsksOverwrite, pasteShiftAllowed, pasteChoiceValid, pasteKinds, pasteOptions, pastesCells, pasteWhat } from "../src/dataOps";
import { mixInputs } from "@tally/bridge";
import { WebMockClient } from "../src/dev/webMock";

const none = { text: false, numbers: false, dateTime: false, formulas: false, notes: false, formats: false };

describe("Paste Special dialog mapping", () => {
  it("Paste all and the presets keep the older option fields", () => {
    expect(pasteOptions(DEFAULT_PASTE_SPECIAL)).toEqual({ valuesOnly: false, formatsOnly: false, transpose: false, skipEmpty: false, operation: "none" });
    expect(pasteWhat(true, none)).toEqual({ what: "all" });
    expect(pasteWhat(false, { ...none, text: true, numbers: true, dateTime: true })).toEqual({ what: "values" });
    expect(pasteWhat(false, { ...none, formats: true })).toEqual({ what: "formats" });
    expect(pasteKinds({ ...DEFAULT_PASTE_SPECIAL, what: "values" })).toEqual({ ...none, text: true, numbers: true, dateTime: true });
  });

  it("other selections, Link and Shift cells map to the new fields", () => {
    const what = pasteWhat(false, { ...none, numbers: true, notes: true });
    expect(what).toEqual({ what: "custom", kinds: { ...none, numbers: true, notes: true } });
    const choice = { ...DEFAULT_PASTE_SPECIAL, ...what, link: true, shift: "down" as const, operation: "add" as const };
    expect(pasteOptions(choice)).toEqual({
      valuesOnly: false,
      formatsOnly: false,
      transpose: false,
      skipEmpty: false,
      operation: "add",
      selection: { ...none, numbers: true, notes: true },
      link: true,
      shift: "down",
    });
    expect(pasteAsksOverwrite(choice)).toBe(false);
  });

  it("operations need a cell kind; OK needs some kind", () => {
    const notes = { ...DEFAULT_PASTE_SPECIAL, ...pasteWhat(false, { ...none, notes: true }), operation: "multiply" as const };
    expect(pastesCells(notes)).toBe(false);
    expect(pasteOptions(notes).operation).toBe("none");
    expect(pasteAsksOverwrite(notes)).toBe(false);
    expect(pasteChoiceValid({ ...DEFAULT_PASTE_SPECIAL, ...pasteWhat(false, none) })).toBe(false);
    expect(pasteChoiceValid(DEFAULT_PASTE_SPECIAL)).toBe(true);
    expect(pasteAsksOverwrite(DEFAULT_PASTE_SPECIAL)).toBe(true);
    expect(pasteAsksOverwrite({ ...DEFAULT_PASTE_SPECIAL, shift: "right" })).toBe(false);
    expect(pasteKinds({ ...DEFAULT_PASTE_SPECIAL, what: "custom" })).toEqual(ALL_KINDS);
  });
});

describe("Shift cells availability (AOO cellsh1.cxx)", () => {
  it("greys out a direction that would push the block off the sheet", () => {
    expect(pasteShiftAllowed({ r: 1_048_572, c: 0 }, { rows: 3, cols: 1 })).toEqual({ down: true, right: true });
    expect(pasteShiftAllowed({ r: 1_048_573, c: 0 }, { rows: 3, cols: 1 })).toEqual({ down: false, right: true });
    expect(pasteShiftAllowed({ r: 0, c: 16_383 }, { rows: 1, cols: 1 })).toEqual({ down: true, right: false });
    expect(pasteShiftAllowed({ r: 0, c: 0 }, null)).toEqual({ down: true, right: true });
  });
});

describe("web mock Paste Special", () => {
  it("mixes like AOO MixData", () => {
    expect(mixInputs("10", "2", "multiply")).toBe("20");
    expect(mixInputs("7", "", "multiply")).toBe("0");
    expect(mixInputs("4", "t", "add")).toBe("4");
    expect(mixInputs("", "t", "add")).toBe("");
    expect(mixInputs("1", "=A1*5", "multiply")).toBe("=1*(A1*5)");
    expect(mixInputs("5", "0", "divide")).toBe("#VALUE!");
  });

  it("selection kinds, link and shift", async () => {
    const c = new WebMockClient();
    const { doc } = await c.newWorkbook(null);
    const tx = (ops: Parameters<WebMockClient["apply"]>[1]["ops"]) => c.apply(doc, { ops, label: null, selectionBefore: null, selectionAfter: null });
    await tx([{ type: "setRangeInputs", sheet: 1, r: 0, c: 0, rows: [["abc"], ["5"], ["=A2*2"]] }]);
    const { token } = await c.copy(doc, { sheet: 1, ranges: [{ r0: 0, c0: 0, r1: 2, c1: 0 }], active: { r: 0, c: 0 } });
    const input = async (r: number, col: number) => (await c.cellEditInfo(doc, 1, r, col)).input;
    const opts = { valuesOnly: false, formatsOnly: false, transpose: false, skipEmpty: false, operation: "none" as const };
    // Numbers only: the formula's value, no text.
    let res = await tx([{ type: "pasteSpecial", sheet: 1, target: { r0: 0, c0: 2, r1: 0, c1: 2 }, payload: { type: "internal", token }, options: { ...opts, selection: { ...none, numbers: true } } }]);
    expect(res.errors).toEqual([]);
    expect([await input(0, 2), await input(1, 2), await input(2, 2)]).toEqual(["", "5", "10"]);
    // Link.
    res = await tx([{ type: "pasteSpecial", sheet: 1, target: { r0: 0, c0: 4, r1: 0, c1: 4 }, payload: { type: "internal", token }, options: { ...opts, link: true } }]);
    expect(res.errors).toEqual([]);
    expect(await input(1, 4)).toBe("=$Sheet1.$A$2");
    // Shift down: E1:E3 move to E4:E6.
    res = await tx([{ type: "pasteSpecial", sheet: 1, target: { r0: 0, c0: 4, r1: 0, c1: 4 }, payload: { type: "internal", token }, options: { ...opts, shift: "down" } }]);
    expect(res.errors).toEqual([]);
    expect(await input(0, 4)).toBe("abc");
    expect(await input(4, 4)).toBe("=$Sheet1.$A$2");
  });
});
