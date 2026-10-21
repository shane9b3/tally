/**
 * W5-6D: Format Cells state ↔ StylePatch, the AutoFormat presets and the
 * app prefs parser.
 */
import type { StyleDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { AUTOFORMAT_PRESETS, DEFAULT_AUTOFORMAT_OPTIONS, previewCell, previewText } from "../src/styles/autoformat";
import { bordersPatchOf, formatCellsError, patchFromState, patchIsEmpty, presetEdges, stateFromStyle, toggleEdge } from "../src/styles/formatCells";
import { DEFAULT_PREFS, parsePrefs, serializePrefs } from "../src/options/prefs";

const base: StyleDto = { id: 7, bold: false, italic: false, underline: false, strike: false, fontFamily: null, fontSizePt: 11, color: null, fill: null, hAlign: "general", vAlign: "bottom", wrap: false, indent: 0, borders: { top: null, bottom: null, left: null, right: null }, numberFormat: "General", display: null };

describe("Format Cells", () => {
  it("writes only the changed fields, including rotation, indent, shrink and the underline style", () => {
    const st = stateFromStyle(base);
    expect(patchIsEmpty(patchFromState(st, base))).toBe(true);
    const next = { ...st, rotationDeg: 45, indent: 2, shrinkToFit: true, bold: true, underline: true, underlineStyle: "double" as const, numberFormat: "0.00" };
    const patch = patchFromState(next, base);
    expect(patch).toMatchObject({ bold: true, underline: true, underlineStyle: "double", rotationDeg: 45, indent: 2, shrinkToFit: true, numberFormat: "0.00", italic: null, fill: null });
    expect(patch.borders).toBeUndefined();
    expect(formatCellsError({ ...st, rotationDeg: 120 })).toMatch(/Rotation/);
    expect(formatCellsError({ ...st, fontSizePt: 0 })).toMatch(/Font size/);
    expect(formatCellsError(st)).toBeNull();
  });
  it("toggles border edges (set → removed → unchanged) and presets include the diagonals", () => {
    let st = stateFromStyle(base);
    st = toggleEdge(st, "diagonalDown");
    expect(st.borders.diagonalDown).toEqual({ style: "solid", widthPx: 1, color: st.borderColor });
    st = toggleEdge(st, "diagonalDown");
    expect(st.borders.diagonalDown).toBeNull();
    const patch = bordersPatchOf(st);
    expect(patch).toMatchObject({ diagonalDown: { line: null }, top: null, diagonalUp: null });
    const all = presetEdges(st, "all");
    expect(Object.keys(all.borders).sort()).toEqual(["bottom", "innerH", "innerV", "left", "right", "top"]);
    expect(presetEdges(st, "outer").borders.innerH).toBeNull();
    expect(Object.values(presetEdges(st, "none").borders).every((v) => v === null)).toBe(true);
    expect(patchFromState(all, base).borders?.innerV).toEqual({ line: { style: "solid", widthPx: 1, color: st.borderColor } });
  });
});

describe("AutoFormat", () => {
  it("has AOO's 17 presets in order and previews header/body/total bands", () => {
    expect(AUTOFORMAT_PRESETS.map((p) => p.name)).toEqual(["Default", "3D", "Black 1", "Black 2", "Blue", "Brown", "Currency", "Currency 3D", "Currency Gray", "Currency Lavender", "Currency Turquoise", "Gray", "Green", "Lavender", "Red", "Turquoise", "Yellow"]);
    const blue = AUTOFORMAT_PRESETS[4]!;
    expect(previewCell(blue, 0, 1, DEFAULT_AUTOFORMAT_OPTIONS)).toEqual({ bg: blue.header[0], fg: blue.header[1], bold: true });
    expect(previewCell(blue, 1, 1, DEFAULT_AUTOFORMAT_OPTIONS)).toEqual({ bg: blue.body[0], fg: blue.body[1], bold: false });
    expect(previewCell(blue, 4, 2, DEFAULT_AUTOFORMAT_OPTIONS).bg).toBe(blue.total[0]);
    expect(previewCell(blue, 1, 1, { ...DEFAULT_AUTOFORMAT_OPTIONS, pattern: false }).bg).toBe("transparent");
    const currency = AUTOFORMAT_PRESETS[6]!;
    expect(previewText(currency, 1, 1, DEFAULT_AUTOFORMAT_OPTIONS)).toBe("$6.00");
    expect(previewText(currency, 1, 1, { ...DEFAULT_AUTOFORMAT_OPTIONS, numberFormat: false })).toBe("6");
  });
});

describe("app prefs", () => {
  it("merges unknown JSON over the defaults and clamps values", () => {
    expect(parsePrefs(undefined)).toEqual(DEFAULT_PREFS);
    expect(parsePrefs({ uiLanguage: "de", autosaveMinutes: 999, enterMoves: "sideways", autoCorrect: { smartQuotes: true, bogus: 1 }, defaultLocale: "../x" })).toMatchObject({ uiLanguage: "de", autosaveMinutes: 120, enterMoves: "down", autoCorrect: { smartQuotes: true, twoInitialCapitals: true }, defaultLocale: "en-US" });
    expect(JSON.parse(serializePrefs({ ...DEFAULT_PREFS, argumentSeparator: "," }))).toMatchObject({ argumentSeparator: "," });
    expect(serializePrefs(DEFAULT_PREFS).length).toBeLessThan(2000);
  });
});
