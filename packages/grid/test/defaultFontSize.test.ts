/**
 * One default font size: the grid's fallback for a cell whose style has
 * not arrived repeats the engine's constant by value, so a change on one
 * side fails here.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { DEFAULT_THEME } from "@tally/tokens";
import { describe, expect, it } from "vitest";
import { DEFAULT_FONT_SIZE_PT, fontPx } from "../src/renderer";

const MODEL_RS = fileURLToPath(new URL("../../../crates/tally-model/src/lib.rs", import.meta.url));

describe("default font size", () => {
  it("is the engine's DEFAULT_FONT_SIZE_PT", () => {
    const m = /pub const DEFAULT_FONT_SIZE_PT: f32 = ([0-9.]+);/.exec(readFileSync(MODEL_RS, "utf8"));
    expect(m).not.toBeNull();
    expect(Number(m?.[1])).toBe(DEFAULT_FONT_SIZE_PT);
  });

  it("draws a cell without a style at the size the engine reports for it", () => {
    const none = fontPx(undefined, DEFAULT_THEME, 1);
    expect(none).toBeCloseTo((DEFAULT_THEME.fonts.sizePx * DEFAULT_FONT_SIZE_PT) / 10, 9);
    expect(fontPx(undefined, DEFAULT_THEME, 2)).toBeCloseTo(none * 2, 9);
  });
});
