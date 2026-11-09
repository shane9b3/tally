// @vitest-environment happy-dom
/**
 * Zoom 20–400 % (AOO MINZOOM / MAXZOOM, U18): the headers keep a legible
 * size below 50 %, tiny rows label every n-th row, and the many cells
 * visible at 20 % are fetched in bounded tile batches.
 */
import { DEFAULT_THEME } from "@tally/tokens";
import type { RangeDto } from "@tally/bridge";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { HEADER_HEIGHT, headerScale } from "../src/geometry";
import { Grid } from "../src/grid";
import { rowLabelStep } from "../src/renderer";
import { cellSelection } from "../src/selection";
import { SyntheticSource } from "../src/synthetic";
import { MAX_TILES_PER_BATCH, TILE_COLS, TILE_ROWS } from "../src/tiles";

function stubCanvas(texts: string[]) {
  const ctx = new Proxy(
    {
      fillText: (t: string) => texts.push(t),
      measureText: (t: string) => ({ width: t.length * 7 }),
    } as Record<string, unknown>,
    {
      get: (target, prop: string) => (prop in target ? target[prop] : () => undefined),
      set: (target, prop: string, v) => {
        target[prop] = v;
        return true;
      },
    },
  );
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation((() => ctx) as any);
}

let container: HTMLDivElement;
let texts: string[];

beforeEach(() => {
  texts = [];
  stubCanvas(texts);
  container = document.createElement("div");
  document.body.appendChild(container);
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 1280 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 820 });
});

afterEach(() => {
  container.remove();
  vi.restoreAllMocks();
});

function make(zoom: number) {
  const source = new SyntheticSource(1_048_576, 16_384);
  const asked: RangeDto[][] = [];
  const ensure = source.ensureRanges.bind(source);
  source.ensureRanges = (ranges: RangeDto[]) => {
    asked.push(ranges);
    ensure(ranges);
  };
  const grid = new Grid(container, { source, sheet: 1, theme: DEFAULT_THEME, zoom, selection: cellSelection({ r: 0, c: 0 }), edit: null });
  grid.flush();
  return { grid, asked };
}

describe("zoom 20–400 %", () => {
  it("headers never shrink below their 50 % size", () => {
    expect(headerScale(0.2)).toBe(0.5);
    expect(headerScale(0.5)).toBe(0.5);
    expect(headerScale(4)).toBe(4);
    const { grid } = make(0.2);
    const g = grid.getGeometry();
    expect(g.zoom).toBe(0.2);
    expect(g.headerH).toBe(Math.round(HEADER_HEIGHT * 0.5));
    expect(g.headerW).toBeGreaterThanOrEqual(22);
    grid.destroy();
  });

  it("labels every n-th row when rows are too low for their number", () => {
    expect(rowLabelStep(17, 11)).toBe(1);
    expect(rowLabelStep(3.4, 5.5)).toBe(5);
    expect(rowLabelStep(1, 5.5)).toBe(10);
    expect(rowLabelStep(0, 5.5)).toBe(0);
    const { grid } = make(0.2);
    expect(texts).toContain("A");
    expect(texts).toContain("5");
    expect(texts).toContain("10");
    grid.destroy();
  });

  it("fetches the many cells visible at 20 % in bounded batches", () => {
    const { grid, asked } = make(0.2);
    const g = grid.getGeometry();
    expect(asked.length).toBeGreaterThan(0);
    for (const ranges of asked) expect(ranges.length).toBeLessThanOrEqual(MAX_TILES_PER_BATCH);
    // About a screenful of tiles plus the prefetch ring, not the sheet.
    const tiles = asked.flat().length;
    const visibleTiles = Math.ceil(g.height / (17 * 0.2) / TILE_ROWS + 2) * Math.ceil(g.width / (64 * 0.2) / TILE_COLS + 2);
    expect(tiles).toBeLessThanOrEqual(visibleTiles * 2);
    grid.destroy();
  });

  it("renders at 400 %", () => {
    const { grid } = make(4);
    const g = grid.getGeometry();
    expect(g.zoom).toBe(4);
    expect(g.headerH).toBe(HEADER_HEIGHT * 4);
    expect(texts).toContain("A");
    expect(texts).toContain("1");
    grid.destroy();
  });
});
