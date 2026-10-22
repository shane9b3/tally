// @vitest-environment happy-dom
/**
 * Document fonts on screen: a cell's font family (the document default
 * arrives as the style's family) goes into the canvas font string with
 * metric-compatible substitutes and the app font as fallbacks, and text
 * is measured in that font, so whether a number fits or shows `###`
 * follows the cell's font.
 */
import type { CellView, SheetLayout, StyleDto } from "@tally/bridge";
import { DEFAULT_THEME } from "@tally/tokens";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Grid } from "../src/grid";
import { fitNumber, isGeneralNumberText } from "../src/fitNumber";
import { cellFont, cleanFamily, cssFontFamily, fontSubstitutes } from "../src/renderer";
import { cellSelection } from "../src/selection";
import type { DirtySet, GridDataSource } from "../src/types";

const APP = DEFAULT_THEME.fonts.family;

describe("font strings", () => {
  it("puts the cell's family first, then substitutes, then the app font", () => {
    expect(cssFontFamily("Arial", APP)).toBe(`"Arial", "Liberation Sans", "Arimo", "Helvetica", ${APP}`);
    expect(cssFontFamily("Gill Sans", APP)).toBe(`"Gill Sans", ${APP}`);
    expect(cssFontFamily(null, APP)).toBe(APP);
    expect(cssFontFamily("Inter", APP)).toBe(APP);
    const st = { bold: true, italic: false, fontSizePt: 10, fontFamily: "Times New Roman" } as StyleDto;
    expect(cellFont(st, DEFAULT_THEME, 1)).toMatch(/^600 [\d.]+px "Times New Roman", "Liberation Serif", "Tinos", "Times", /);
  });

  it("uses the PDF writer's substitute groups both ways", () => {
    expect(fontSubstitutes("Liberation Sans")).toEqual(["Arial", "Arimo", "Helvetica"]);
    expect(fontSubstitutes("arimo")).toEqual(["Arial", "Liberation Sans", "Helvetica"]);
    expect(fontSubstitutes("Tinos")).toEqual(["Times New Roman", "Liberation Serif", "Times"]);
    expect(fontSubstitutes("Times")).toEqual(["Times New Roman", "Liberation Serif", "Tinos"]);
    expect(fontSubstitutes("Cousine")).toEqual(["Courier New", "Liberation Mono", "Courier"]);
    expect(fontSubstitutes("Carlito")).toEqual(["Calibri"]);
    expect(fontSubstitutes("Caladea")).toEqual(["Cambria"]);
    expect(fontSubstitutes("Gill Sans")).toEqual([]);
  });

  it("cleans names from files", () => {
    expect(cleanFamily('Evil", 99px "x')).toBe("Evil, 99px x");
    expect(cleanFamily("a\\b\u0000c")).toBe("abc");
    expect(cleanFamily("  ")).toBeNull();
    expect(cleanFamily("N".repeat(500))?.length).toBe(128);
    // A quote cannot end the quoted name early.
    expect(cssFontFamily('x"; color: red', APP)).toBe(`"x; color: red", ${APP}`);
  });
});

let fills: string[];

/** A canvas whose text is 6 px per character in Arial and 8 px otherwise. */
function stubCanvas() {
  const state: Record<string, unknown> = { fillStyle: "", strokeStyle: "", font: "" };
  const ctx = new Proxy(state, {
    get: (target, prop: string) => {
      if (prop === "measureText") return (t: string) => ({ width: t.length * (String(target.font).includes('"Arial"') ? 6 : 8) });
      if (prop === "createLinearGradient") return () => ({ addColorStop: () => undefined });
      if (prop === "fillText") return (t: string) => fills.push(t);
      if (prop in target) return target[prop];
      return () => undefined;
    },
    set: (target, prop: string, v) => {
      target[prop] = v;
      return true;
    },
  });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation((() => ctx) as any);
}

const base: StyleDto = {
  id: 0,
  bold: false,
  italic: false,
  underline: false,
  strike: false,
  fontFamily: null,
  fontSizePt: 10,
  color: null,
  fill: null,
  hAlign: "general",
  vAlign: "bottom",
  wrap: false,
  indent: 0,
  borders: { top: null, bottom: null, left: null, right: null },
  numberFormat: "$#,##0.00",
  display: null,
};
const arial: StyleDto = { ...base, id: 1, fontFamily: "Arial" };

const num = (r: number, text: string, style: number): CellView => ({ r, c: 0, text, kind: "number", num: 1234.56, style, color: null, flags: 0 });

class Source implements GridDataSource {
  private readonly cells = [num(0, "$1,234.56", 0), num(1, "$1,234.56", 1)];
  getLayout(sheet: number): SheetLayout {
    // 9 characters: 54 px in Arial, 72 px otherwise; 80 px column minus
    // 2 × 6 px padding leaves 68 px.
    return { sheet, used: { r0: 0, c0: 0, r1: 1, c1: 0 }, defaultColWidthPx: 80, defaultRowHeightPx: 24, colRuns: [], rowRuns: [], frozen: null, merges: [] };
  }
  getCell(r: number, c: number): CellView | undefined {
    return this.cells.find((v) => v.r === r && v.c === c);
  }
  getStyle(id: number): StyleDto | undefined {
    return id === 1 ? arial : id === 0 ? base : undefined;
  }
  ensureRanges(): void {}
  subscribe(_fn: (d: DirtySet) => void): () => void {
    return () => undefined;
  }
}

let container: HTMLDivElement;

beforeEach(() => {
  fills = [];
  stubCanvas();
  container = document.createElement("div");
  document.body.appendChild(container);
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 600 });
});

afterEach(() => {
  container.remove();
  vi.restoreAllMocks();
});

describe("### follows the cell's font", () => {
  it("fits the number in Arial and shows ### in the wider app font", () => {
    const grid = new Grid(container, { source: new Source(), sheet: 1, theme: DEFAULT_THEME, zoom: 1, selection: cellSelection({ r: 5, c: 5 }), edit: null });
    grid.flush();
    expect(fills.filter((t) => t === "$1,234.56")).toHaveLength(1);
    expect(fills.filter((t) => t === "###")).toHaveLength(1);
    grid.destroy();
  });
});

describe("General shrink-to-fit keeps the locale's decimal separator", () => {
  const w = (s: string) => s.length;
  it("drops decimals and goes scientific with a decimal comma", () => {
    expect(fitNumber(12.6714286, 7, w, "12,6714286", ",")).toBe("12,6714");
    expect(fitNumber(123456789.5, 7, w, "123456789,5", ",")).toBe("1,2E+08");
    expect(fitNumber(12.6714286, 7, w, "12.6714286")).toBe("12.6714");
    expect(isGeneralNumberText("-12,5", ",")).toBe(true);
    expect(isGeneralNumberText("1,000", ".")).toBe(false);
  });
});
