// @vitest-environment happy-dom
/**
 * Bottom-aligned text in rows lower than the default (AOO files often set
 * 0.43 cm rows): the baseline comes down so the top of the text is not
 * drawn above the row; default and taller rows are unchanged.
 */
import type { CellView, SheetLayout, StyleDto } from "@tally/bridge";
import { DEFAULT_THEME } from "@tally/tokens";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Grid } from "../src/grid";
import { fontPx } from "../src/renderer";
import { cellSelection } from "../src/selection";
import type { DirtySet, GridDataSource } from "../src/types";

interface Call {
  name: string;
  args: unknown[];
}

let calls: Call[];

function stubCanvas() {
  const state: Record<string, unknown> = { fillStyle: "", strokeStyle: "" };
  const ctx = new Proxy(state, {
    get: (target, prop: string) => {
      if (prop === "measureText") return (t: string) => ({ width: t.length * 7 });
      if (prop === "createLinearGradient") return () => ({ addColorStop: () => undefined });
      if (prop in target) return target[prop];
      return (...args: unknown[]) => {
        calls.push({ name: prop, args });
      };
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
  fontSizePt: 11,
  color: null,
  fill: null,
  hAlign: "general",
  vAlign: "bottom",
  wrap: false,
  indent: 0,
  borders: { top: null, bottom: null, left: null, right: null },
  numberFormat: "General",
  display: null,
};
const big: StyleDto = { ...base, id: 1, fontSizePt: 13 };

const view = (r: number, c: number, text: string, style = 0): CellView => ({ r, c, text, kind: "text", num: null, style, color: null, flags: 0 });

/** Row 0: 13 pt in a default row; row 1: 11 pt in a 16 px row; row 2: 11 pt in a 40 px row. */
class Source implements GridDataSource {
  private readonly cells = [view(0, 0, "Big", 1), view(1, 0, "Low"), view(2, 0, "Tall")];
  getLayout(sheet: number): SheetLayout {
    return {
      sheet,
      used: { r0: 0, c0: 0, r1: 3, c1: 1 },
      defaultColWidthPx: 100,
      defaultRowHeightPx: 24,
      colRuns: [],
      rowRuns: [
        { start: 1, end: 1, size: 16, hidden: false },
        { start: 2, end: 2, size: 40, hidden: false },
      ],
      frozen: null,
      merges: [],
    };
  }
  getCell(r: number, c: number): CellView | undefined {
    return this.cells.find((v) => v.r === r && v.c === c);
  }
  getStyle(id: number): StyleDto | undefined {
    return id === 1 ? big : id === 0 ? base : undefined;
  }
  ensureRanges(): void {}
  subscribe(_fn: (d: DirtySet) => void): () => void {
    return () => undefined;
  }
}

let container: HTMLDivElement;

beforeEach(() => {
  calls = [];
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

/** The baseline `fillText` used for `text`. */
function baseline(text: string): number {
  const c = calls.find((x) => x.name === "fillText" && x.args[0] === text);
  expect(c).toBeDefined();
  return c!.args[2] as number;
}

describe("bottom-aligned text in low rows", () => {
  it("keeps the top of the text inside the row", () => {
    const grid = new Grid(container, { source: new Source(), sheet: 1, theme: DEFAULT_THEME, zoom: 1, selection: cellSelection({ r: 0, c: 0 }), edit: null });
    grid.flush();
    const cap = (pt: number) => 0.727 * fontPx({ ...base, fontSizePt: pt }, DEFAULT_THEME, 1);
    const header = 24;
    // Default row: the cap height is centred, as before.
    const y0 = baseline("Big");
    expect(y0 - cap(13) - header).toBeCloseTo(header + 24 - y0, 5);
    // 16 px row: the caps start inside the row, the baseline a descent
    // (or less) above its bottom.
    const top1 = header + 24;
    const y1 = baseline("Low");
    expect(y1 - cap(11)).toBeGreaterThanOrEqual(top1);
    expect(top1 + 16 - y1).toBeGreaterThanOrEqual(0.241 * fontPx(base, DEFAULT_THEME, 1) - 1e-9);
    // A tall row keeps the default-row offset from its bottom.
    const top2 = top1 + 16;
    const y2 = baseline("Tall");
    expect(top2 + 40 - y2).toBeCloseTo((24 - cap(11)) / 2, 5);
    grid.destroy();
  });
});
