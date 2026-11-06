/**
 * User AutoFormats: building one from a selection (AOO's 4 × 4 sample), the
 * name / rename / delete rules, applying with the formatting options, and the
 * prefs round trip with its validation (untrusted JSON) and cap.
 */
import type { StyleDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import type { AppController } from "../src/controller";
import { W56Features } from "../src/data/w56";
import { useW56Store } from "../src/data/store";
import { setTranslations } from "../src/i18n";
import { DEFAULT_AUTOFORMAT_OPTIONS } from "../src/styles/autoformat";
import { DEFAULT_PREFS, parsePrefs, serializePrefs } from "../src/options/prefs";
import {
  MAX_AUTOFORMAT_NAME,
  MAX_USER_AUTOFORMATS,
  addAutoFormat,
  autoFormatNameError,
  buildAutoFormat,
  deleteAutoFormat,
  emptyAutoFormatCell,
  parseUserAutoFormats,
  renameAutoFormat,
  roleOf,
  sampleOffsets,
  serializeUserAutoFormats,
  userAutoFormatOps,
  type UserAutoFormat,
} from "../src/styles/userAutoFormat";

function style(over: Partial<StyleDto> = {}): StyleDto {
  return { id: 0, bold: false, italic: false, underline: false, strike: false, fontFamily: null, fontSizePt: 10, color: null, fill: null, hAlign: "general", vAlign: "bottom", wrap: false, indent: 0, borders: { top: null, bottom: null, left: null, right: null }, numberFormat: "General", display: null, ...over };
}

/** A format whose slot `i` has fill `0x10 * (i + 1)` so every slot is recognisable. */
function marked(name: string): UserAutoFormat {
  return { name, cells: Array.from({ length: 16 }, (_, i) => ({ ...emptyAutoFormatCell(), fill: (i + 1) * 0x100 + 0xff, bold: i < 4 })) };
}

describe("building from a selection", () => {
  it("samples rows and columns 0, 1, 2 and last", () => {
    expect(sampleOffsets(3)).toEqual([0, 1, 1, 2]);
    expect(sampleOffsets(4)).toEqual([0, 1, 2, 3]);
    expect(sampleOffsets(10)).toEqual([0, 1, 2, 9]);
    const read: string[] = [];
    const f = buildAutoFormat("  Mine ", 10, 7, (dr, dc) => {
      read.push(`${dr},${dc}`);
      return style({ fill: dr * 100 + dc });
    });
    expect(f.name).toBe("Mine");
    expect(f.cells).toHaveLength(16);
    expect(read).toHaveLength(16);
    expect(f.cells.map((c) => c.fill)).toEqual([0, 1, 2, 6, 100, 101, 102, 106, 200, 201, 202, 206, 900, 901, 902, 906]);
  });

  it("carries font, colours, borders, alignment and number format", () => {
    const line = { style: "solid" as const, widthPx: 1, color: 0xff0000ff };
    const f = buildAutoFormat("X", 3, 3, () => style({ bold: true, italic: true, color: 0x112233ff, fontFamily: "Arial", hAlign: "center", numberFormat: "0.00", borders: { top: line, bottom: null, left: null, right: line } }));
    expect(f.cells[5]).toMatchObject({ bold: true, italic: true, color: 0x112233ff, fontFamily: "Arial", hAlign: "center", numberFormat: "0.00", borders: { top: line, right: line, bottom: null } });
  });

  it("needs at least 3 x 3 cells", () => {
    expect(() => buildAutoFormat("X", 2, 5, () => style())).toThrow();
    expect(() => buildAutoFormat("X", 5, 2, () => style())).toThrow();
  });

  it("maps a position to a role like AOO's GetIdx", () => {
    expect([0, 1, 2, 3, 4, 5].map((i) => roleOf(i, 6))).toEqual([0, 1, 2, 1, 2, 3]);
  });
});

describe("names, rename and delete", () => {
  it("validates names", () => {
    const list = [marked("Mine")];
    expect(autoFormatNameError("", list)).not.toBeNull();
    expect(autoFormatNameError("   ", list)).not.toBeNull();
    expect(autoFormatNameError("mine", list)).not.toBeNull();
    expect(autoFormatNameError("Default", list)).not.toBeNull();
    expect(autoFormatNameError("blue", list)).not.toBeNull();
    expect(autoFormatNameError("a\nb", list)).not.toBeNull();
    expect(autoFormatNameError("x".repeat(MAX_AUTOFORMAT_NAME + 1), list)).not.toBeNull();
    expect(autoFormatNameError("x".repeat(MAX_AUTOFORMAT_NAME), list)).toBeNull();
    expect(autoFormatNameError("Mine", list, "Mine")).toBeNull();
    expect(autoFormatNameError("Other", list)).toBeNull();
  });

  it("adds, renames and deletes user formats only", () => {
    let list = addAutoFormat([], marked(" Mine "));
    expect(list.map((f) => f.name)).toEqual(["Mine"]);
    expect(() => addAutoFormat(list, marked("MINE"))).toThrow();
    list = addAutoFormat(list, marked("Two"));
    expect(renameAutoFormat(list, "Mine", "Renamed").map((f) => f.name)).toEqual(["Renamed", "Two"]);
    expect(() => renameAutoFormat(list, "Mine", "Two")).toThrow();
    expect(() => renameAutoFormat(list, "Mine", "Gray")).toThrow();
    expect(() => renameAutoFormat(list, "Blue", "Mine2")).toThrow();
    expect(deleteAutoFormat(list, "Mine").map((f) => f.name)).toEqual(["Two"]);
    expect(() => deleteAutoFormat(list, "Default")).toThrow();
    expect(() => deleteAutoFormat(list, "Nope")).toThrow();
  });

  it("caps the list", () => {
    let list: UserAutoFormat[] = [];
    for (let i = 0; i < MAX_USER_AUTOFORMATS; i++) list = addAutoFormat(list, marked(`F${i}`));
    expect(() => addAutoFormat(list, marked("One more"))).toThrow();
  });
});

describe("applying", () => {
  const f = marked("Mine");

  it("builds one op with the 16 slot patches, whatever the range size", () => {
    for (const [r1, c1] of [[5, 4], [999, 20], [99_999, 9]] as const) {
      const ops = userAutoFormatOps(1, { r0: 0, c0: 0, r1, c1 }, f, DEFAULT_AUTOFORMAT_OPTIONS);
      expect(ops).toHaveLength(1);
      const op = ops[0]!;
      expect(op.type).toBe("applyAutoFormatCustom");
      if (op.type !== "applyAutoFormatCustom") throw new Error("type");
      expect(op.cells).toHaveLength(16);
      expect(op.range).toEqual({ r0: 0, c0: 0, r1, c1 });
      expect(JSON.stringify(op).length).toBeLessThan(8000);
    }
    const op = userAutoFormatOps(1, { r0: 0, c0: 0, r1: 4, c1: 4 }, f, DEFAULT_AUTOFORMAT_OPTIONS)[0]!;
    if (op.type !== "applyAutoFormatCustom") throw new Error("type");
    expect(op.cells.map((c) => c.fill)).toEqual(f.cells.map((c) => c.fill));
    expect(op.autofit).toBe(true);
  });

  it("honours the options", () => {
    const base = { numberFormat: false, font: false, alignment: false, borders: false, pattern: false, autofit: false };
    const one = (o: Partial<typeof base>) => {
      const op = userAutoFormatOps(1, { r0: 0, c0: 0, r1: 2, c1: 2 }, f, { ...base, ...o })[0]!;
      if (op.type !== "applyAutoFormatCustom") throw new Error("type");
      return op;
    };
    const none = one({});
    expect(none.cells[0]).toMatchObject({ bold: null, fill: null, hAlign: null, numberFormat: null });
    expect(none.cells[0]?.borders).toBeUndefined();
    expect(none.autofit).toBe(false);
    expect(one({ pattern: true }).cells[0]?.fill).toBe(0x1ff);
    expect(one({ font: true }).cells[0]?.bold).toBe(true);
    expect(one({ alignment: true }).cells[0]?.hAlign).toBe("general");
    expect(one({ numberFormat: true }).cells[0]?.numberFormat).toBe("General");
    expect(one({ borders: true }).cells[0]?.borders?.top).toEqual({ line: null });
    expect(one({ autofit: true }).autofit).toBe(true);
  });

  it("clears the fill and colour of a slot that has none", () => {
    const plain: UserAutoFormat = { name: "P", cells: Array.from({ length: 16 }, () => emptyAutoFormatCell()) };
    const op = userAutoFormatOps(1, { r0: 0, c0: 0, r1: 2, c1: 2 }, plain, DEFAULT_AUTOFORMAT_OPTIONS)[0]!;
    expect(op.type === "applyAutoFormatCustom" && op.cells[0]).toMatchObject({ clearFill: true, clearColor: true });
  });

  it("refuses ranges below 3 x 3 and damaged formats", () => {
    expect(() => userAutoFormatOps(1, { r0: 0, c0: 0, r1: 1, c1: 5 }, f, DEFAULT_AUTOFORMAT_OPTIONS)).toThrow();
    expect(() => userAutoFormatOps(1, { r0: 0, c0: 0, r1: 4, c1: 4 }, { name: "x", cells: [] }, DEFAULT_AUTOFORMAT_OPTIONS)).toThrow();
  });
});

describe("localised built-in names", () => {
  it("refuses the translated name of a built-in too", () => {
    setTranslations({ "stylesUi.preset.default": "Standard", "stylesUi.preset.gray": "Grau" });
    try {
      expect(autoFormatNameError("Standard", [])).not.toBeNull();
      expect(autoFormatNameError("grau", [])).not.toBeNull();
      expect(autoFormatNameError("Default", [])).not.toBeNull();
    } finally {
      setTranslations({});
    }
    expect(autoFormatNameError("Standard", [])).toBeNull();
  });
});

describe("saving the prefs", () => {
  const big = (n: number): UserAutoFormat[] =>
    Array.from({ length: n }, (_, i) => ({ name: `Format number ${i}`, cells: Array.from({ length: 16 }, () => ({ bold: true, italic: true, underline: true, strike: true, fontFamily: "F".repeat(100), fontSizePt: 11.5, color: 0xff00ffff, fill: 0xeeeeeeff, hAlign: "center" as const, vAlign: "middle" as const, numberFormat: "0".repeat(255), borders: { top: { style: "double" as const, widthPx: 2, color: 1 }, bottom: { style: "double" as const, widthPx: 2, color: 1 }, left: { style: "double" as const, widthPx: 2, color: 1 }, right: { style: "double" as const, widthPx: 2, color: 1 } } })) }));

  function features(setPrefs: (json: string) => Promise<void>) {
    const errors: unknown[] = [];
    const ctl = { shell: { setPrefs }, notifyError: (e: unknown) => errors.push(e), broadcastPrefs: () => Promise.resolve() };
    return { w56: new W56Features(ctl as unknown as AppController), errors };
  }

  it("refuses an oversized document without touching the store or the shell", async () => {
    const sent: string[] = [];
    const { w56 } = features((j) => (sent.push(j), Promise.resolve()));
    useW56Store.getState().setPrefs(DEFAULT_PREFS);
    expect(await w56.savePrefs({ ...DEFAULT_PREFS, autoFormats: big(100) })).toBe(false);
    expect(sent).toHaveLength(0);
    expect(useW56Store.getState().prefs.autoFormats).toEqual([]);
    expect(await w56.savePrefs({ ...DEFAULT_PREFS, autoFormats: [marked("Ok")] })).toBe(true);
    expect(sent).toHaveLength(1);
    expect(useW56Store.getState().prefs.autoFormats.map((f) => f.name)).toEqual(["Ok"]);
  });

  it("rolls the store back when the shell refuses", async () => {
    const { w56, errors } = features(() => Promise.reject(new Error("too large")));
    useW56Store.getState().setPrefs({ ...DEFAULT_PREFS, autoFormats: [marked("Old")] });
    expect(await w56.savePrefs({ ...DEFAULT_PREFS, autoFormats: [marked("Old"), marked("New")] })).toBe(false);
    expect(useW56Store.getState().prefs.autoFormats.map((f) => f.name)).toEqual(["Old"]);
    expect(errors).toHaveLength(1);
    expect(await w56.addAutoFormat({ r0: 0, c0: 0, r1: 2, c1: 2 }, "X")).toBe(false);
  });
});

describe("prefs", () => {
  it("round-trips compactly", () => {
    const line = { style: "dashed" as const, widthPx: 2, color: 0x336699ff };
    const f = marked("Mine");
    f.cells[3] = { ...f.cells[3]!, borders: { top: line, bottom: null, left: null, right: null }, hAlign: "right", fontFamily: "Arial", fontSizePt: 12, numberFormat: "0.00" };
    const text = serializePrefs({ ...DEFAULT_PREFS, autoFormats: [f] });
    expect(parsePrefs(JSON.parse(text)).autoFormats).toEqual([f]);
    expect(JSON.stringify(serializeUserAutoFormats([{ name: "E", cells: Array.from({ length: 16 }, () => emptyAutoFormatCell()) }]))).toBe(`[{"name":"E","cells":[${Array(16).fill("{}").join(",")}]}]`);
  });

  it("drops malformed entries and fixes bad fields", () => {
    const good = JSON.parse(JSON.stringify(serializeUserAutoFormats([marked("Good")])));
    const parsed = parseUserAutoFormats([
      null,
      7,
      "x",
      { name: 5, cells: good[0].cells },
      { name: "Short", cells: [] },
      { name: "Default", cells: good[0].cells },
      { name: "A\u0000B", cells: good[0].cells },
      good[0],
      { ...good[0], name: "good" },
      { name: "Wild", cells: Array.from({ length: 16 }, () => ({ fill: "red", color: -1, bold: "yes", hAlign: "evil", fontSizePt: 1e9, numberFormat: 3, borders: { top: { style: "wavy", widthPx: 1, color: 1 }, left: { style: "solid", widthPx: 99, color: 1.5 } } })) },
    ]);
    expect(parsed.map((f) => f.name)).toEqual(["Good", "Wild"]);
    expect(parsed[1]!.cells[0]).toEqual({ ...emptyAutoFormatCell(), borders: { top: null, bottom: null, left: { style: "solid", widthPx: 8, color: 0x000000ff }, right: null } });
    expect(parseUserAutoFormats("nope")).toEqual([]);
    expect(parseUserAutoFormats({})).toEqual([]);
  });

  it("caps the stored list at 100 and the name length", () => {
    const cells = serializeUserAutoFormats([marked("x")])[0] as { cells: unknown[] };
    const many = Array.from({ length: 150 }, (_, i) => ({ name: `F${i}`, cells: cells.cells }));
    expect(parseUserAutoFormats(many)).toHaveLength(MAX_USER_AUTOFORMATS);
    expect(parseUserAutoFormats([{ name: "n".repeat(MAX_AUTOFORMAT_NAME + 1), cells: cells.cells }])).toEqual([]);
    expect(parsePrefs({ autoFormats: many }).autoFormats).toHaveLength(MAX_USER_AUTOFORMATS);
    expect(parsePrefs({}).autoFormats).toEqual([]);
  });

  it("100 fully formatted formats fit the prefs size limit", () => {
    const full = (i: number): UserAutoFormat => ({ name: `Format number ${i}`, cells: Array.from({ length: 16 }, () => ({ bold: true, italic: true, underline: true, strike: true, fontFamily: "Helvetica Neue", fontSizePt: 11.5, color: 0xff00ffff, fill: 0xeeeeeeff, hAlign: "center" as const, vAlign: "middle" as const, numberFormat: "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00", borders: { top: { style: "double" as const, widthPx: 2, color: 1 }, bottom: { style: "double" as const, widthPx: 2, color: 1 }, left: { style: "double" as const, widthPx: 2, color: 1 }, right: { style: "double" as const, widthPx: 2, color: 1 } } })) });
    const text = serializePrefs({ ...DEFAULT_PREFS, autoFormats: Array.from({ length: 100 }, (_, i) => full(i)) });
    expect(text.length).toBeLessThan(1024 * 1024);
  });
});
