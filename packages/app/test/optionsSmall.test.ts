/**
 * Small AOO 4.1 options: the two-digit year window pref (P17), the mock's
 * year and General-decimals emulation, the 20–400 % zoom range (U18) with
 * its steps, pinch / ⌃-wheel zoom, the Zoom dialog's Variable field and
 * the mock's `setViewZoom`.
 */
import { MockClient } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { clampZoom, MAX_ZOOM, MIN_ZOOM, nextZoom } from "../src/formats";
import { DEFAULT_PREFS, isTwoDigitYearStart, parsePrefs, serializePrefs } from "../src/options/prefs";
import { dateTexts, expandTwoDigitYear, limitGeneral, recognizeTwoDigitDate, serialOf } from "../src/dev/mockSmallOptions";
import { wheelZoom } from "../src/objects/pinchZoom";
import { variableZoom } from "../src/objects/ZoomDialog";

describe("two-digit year window pref", () => {
  it("defaults to 1930 and keeps only AOO's range 1583–9857", () => {
    expect(DEFAULT_PREFS.twoDigitYearStart).toBe(1930);
    expect(parsePrefs({}).twoDigitYearStart).toBe(1930);
    expect(parsePrefs({ twoDigitYearStart: 1950 }).twoDigitYearStart).toBe(1950);
    expect(parsePrefs({ twoDigitYearStart: 1582 }).twoDigitYearStart).toBe(1930);
    expect(parsePrefs({ twoDigitYearStart: 9858 }).twoDigitYearStart).toBe(1930);
    expect(parsePrefs({ twoDigitYearStart: 1950.5 }).twoDigitYearStart).toBe(1930);
    expect(parsePrefs({ twoDigitYearStart: "1950" }).twoDigitYearStart).toBe(1930);
    expect(isTwoDigitYearStart(1583) && isTwoDigitYearStart(9857)).toBe(true);
    expect(JSON.parse(serializePrefs({ ...DEFAULT_PREFS, twoDigitYearStart: 2000 }))).toMatchObject({ twoDigitYearStart: 2000 });
  });

  it("the mock expands years like AOO's ExpandTwoDigitYear", () => {
    expect(expandTwoDigitYear(45, 1930)).toBe(1945);
    expect(expandTwoDigitYear(29, 1930)).toBe(2029);
    expect(expandTwoDigitYear(45, 1950)).toBe(2045);
    expect(expandTwoDigitYear(50, 1950)).toBe(1950);
    expect(expandTwoDigitYear(99, 2000)).toBe(2099);
    const d = recognizeTwoDigitDate("1/1/45", 1950);
    expect(d).toEqual({ serial: serialOf(2045, 1, 1), y: 2045, m: 1, d: 1 });
    expect(dateTexts(d!)).toEqual({ cell: "01/01/45", edit: "01/01/2045" });
    expect(recognizeTwoDigitDate("1/1/45", 1930)?.y).toBe(1945);
    expect(recognizeTwoDigitDate("2/30/45", 1930)).toBeNull();
    expect(recognizeTwoDigitDate("1/1/1945", 1930)).toBeNull();
    expect(serialOf(1900, 1, 1)).toBe(2);
  });

  it("the mock limits General decimals like ImpGetOutputStandard", () => {
    expect(limitGeneral(Math.PI, 2)).toBe("3.14");
    expect(limitGeneral(1.5, 2)).toBe("1.5");
    expect(limitGeneral(2, 2)).toBe("2");
    expect(limitGeneral(-0.001, 2)).toBe("0");
    expect(limitGeneral(1e16, 2)).toBe("1.00E+16");
  });
});

describe("zoom 20–400 %", () => {
  it("clamps to AOO's MINZOOM / MAXZOOM", () => {
    expect([MIN_ZOOM, MAX_ZOOM]).toEqual([0.2, 4]);
    expect(clampZoom(0.1)).toBe(0.2);
    expect(clampZoom(5)).toBe(4);
    expect(clampZoom(0.2)).toBe(0.2);
    expect(clampZoom(4)).toBe(4);
  });

  it("steps reach both ends", () => {
    let z = 1;
    for (let i = 0; i < 20; i++) z = nextZoom(z, -1);
    expect(z).toBe(0.2);
    for (let i = 0; i < 30; i++) z = nextZoom(z, 1);
    expect(z).toBe(4);
    expect(nextZoom(0.5, -1)).toBe(0.33);
    expect(nextZoom(2, 1)).toBe(2.5);
  });

  it("pinch follows the fingers and a ⌃-wheel notch steps 10 points", () => {
    expect(wheelZoom(1, 0, 0)).toBe(1);
    expect(wheelZoom(1, -10, 0)).toBeGreaterThan(1);
    expect(wheelZoom(1, 10, 0)).toBeLessThan(1);
    expect(wheelZoom(1, -100, 0)).toBe(1.1);
    expect(wheelZoom(1, 100, 0)).toBe(0.9);
    expect(wheelZoom(1, 3, 1)).toBe(0.9);
    expect(wheelZoom(0.2, 100, 0)).toBe(0.2);
    expect(wheelZoom(4, -100, 0)).toBe(4);
    expect(wheelZoom(3.99, -40, 0)).toBe(4);
  });

  it("the Zoom dialog's Variable field clamps to 20–400", () => {
    expect(variableZoom("150")).toBe(150);
    expect(variableZoom("150 %")).toBe(150);
    expect(variableZoom("5")).toBe(20);
    expect(variableZoom("1000")).toBe(400);
    expect(variableZoom("")).toBeNull();
    expect(variableZoom("abc")).toBeNull();
  });

  it("the mock keeps the view zoom per document (view state, not undoable)", async () => {
    const client = new MockClient();
    const info = await client.newWorkbook(null);
    const sheet = info.sheets[0]!.id;
    expect((await client.viewOptions(info.doc, sheet)).zoom).toBeNull();
    await client.setViewZoom(info.doc, 20);
    expect((await client.viewOptions(info.doc, sheet)).zoom).toBe(20);
    await expect(client.setViewZoom(info.doc, 19)).rejects.toMatchObject({ code: "invalid_argument" });
    await expect(client.setViewZoom(info.doc, 401)).rejects.toMatchObject({ code: "invalid_argument" });
    await client.setViewZoom(info.doc, 100);
    expect((await client.viewOptions(info.doc, sheet)).zoom).toBeNull();
  });
});
