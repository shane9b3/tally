import { describe, expect, it } from "vitest";
import { pasteArea } from "@tally/bridge";
import { nextUnlocked, type UnlockedSheet } from "../src/dev/nextUnlocked";
import { DEFAULT_PREFS, parsePrefs } from "../src/options/prefs";

const sheet = (over: Partial<UnlockedSheet> = {}): UnlockedSheet => ({
  unlocked: [],
  rowHidden: () => false,
  colHidden: () => false,
  covered: () => false,
  rows: 100,
  cols: 20,
  ...over,
});
const cell = (r: number, c: number) => ({ r, c });

describe("nextUnlocked (the mock's walk, same as the engine's)", () => {
  // B3:B4, D3
  const unlocked = [
    { r0: 2, c0: 1, r1: 3, c1: 1 },
    { r0: 2, c0: 3, r1: 2, c1: 3 },
  ];
  it("walks reading order and wraps, both ways", () => {
    const s = sheet({ unlocked });
    expect(nextUnlocked(s, cell(0, 0), "right", null)).toEqual(cell(2, 1));
    expect(nextUnlocked(s, cell(2, 1), "right", null)).toEqual(cell(2, 3));
    expect(nextUnlocked(s, cell(2, 3), "right", null)).toEqual(cell(3, 1));
    expect(nextUnlocked(s, cell(3, 1), "right", null)).toEqual(cell(2, 1));
    expect(nextUnlocked(s, cell(3, 1), "left", null)).toEqual(cell(2, 3));
    expect(nextUnlocked(s, cell(2, 1), "left", null)).toEqual(cell(3, 1));
  });
  it("Enter goes down columns", () => {
    const s = sheet({ unlocked });
    expect(nextUnlocked(s, cell(2, 1), "down", null)).toEqual(cell(3, 1));
    expect(nextUnlocked(s, cell(3, 1), "down", null)).toEqual(cell(2, 3));
    expect(nextUnlocked(s, cell(2, 3), "up", null)).toEqual(cell(3, 1));
  });
  it("stays inside a selection and gives null without a stop", () => {
    const s = sheet({ unlocked });
    expect(nextUnlocked(s, cell(2, 1), "right", { r0: 2, c0: 1, r1: 3, c1: 1 })).toEqual(cell(3, 1));
    expect(nextUnlocked(s, cell(0, 0), "right", { r0: 8, c0: 8, r1: 9, c1: 9 })).toBeNull();
    expect(nextUnlocked(sheet(), cell(0, 0), "right", null)).toBeNull();
  });
  it("skips the covered cells of a merge, so Tab from B2 over merged B2:C2 and locked D2 reaches E2", () => {
    const s = sheet({ unlocked: [{ r0: 1, c0: 1, r1: 1, c1: 2 }, { r0: 1, c0: 4, r1: 1, c1: 4 }], covered: (r, c) => r === 1 && c === 2 });
    expect(nextUnlocked(s, cell(1, 1), "right", null)).toEqual(cell(1, 4));
    expect(nextUnlocked(s, cell(1, 4), "left", null)).toEqual(cell(1, 1));
  });
  it("skips hidden rows and columns", () => {
    const s = sheet({ unlocked: [{ r0: 1, c0: 1, r1: 4, c1: 2 }], rowHidden: (r) => r === 2 || r === 3, colHidden: (c) => c === 2 });
    expect(nextUnlocked(s, cell(1, 1), "right", null)).toEqual(cell(4, 1));
    expect(nextUnlocked(s, cell(1, 1), "down", null)).toEqual(cell(4, 1));
    expect(nextUnlocked(sheet({ unlocked: [{ r0: 2, c0: 1, r1: 3, c1: 1 }], rowHidden: () => true }), cell(0, 0), "right", null)).toBeNull();
  });
});

describe("paste target (the engine's paste_area)", () => {
  const o = { transpose: false };
  const A1_A10 = { r0: 0, c0: 0, r1: 9, c1: 0 };
  it("external TSV is never tiled", () => {
    expect(pasteArea(A1_A10, { type: "tsv", text: "x" }, o, null)).toEqual({ r0: 0, c0: 0, r1: 0, c1: 0 });
    expect(pasteArea({ r0: 2, c0: 1, r1: 2, c1: 1 }, { type: "tsv", text: "a\tb\nc\n" }, { transpose: true }, null)).toEqual({ r0: 2, c0: 1, r1: 3, c1: 2 });
  });
  it("an internal block fills a larger target, the last copy cut (AOO CopyFromClip)", () => {
    const p = { type: "internal", token: "t" } as const;
    expect(pasteArea(A1_A10, p, o, { rows: 2, cols: 1 })).toEqual(A1_A10);
    expect(pasteArea({ r0: 0, c0: 0, r1: 2, c1: 0 }, p, o, { rows: 2, cols: 1 })).toEqual({ r0: 0, c0: 0, r1: 2, c1: 0 });
    expect(pasteArea({ r0: 0, c0: 0, r1: 0, c1: 0 }, p, o, { rows: 2, cols: 3 })).toEqual({ r0: 0, c0: 0, r1: 1, c1: 2 });
    expect(pasteArea({ r0: 0, c0: 0, r1: 0, c1: 0 }, p, o, null)).toBeNull();
  });
});

describe("overwrite warning pref", () => {
  it("is on by default and persisted", () => {
    expect(DEFAULT_PREFS.pasteOverwriteWarning).toBe(true);
    expect(parsePrefs({ pasteOverwriteWarning: false }).pasteOverwriteWarning).toBe(false);
  });
});
