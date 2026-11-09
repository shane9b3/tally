// @vitest-environment happy-dom
/**
 * v0.3 grid: conditional-format styles (cfStyle), data bars, icon sets,
 * note markers, list dropdowns, links, page breaks, the lock badge and the
 * engine source fetching synthesized styles.
 */
import type { CellView, EngineClient, SheetLayout, StyleDto } from "@tally/bridge";
import { DEFAULT_THEME } from "@tally/tokens";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { iconCount } from "../src/cfIcons";
import { EngineClientSource } from "../src/engineSource";
import { Grid } from "../src/grid";
import { drawStyleId } from "../src/renderer";
import { cellSelection, reduceKey } from "../src/selection";
import type { DirtySet, GridDataSource, GridProps } from "../src/types";

interface Call {
  name: string;
  args: unknown[];
  fillStyle: unknown;
  strokeStyle: unknown;
}

let calls: Call[];

function stubCanvas() {
  const state: Record<string, unknown> = { fillStyle: "", strokeStyle: "" };
  const ctx = new Proxy(state, {
    get: (target, prop: string) => {
      if (prop === "measureText") return (t: string) => ({ width: t.length * 7 });
      if (prop === "createLinearGradient") {
        return (...args: unknown[]) => {
          calls.push({ name: "createLinearGradient", args, fillStyle: target.fillStyle, strokeStyle: target.strokeStyle });
          return { addColorStop: () => undefined };
        };
      }
      if (prop in target) return target[prop];
      return (...args: unknown[]) => {
        calls.push({ name: prop, args, fillStyle: target.fillStyle, strokeStyle: target.strokeStyle });
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
  fontSizePt: 10,
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

const view = (r: number, c: number, text: string, extra: Partial<CellView> = {}): CellView => ({ r, c, text, kind: text === "" ? "empty" : /^\d/.test(text) ? "number" : "text", num: /^\d/.test(text) ? Number(text) : null, style: 0, color: null, flags: 0, ...extra });

/** A fixed in-memory source. */
class FixedSource implements GridDataSource {
  constructor(
    private readonly cells: CellView[],
    private readonly styles: StyleDto[],
  ) {}
  getLayout(sheet: number): SheetLayout {
    return { sheet, used: { r0: 0, c0: 0, r1: 20, c1: 6 }, defaultColWidthPx: 100, defaultRowHeightPx: 24, colRuns: [], rowRuns: [], frozen: null, merges: [] };
  }
  getCell(r: number, c: number): CellView | undefined {
    return this.cells.find((v) => v.r === r && v.c === c);
  }
  getStyle(id: number): StyleDto | undefined {
    return this.styles.find((s) => s.id === id) ?? (id === 0 ? base : undefined);
  }
  ensureRanges(): void {}
  subscribe(_fn: (d: DirtySet) => void): () => void {
    return () => undefined;
  }
}

const CF = 0x4000_0003;
const cells: CellView[] = [
  view(1, 1, "67", { cfStyle: CF }),
  view(2, 1, "40", { cfBar: { fraction: 0.5, color: 0x638ec6ff, negative: false, axis: null, gradient: true } }),
  view(3, 1, "12", { cfIcon: { set: "threeTrafficLights", index: 0 } }),
  view(4, 1, "Paid", { flags: 8 }),
  view(5, 1, "Due", { flags: 16 }),
  view(6, 1, "Docs", { flags: 32, link: "https://example.com" }),
];
const styles: StyleDto[] = [base, { ...base, id: CF, fill: 0xfee2e2ff, bold: true }];

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
  vi.useRealTimers();
});

function make(extra: Partial<GridProps> = {}) {
  const props: GridProps = { source: new FixedSource(cells, styles), sheet: 1, theme: DEFAULT_THEME, zoom: 1, selection: cellSelection({ r: 0, c: 0 }), edit: null, ...extra };
  const grid = new Grid(container, props);
  grid.flush();
  const root = container.querySelector(".tally-grid") as HTMLElement;
  const scroller = root.children[2] as HTMLElement;
  return { grid, root, scroller };
}

/** Column B (100 px wide) after the 44 px row header; row r below the 24 px header. */
const cellX = (dx: number) => 44 + 100 + dx;
const rowY = (r: number) => 24 + r * 24 + 12;

function pointer(el: HTMLElement, type: string, x: number, y: number, init: PointerEventInit = {}) {
  el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, button: 0, pointerId: 1, clientX: x, clientY: y, ...init }));
}

describe("conditional formatting on the canvas", () => {
  it("draws with the synthesized cf style", () => {
    expect(drawStyleId(cells[0])).toBe(CF);
    expect(drawStyleId(view(0, 0, "x"))).toBe(0);
    make();
    expect(calls.some((c) => c.name === "fillRect" && c.fillStyle === "rgb(254,226,226)")).toBe(true);
    // Bold text from the cf style.
    expect(calls.some((c) => c.name === "fillText" && c.args[0] === "67")).toBe(true);
  });

  it("draws data bars, icons and note markers", () => {
    make();
    expect(calls.some((c) => c.name === "createLinearGradient")).toBe(true);
    expect(calls.some((c) => c.name === "arc")).toBe(true); // traffic light
    // The note marker: a filled triangle in the note colour.
    expect(calls.some((c) => c.name === "fill" && c.fillStyle === DEFAULT_THEME.colors.noteMarker)).toBe(true);
    expect(iconCount("fiveRating")).toBe(5);
  });

  it("draws links in the link colour", () => {
    make();
    expect(calls.some((c) => c.name === "fillText" && c.args[0] === "Docs" && c.fillStyle === DEFAULT_THEME.colors.link)).toBe(true);
  });

  it("draws page breaks dashed and the print range", () => {
    make({ pageBreaks: { rows: [{ at: 5, manual: true }], cols: [{ at: 2, manual: false }] }, printRange: { r0: 0, c0: 0, r1: 9, c1: 3 } });
    expect(calls.filter((c) => c.name === "setLineDash").length).toBeGreaterThanOrEqual(2);
    expect(calls.some((c) => c.name === "strokeRect" && c.strokeStyle === DEFAULT_THEME.colors.pageBreak)).toBe(true);
  });
});

describe("Tools ▸ Options ▸ View on the canvas", () => {
  const noteFill = () => calls.some((c) => c.name === "fill" && c.fillStyle === DEFAULT_THEME.colors.noteMarker);
  const clipMarks = () => calls.filter((c) => c.name === "fill" && c.fillStyle === "#ff0000").length;

  it("Comment indicator off hides the note markers", () => {
    // Only the note cell (the traffic-light icon shares the marker's red).
    const { grid } = make({ showNotes: false, source: new FixedSource([view(4, 1, "Paid", { flags: 8 })], styles) });
    expect(noteFill()).toBe(false);
    calls.length = 0;
    grid.update({ showNotes: true });
    grid.flush();
    expect(noteFill()).toBe(true);
    grid.destroy();
  });

  it("Text overflow marks text cut off by a filled neighbour, and only then", () => {
    const source = new FixedSource([view(8, 1, "A very long text that is cut"), view(8, 2, "x"), view(9, 1, "A very long text that overflows")], styles);
    const { grid } = make({ source });
    // Row 8 is cut at its right edge; row 9 overflows into empty cells.
    expect(clipMarks()).toBe(1);
    calls.length = 0;
    grid.update({ clipMarks: false });
    grid.flush();
    expect(clipMarks()).toBe(0);
    grid.destroy();
  });
});

describe("cell buttons, links and notes", () => {
  it("a click on a list cell's ▾ reports the dropdown", () => {
    const onCellButton = vi.fn();
    const { scroller, grid } = make({ onCellButton });
    pointer(scroller, "pointerdown", cellX(100 - 10), rowY(5));
    expect(onCellButton).toHaveBeenCalledWith("dropdown", { r: 5, c: 1 }, expect.objectContaining({ left: expect.any(Number) }));
    expect(grid.getSelection().active).toEqual({ r: 5, c: 1 });
  });

  it("⌘-click on a link opens it; a plain click selects", () => {
    const onOpenLink = vi.fn();
    const onSelectionChange = vi.fn();
    const { scroller } = make({ onOpenLink, onSelectionChange });
    pointer(scroller, "pointerdown", cellX(30), rowY(6));
    pointer(scroller, "pointerup", cellX(30), rowY(6));
    expect(onOpenLink).not.toHaveBeenCalled();
    pointer(scroller, "pointerdown", cellX(30), rowY(6), { metaKey: true });
    expect(onOpenLink).toHaveBeenCalledWith({ r: 6, c: 1 }, "https://example.com");
  });

  it("the active link cell's ↗ opens the link", () => {
    const onCellButton = vi.fn();
    const { scroller } = make({ onCellButton, selection: cellSelection({ r: 6, c: 1 }) });
    pointer(scroller, "pointerdown", cellX(100 - 10), rowY(6));
    expect(onCellButton).toHaveBeenCalledWith("link", { r: 6, c: 1 }, expect.anything());
  });

  it("a push-button cell (DataPilot Filter) is pressed by a click or ⌥↓, and described as a button", () => {
    const onCellButton = vi.fn();
    const onSelectionChange = vi.fn();
    const { scroller, root, grid } = make({ onCellButton, onSelectionChange, pushButtons: [{ r: 0, c: 1 }] });
    // Drawn as a framed button.
    expect(calls.some((c) => c.name === "stroke" && c.strokeStyle === DEFAULT_THEME.colors.headerLine)).toBe(true);
    // Anywhere on the cell.
    pointer(scroller, "pointerdown", cellX(20), rowY(0));
    expect(onCellButton).toHaveBeenCalledWith("button", { r: 0, c: 1 }, expect.objectContaining({ left: expect.any(Number) }));
    expect(grid.getSelection().active).toEqual({ r: 0, c: 1 });
    expect(grid.ariaMirror.cell.textContent).toContain(", button");
    // ⌥↓ on the active button cell presses it; elsewhere it moves as usual.
    onCellButton.mockClear();
    const ta = root.querySelector("textarea") as HTMLTextAreaElement;
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, bubbles: true, cancelable: true }));
    expect(onCellButton).toHaveBeenCalledWith("button", { r: 0, c: 1 }, expect.anything());
    grid.setSelection(cellSelection({ r: 2, c: 1 }));
    onCellButton.mockClear();
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, bubbles: true, cancelable: true }));
    expect(onCellButton).not.toHaveBeenCalled();
  });

  it("hovering a note cell reports it, leaving reports null", () => {
    const onNoteHover = vi.fn();
    const { scroller } = make({ onNoteHover });
    pointer(scroller, "pointermove", cellX(30), rowY(4));
    expect(onNoteHover).toHaveBeenLastCalledWith({ r: 4, c: 1 }, expect.objectContaining({ top: expect.any(Number) }));
    pointer(scroller, "pointermove", cellX(30), rowY(1));
    expect(onNoteHover).toHaveBeenLastCalledWith(null, null);
  });

  it("flashLocked shows the lock badge for a moment", () => {
    vi.useFakeTimers();
    const { grid, root } = make();
    grid.flashLocked({ r: 1, c: 1 }, 500);
    expect(root.dataset.locked).toBe("1,1");
    vi.advanceTimersByTime(600);
    expect(root.dataset.locked).toBeUndefined();
  });

  it("⇧F2 is left to the app (insert note)", () => {
    const lim = { rows: 100, cols: 100, pageRows: 10, pageCols: 5, used: null };
    expect(reduceKey(cellSelection({ r: 0, c: 0 }), "F2", { shift: true, mod: false, alt: false }, lim)).toEqual({ type: "none" });
    expect(reduceKey(cellSelection({ r: 0, c: 0 }), "F2", { shift: false, mod: false, alt: false }, lim)).toEqual({ type: "startEdit" });
  });
});

describe("EngineClientSource", () => {
  it("fetches synthesized cf style ids like any other", async () => {
    const asked: number[][] = [];
    const client = {
      onDocChanged: () => () => undefined,
      sheetLayout: async (_d: number, sheet: number) => new FixedSource([], []).getLayout(sheet),
      cells: async (_d: number, _s: number, ranges: { r0: number; c0: number; r1: number; c1: number }[]) =>
        ranges.map((range) => ({ sheet: 1, range, version: 1, cells: range.r0 === 0 && range.c0 === 0 ? [view(1, 1, "5", { style: 2, cfStyle: CF })] : [] })),
      styles: async (_d: number, ids: number[]) => {
        asked.push(ids);
        return ids.map((id) => ({ ...base, id }));
      },
    } as unknown as EngineClient;
    const src = new EngineClientSource(client, 1, 1);
    src.ensureRanges([{ r0: 0, c0: 0, r1: 5, c1: 5 }]);
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(asked.flat()).toContain(CF);
    expect(src.getStyle(CF)?.id).toBe(CF);
  });
});
