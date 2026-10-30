/**
 * Canvas renderer (docs/architecture-plan.md §4.2).
 *
 * - main canvas: fills, gridlines, overflowing text, borders, headers;
 *   redrawn on scroll and data changes, only for visible cells;
 * - overlay canvas: selection tint, range border, fill handle, active
 *   header highlight, resize guide; redrawn on selection changes (cheap).
 *
 * DPR aware: the backing store is `css × devicePixelRatio`, lines are
 * snapped to device pixels. Text widths are cached per (font, text).
 */
import type { BorderLineDto, BordersDto, CellPos, CellView, RangeDto, StyleDto } from "@tally/bridge";
import type { Theme } from "@tally/tokens";
import { colToLetters } from "./colLetters";
import { fitNumber, isGeneralNumberText } from "./fitNumber";
import { snapFill, type Geometry, type Pane, type Rect } from "./geometry";
import { Lru } from "./lru";
import { drawCfIcon, drawDataBar } from "./cfIcons";
import { CELL_FLAGS, type FilterButtons, type GridDataSource, type OutlineProps, type PageBreaks, type RefHighlight, type Selection, type TraceArrow } from "./types";

/** Horizontal text padding inside a cell at zoom 100%. */
export const CELL_PADDING = 6;
/** How far (in columns) to look outside the viewport for text overflowing into it. */
export const OVERFLOW_SCAN = 32;

const MEASURE_CACHE = 5000;
/** Inter's cap height as a fraction of the font size. */
const CAP_HEIGHT = 0.727;
/** Inter's descent (494 of 2048 units) as a fraction of the font size. */
const DESCENT = 0.241;
/** Row height (zoom 100%) whose vertical centre the text baseline is tuned for. */
const DEFAULT_ROW_REF = 24;

/** Measures text widths with a per-(font, text) LRU cache. */
export class TextMeasurer {
  private readonly cache = new Lru<string, number>(MEASURE_CACHE);
  private font = "";
  hits = 0;
  misses = 0;

  constructor(private readonly ctx: CanvasRenderingContext2D) {}

  measure(font: string, text: string): number {
    const key = `${font}\u0000${text}`;
    const hit = this.cache.get(key);
    if (hit !== undefined) {
      this.hits++;
      return hit;
    }
    this.misses++;
    if (this.font !== font) {
      this.ctx.font = font;
      this.font = font;
    }
    const w = this.ctx.measureText(text).width;
    this.cache.set(key, w);
    return w;
  }

  clear(): void {
    this.cache.clear();
    this.font = "";
  }
}

const colorCache = new Map<number, string>();

/** `0xRRGGBBAA` → CSS colour. */
export function cssColor(rgba: number): string {
  let s = colorCache.get(rgba);
  if (s === undefined) {
    const n = rgba >>> 0;
    const r = (n >>> 24) & 255;
    const g = (n >>> 16) & 255;
    const b = (n >>> 8) & 255;
    const a = (n & 255) / 255;
    s = a >= 1 ? `rgb(${r},${g},${b})` : `rgba(${r},${g},${b},${Math.round(a * 1000) / 1000})`;
    if (colorCache.size > 4096) colorCache.clear();
    colorCache.set(rgba, s);
  }
  return s;
}

/** Cell font size in px for a style (10 pt = the theme size). */
export function fontPx(style: StyleDto | undefined, theme: Theme, zoom: number): number {
  const pt = style?.fontSizePt ?? 10;
  return (theme.fonts.sizePx * pt * zoom) / 10;
}

/**
 * Metric-compatible stand-ins tried after a document font that may not be
 * installed: the same groups, in the same order, as the PDF writer
 * (`family_candidates` in crates/tally-io-pdf/src/fonts.rs).
 */
const FONT_GROUPS: readonly (readonly [readonly string[], readonly string[]])[] = [
  [["arial", "liberation sans", "arimo", "helvetica", "helvetica neue"], ["Arial", "Liberation Sans", "Arimo", "Helvetica"]],
  [["times new roman", "liberation serif", "tinos", "times"], ["Times New Roman", "Liberation Serif", "Tinos", "Times"]],
  [["courier new", "liberation mono", "cousine", "courier"], ["Courier New", "Liberation Mono", "Cousine", "Courier"]],
  [["calibri", "carlito"], ["Calibri", "Carlito"]],
  [["cambria", "caladea"], ["Cambria", "Caladea"]],
  [["menlo", "monospace"], ["Menlo", "Courier New", "Liberation Mono"]],
];

/** The substitutes of `name` (without `name` itself). */
export function fontSubstitutes(name: string): string[] {
  const lower = name.toLowerCase();
  const group = FONT_GROUPS.find(([keys]) => keys.includes(lower));
  return group ? group[1].filter((n) => n.toLowerCase() !== lower) : [];
}

/** Longest family name used (names come from untrusted files). */
const MAX_FAMILY_LEN = 128;

/**
 * A document font family as a CSS family name: quotes, backslashes and
 * control characters removed, at most 128 characters; `null` when nothing
 * usable is left (the app font is used).
 */
export function cleanFamily(family: string | null | undefined): string | null {
  if (!family) return null;
  // eslint-disable-next-line no-control-regex
  const s = family.replace(/["\\\u0000-\u001f\u007f]/g, "").trim().slice(0, MAX_FAMILY_LEN).trim();
  return s.length > 0 ? s : null;
}

const familyCache = new Map<string, string>();

/**
 * The CSS `font-family` list for a cell: the document font, its
 * metric-compatible substitutes, then the app font (`appFamily`). The
 * browser falls through names that are not installed.
 */
export function cssFontFamily(family: string | null | undefined, appFamily: string): string {
  const name = cleanFamily(family);
  if (!name || /^inter( variable)?$/i.test(name)) return appFamily;
  const key = `${name}\u0000${appFamily}`;
  let css = familyCache.get(key);
  if (css === undefined) {
    const subs = fontSubstitutes(name);
    css = [name, ...subs].map((n) => `"${n}"`).join(", ") + `, ${appFamily}`;
    if (familyCache.size > 1024) familyCache.clear();
    familyCache.set(key, css);
  }
  return css;
}

/** The `ctx.font` string for a cell (its font family, else the app font). */
export function cellFont(style: StyleDto | undefined, theme: Theme, zoom: number): string {
  const px = Math.round(fontPx(style, theme, zoom) * 100) / 100;
  const family = cssFontFamily(style?.fontFamily, theme.fonts.family);
  return `${style?.italic ? "italic " : ""}${style?.bold ? "600" : "400"} ${px}px ${family}`;
}

export type EffectiveAlign = "left" | "center" | "right";

/** General alignment: numbers right, booleans and errors centred, text left. */
export function effectiveAlign(cell: CellView, style: StyleDto | undefined): EffectiveAlign {
  const h = style?.hAlign ?? "general";
  if (h === "left" || h === "justify") return "left";
  if (h === "right") return "right";
  if (h === "center") return "center";
  switch (cell.kind) {
    case "number":
      return "right";
    case "bool":
    case "error":
      return "center";
    default:
      return "left";
  }
}

/** Stands in for cells inside merged areas when checking where text may overflow. */
const MERGED_CELL: CellView = { r: -1, c: -1, text: " ", kind: "text", num: null, style: 0, color: null, flags: 0 };

/**
 * The ▾ button of an AutoFilter header cell (screen px): a square at the
 * right edge of the cell, vertically centred.
 */
export function filterButtonRect(geo: Geometry, r: number, c: number): Rect {
  const h = geo.rowH(r);
  const w = geo.colW(c);
  const s = Math.max(0, Math.min(h - 4 * geo.zoom, 16 * geo.zoom, w - 4 * geo.zoom));
  return { x: geo.colRight(c) - s - 3 * geo.zoom, y: geo.rowY(r) + (h - s) / 2, w: s, h: s };
}

/** The style id to draw a cell with: the conditional-format style when one matches. */
export function drawStyleId(cell: CellView | undefined): number {
  return cell ? (cell.cfStyle ?? cell.style) : 0;
}

/** Side of a conditional-format icon in a row of height `h` (screen px). */
export function cfIconSize(h: number, zoom: number): number {
  return Math.max(6, Math.min(h - 6 * zoom, 14 * zoom));
}

/**
 * The validation ▾ of a list cell (screen px): a small square at the right
 * edge of the cell, vertically centred (the same place as a filter button).
 */
export function dropdownButtonRect(geo: Geometry, r: number, c: number): Rect {
  return filterButtonRect(geo, r, c);
}

/**
 * The ↗ of the active link cell (screen px): a small square at the right
 * edge, left of a dropdown if the cell has both.
 */
export function linkButtonRect(geo: Geometry, r: number, c: number, withDropdown: boolean): Rect {
  const b = filterButtonRect(geo, r, c);
  return withDropdown ? { ...b, x: b.x - b.w - 2 * geo.zoom } : b;
}

function isEmptyCell(cell: CellView | undefined): boolean {
  return cell === undefined || cell.kind === "empty" || cell.text === "";
}

export interface MainFrame {
  geo: Geometry;
  source: GridDataSource;
  theme: Theme;
  dpr: number;
  /** AutoFilter header buttons to draw. */
  filterButtons?: FilterButtons | null;
  /** Page-break lines ("View ▸ Page breaks"). */
  pageBreaks?: PageBreaks | null;
  /** The print range outline. */
  printRange?: RangeDto | null;
  /** Outline gutter groups (wave 5 seam; the gutter sizes are in `geo`). */
  outline?: OutlineProps | null;
  /** Page Break Preview: a "Page N" watermark per page. */
  pageBreakPreview?: boolean;
  /** View ▸ Grid lines (default `true`). */
  showGridlines?: boolean;
  /** View ▸ Column and row headers (default `true`). */
  showHeaders?: boolean;
  /** The locale's decimal separator (default `.`). */
  decimalSep?: string;
}

export interface OverlayFrame extends MainFrame {
  /** Detective arrows (wave 5 seam). */
  traces?: readonly TraceArrow[] | null;
  selection: Selection;
  /** Hide the fill handle and active-cell clearing while editing. */
  editing: boolean;
  /** A chart or picture is selected: the cell selection and header highlight are not drawn. */
  objectSelected?: boolean;
  /** The selection is not drawn (an edit of another sheet's cell picks references here). */
  hideSelection?: boolean;
  /** Header gutter under the pointer. */
  hoverGutter: { axis: "col" | "row"; index: number } | null;
  /** Live resize: the axis, index and its new far edge (screen px). */
  resizeGuide: { axis: "col" | "row"; index: number; pos: number } | null;
  /** Fill-handle drag preview: the target range. */
  fillPreview?: RangeDto | null;
  /** Coloured reference outlines while editing a formula. */
  refHighlights?: readonly RefHighlight[];
  /** The array-formula block of the active cell (thin outline). */
  arrayRange?: RangeDto | null;
  /** The copied/cut range (dashed outline). */
  clipRange?: RangeDto | null;
  /** A cell whose edit was refused (protected): a lock badge. */
  lockedCell?: CellPos | null;
  /** Detective "Mark invalid data": red circles around these cells (W5-5). */
  invalidCells?: readonly CellPos[] | null;
  /** A page-break line being dragged (Page Break Preview): its live screen position. */
  breakDrag?: { axis: "row" | "col"; pos: number } | null;
}

/** Per-frame statistics, for the perf demo. */
export interface FrameStats {
  cells: number;
  textDraws: number;
  ms: number;
}

export class Renderer {
  readonly measurer: TextMeasurer;
  private readonly main: CanvasRenderingContext2D;
  private readonly overlay: CanvasRenderingContext2D;
  private readonly fitCache = new Lru<string, string>(MEASURE_CACHE);
  private cssW = 0;
  private cssH = 0;
  private dpr = 1;
  private curFont = "";
  private curFill = "";
  lastStats: FrameStats = { cells: 0, textDraws: 0, ms: 0 };

  constructor(
    private readonly mainCanvas: HTMLCanvasElement,
    private readonly overlayCanvas: HTMLCanvasElement,
    measureCanvas: HTMLCanvasElement,
  ) {
    const m = mainCanvas.getContext("2d", { alpha: false });
    const o = overlayCanvas.getContext("2d");
    const t = measureCanvas.getContext("2d");
    if (!m || !o || !t) throw new Error("@tally/grid: 2D canvas is not available");
    this.main = m;
    this.overlay = o;
    this.measurer = new TextMeasurer(t);
  }

  /** Resizes both canvases' backing stores. */
  resize(cssW: number, cssH: number, dpr: number): void {
    if (cssW === this.cssW && cssH === this.cssH && dpr === this.dpr) return;
    this.cssW = cssW;
    this.cssH = cssH;
    this.dpr = dpr;
    for (const c of [this.mainCanvas, this.overlayCanvas]) {
      c.width = Math.max(1, Math.round(cssW * dpr));
      c.height = Math.max(1, Math.round(cssH * dpr));
      c.style.width = `${cssW}px`;
      c.style.height = `${cssH}px`;
    }
    this.curFont = "";
    this.curFill = "";
  }

  clearCaches(): void {
    this.measurer.clear();
    this.fitCache.clear();
    this.curFont = "";
  }

  private setFont(font: string): void {
    if (font !== this.curFont) {
      this.main.font = font;
      this.curFont = font;
    }
  }

  private setFill(color: string): void {
    if (color !== this.curFill) {
      this.main.fillStyle = color;
      this.curFill = color;
    }
  }

  // -------------------------------------------------------------------------
  // Main layer
  // -------------------------------------------------------------------------

  drawMain(f: MainFrame): FrameStats {
    const t0 = now();
    const ctx = this.main;
    const { geo, theme, dpr } = f;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.curFont = "";
    this.curFill = "";
    this.setFill(theme.colors.bg);
    ctx.fillRect(0, 0, geo.width, geo.height);

    let count = 0;
    let textDraws = 0;
    for (const pane of geo.panes()) {
      const s = this.drawPane(f, pane);
      count += s.cells;
      textDraws += s.textDraws;
    }
    this.curFill = "";
    this.curFont = "";

    this.drawHeaders(f);
    this.drawFrozenDividers(f);
    const stats = { cells: count, textDraws, ms: now() - t0 };
    this.lastStats = stats;
    return stats;
  }

  /** The 1 px lines between frozen panes and the scrolling area. */
  private drawFrozenDividers(f: MainFrame): void {
    const { geo, theme, dpr } = f;
    const ctx = this.main;
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    this.setFill(theme.colors.frozenLine);
    if (geo.frozen.cols > 0 && geo.frozenW > 0) {
      const x = snapFill(geo.headerW + geo.frozenW, dpr) - lw;
      if (x < geo.width) ctx.fillRect(x, 0, lw, geo.height);
    }
    if (geo.frozen.rows > 0 && geo.frozenH > 0) {
      const y = snapFill(geo.headerH + geo.frozenH, dpr) - lw;
      if (y < geo.height) ctx.fillRect(0, y, geo.width, lw);
    }
    this.curFill = "";
  }

  /** Draws the cells of one pane (fills, gridlines, text, merges, borders, filter buttons), clipped to it. */
  private drawPane(f: MainFrame, pane: Pane): { cells: number; textDraws: number } {
    const ctx = this.main;
    const { geo, theme, dpr } = f;
    const vis: RangeDto = { r0: pane.r0, c0: pane.c0, r1: pane.r1, c1: pane.c1 };
    const nr = vis.r1 - vis.r0 + 1;
    const nc = vis.c1 - vis.c0 + 1;
    if (nr <= 0 || nc <= 0 || pane.clip.w <= 0 || pane.clip.h <= 0) return { cells: 0, textDraws: 0 };
    // Column/row edges for the visible span (the far edge from the last index, so it stays in this pane).
    const xs = new Float64Array(nc + 1);
    const ys = new Float64Array(nr + 1);
    for (let i = 0; i < nc; i++) xs[i] = geo.colX(vis.c0 + i);
    xs[nc] = geo.colRight(vis.c1);
    for (let i = 0; i < nr; i++) ys[i] = geo.rowY(vis.r0 + i);
    ys[nr] = geo.rowBottom(vis.r1);

    // Merged areas touching this pane.
    const merges = geo.merges.filter((m) => m.r0 <= vis.r1 && m.r1 >= vis.r0 && m.c0 <= vis.c1 && m.c1 >= vis.c0);
    const mergeAt = (r: number, c: number): RangeDto | undefined => {
      for (const m of merges) if (r >= m.r0 && r <= m.r1 && c >= m.c0 && c <= m.c1) return m;
      return undefined;
    };

    // Fetch cells once per frame.
    const cells: (CellView | undefined)[] = new Array<CellView | undefined>(nr * nc);
    const styles: (StyleDto | undefined)[] = new Array<StyleDto | undefined>(nr * nc);
    let count = 0;
    for (let i = 0; i < nr; i++) {
      if ((ys[i + 1] ?? 0) - (ys[i] ?? 0) <= 0) continue;
      for (let j = 0; j < nc; j++) {
        const cell = f.source.getCell(vis.r0 + i, vis.c0 + j);
        if (cell === undefined) continue;
        cells[i * nc + j] = cell;
        styles[i * nc + j] = f.source.getStyle(drawStyleId(cell));
        count++;
      }
    }

    ctx.save();
    ctx.beginPath();
    ctx.rect(pane.clip.x, pane.clip.y, pane.clip.w, pane.clip.h);
    ctx.clip();

    // 1. Fills (merged areas are filled from their anchor below).
    for (let i = 0; i < nr; i++) {
      for (let j = 0; j < nc; j++) {
        const st = styles[i * nc + j];
        if (!st || st.fill === null || (st.fill & 255) === 0) continue;
        if (merges.length > 0 && mergeAt(vis.r0 + i, vis.c0 + j)) continue;
        this.setFill(cssColor(st.fill));
        const x0 = snapFill(xs[j] ?? 0, dpr);
        const y0 = snapFill(ys[i] ?? 0, dpr);
        ctx.fillRect(x0, y0, snapFill(xs[j + 1] ?? 0, dpr) - x0, snapFill(ys[i + 1] ?? 0, dpr) - y0);
      }
    }

    // 1b. Conditional-format data bars (over the fill, under the text).
    for (let i = 0; i < nr; i++) {
      for (let j = 0; j < nc; j++) {
        const cell = cells[i * nc + j];
        const bar = cell?.cfBar;
        if (!bar) continue;
        const y0 = ys[i] ?? 0;
        const h = (ys[i + 1] ?? 0) - y0;
        // An icon of the same cell sits left of the bar.
        const iconSpace = cell.cfIcon ? cfIconSize(h, geo.zoom) + 3 * geo.zoom : 0;
        const x0 = (xs[j] ?? 0) + iconSpace;
        drawDataBar(ctx, bar, x0, y0, (xs[j + 1] ?? 0) - x0, h, geo.zoom);
      }
    }
    this.curFill = "";

    // 2. Gridlines: one device-aligned 1 CSS px line on the right/bottom edge of each cell
    //    (View ▸ Grid lines off: none).
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    this.setFill(theme.colors.gridLine);
    const top = Math.max(pane.clip.y, ys[0] ?? pane.clip.y);
    const left = Math.max(pane.clip.x, xs[0] ?? pane.clip.x);
    const bottom = Math.min(pane.clip.y + pane.clip.h, ys[nr] ?? geo.height);
    const right = Math.min(pane.clip.x + pane.clip.w, xs[nc] ?? geo.width);
    if (f.showGridlines !== false) {
      for (let j = 1; j <= nc; j++) {
        const x = xs[j] ?? 0;
        if (x === xs[j - 1]) continue;
        ctx.fillRect(snapFill(x, dpr) - lw, top, lw, bottom - top);
      }
      for (let i = 1; i <= nr; i++) {
        const y = ys[i] ?? 0;
        if (y === ys[i - 1]) continue;
        ctx.fillRect(left, snapFill(y, dpr) - lw, right - left, lw);
      }
    }

    // 2b. Merged areas: one fill over the whole rect hides the inner gridlines.
    for (const m of merges) {
      const r = geo.rangeRect(m, false);
      if (r.w <= 0 || r.h <= 0) continue;
      const anchor = f.source.getCell(m.r0, m.c0);
      const st = f.source.getStyle(drawStyleId(anchor));
      const fill = st && st.fill !== null && (st.fill & 255) > 0 ? cssColor(st.fill) : theme.colors.bg;
      this.setFill(fill);
      const x0 = snapFill(r.x, dpr);
      const y0 = snapFill(r.y, dpr);
      ctx.fillRect(x0, y0, snapFill(r.x + r.w, dpr) - lw - x0, snapFill(r.y + r.h, dpr) - lw - y0);
    }

    // 3. Text (with overflow), per row. Cells inside merged areas are drawn with their area.
    let textDraws = 0;
    const bounds = { cMin: pane.cMin, cMax: pane.cMax };
    const merged = (r: number, c: number): boolean => merges.length > 0 && mergeAt(r, c) !== undefined;
    for (let i = 0; i < nr; i++) {
      const rowTop = ys[i] ?? 0;
      const h = (ys[i + 1] ?? 0) - rowTop;
      if (h <= 0) continue;
      const r = vis.r0 + i;
      const getRowCell = (c: number): CellView | undefined =>
        merged(r, c) ? MERGED_CELL : c >= vis.c0 && c <= vis.c1 ? cells[i * nc + (c - vis.c0)] : f.source.getCell(r, c);
      // Cells left of the viewport whose text may overflow into it.
      for (let c = vis.c0 - 1; c >= Math.max(pane.cMin, vis.c0 - OVERFLOW_SCAN); c--) {
        if (merged(r, c)) break;
        const cell = f.source.getCell(r, c);
        if (isEmptyCell(cell)) continue;
        if (cell) textDraws += this.drawCellText(f, r, c, cell, rowTop, h, getRowCell, vis, bounds);
        break;
      }
      for (let j = 0; j < nc; j++) {
        const cell = cells[i * nc + j];
        if (isEmptyCell(cell) || !cell) continue;
        if (merged(r, vis.c0 + j)) continue;
        textDraws += this.drawCellText(f, r, vis.c0 + j, cell, rowTop, h, getRowCell, vis, bounds, styles[i * nc + j]);
      }
      // Cells right of the viewport (right-aligned or centred overflow).
      for (let c = vis.c1 + 1; c <= Math.min(pane.cMax, vis.c1 + OVERFLOW_SCAN); c++) {
        if (merged(r, c)) break;
        const cell = f.source.getCell(r, c);
        if (isEmptyCell(cell)) continue;
        if (cell) textDraws += this.drawCellText(f, r, c, cell, rowTop, h, getRowCell, vis, bounds);
        break;
      }
    }
    for (const m of merges) {
      const anchor = f.source.getCell(m.r0, m.c0);
      if (isEmptyCell(anchor) || !anchor) continue;
      const r = geo.rangeRect(m, false);
      if (r.w <= 0 || r.h <= 0) continue;
      const box = { x: geo.colX(m.c0), w: geo.colRight(m.c1) - geo.colX(m.c0) };
      const y0 = geo.rowY(m.r0);
      textDraws += this.drawCellText(f, m.r0, m.c0, anchor, y0, geo.rowBottom(m.r1) - y0, () => MERGED_CELL, vis, bounds, undefined, box);
    }

    // 4. Borders (a merged area takes its anchor's borders around the whole area).
    const drawn = new Set<RangeDto>();
    for (let i = 0; i < nr; i++) {
      for (let j = 0; j < nc; j++) {
        const r = vis.r0 + i;
        const c = vis.c0 + j;
        const m = merges.length > 0 ? mergeAt(r, c) : undefined;
        if (m) {
          if (drawn.has(m)) continue;
          drawn.add(m);
          const anchor = f.source.getCell(m.r0, m.c0);
          const st = f.source.getStyle(drawStyleId(anchor));
          if (st) this.drawBorders(st.borders, geo.colX(m.c0), geo.colRight(m.c1), geo.rowY(m.r0), geo.rowBottom(m.r1), dpr);
          continue;
        }
        const st = styles[i * nc + j];
        if (!st) continue;
        const b = st.borders;
        if (!b.top && !b.bottom && !b.left && !b.right) continue;
        this.drawBorders(b, xs[j] ?? 0, xs[j + 1] ?? 0, ys[i] ?? 0, ys[i + 1] ?? 0, dpr);
      }
    }

    // 4b. Cell decorations: icon sets, note markers, list dropdowns.
    const fb = f.filterButtons;
    for (let i = 0; i < nr; i++) {
      const h = (ys[i + 1] ?? 0) - (ys[i] ?? 0);
      if (h <= 0) continue;
      for (let j = 0; j < nc; j++) {
        const cell = cells[i * nc + j];
        if (!cell || (cell.flags & (CELL_FLAGS.note | CELL_FLAGS.dropdown)) === 0 && !cell.cfIcon) continue;
        const r = vis.r0 + i;
        const c = vis.c0 + j;
        const w = (xs[j + 1] ?? 0) - (xs[j] ?? 0);
        if (w <= 0) continue;
        if (cell.cfIcon) {
          const size = cfIconSize(h, geo.zoom);
          drawCfIcon(ctx, cell.cfIcon, (xs[j] ?? 0) + 3 * geo.zoom, (ys[i] ?? 0) + (h - size) / 2, size);
        }
        if (cell.flags & CELL_FLAGS.dropdown && !(fb && fb.row === r && c >= fb.c0 && c <= fb.c1)) {
          this.drawDropdownButton(geo, r, c, theme, dpr);
        }
        if (cell.flags & CELL_FLAGS.note) {
          const s = Math.min(7 * geo.zoom, w / 3, h / 3);
          const xr = (xs[j + 1] ?? 0) - lw;
          const yt = ys[i] ?? 0;
          ctx.beginPath();
          ctx.moveTo(xr - s, yt);
          ctx.lineTo(xr, yt);
          ctx.lineTo(xr, yt + s);
          ctx.closePath();
          ctx.fillStyle = theme.colors.noteMarker;
          ctx.fill();
        }
      }
    }
    this.curFill = "";

    // 4c. Page breaks and the print range ("View ▸ Page breaks").
    if (f.pageBreaks || f.printRange) this.drawPageBreaks(f, pane, vis);

    // 5. AutoFilter buttons in the header row of the filter area.
    if (fb && fb.row >= vis.r0 && fb.row <= vis.r1 && geo.rowH(fb.row) > 0) {
      for (let c = Math.max(fb.c0, vis.c0); c <= Math.min(fb.c1, vis.c1); c++) {
        if (geo.colW(c) <= 0) continue;
        this.drawFilterButton(geo, fb.row, c, fb.active.includes(c), theme, dpr);
      }
    }

    ctx.restore();
    this.curFill = "";
    this.curFont = "";
    return { cells: count, textDraws };
  }

  /** The four borders of a cell (or merged area) between x0..x1 and y0..y1 (cell edges, screen px). */
  private drawBorders(b: BordersDto, x0: number, x1: number, y0: number, y1: number, dpr: number): void {
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    if (b.top) this.borderLine(b.top, x0 - lw, x1, y0, true, dpr);
    if (b.bottom) this.borderLine(b.bottom, x0 - lw, x1, y1, true, dpr);
    if (b.left) this.borderLine(b.left, y0 - lw, y1, x0, false, dpr);
    if (b.right) this.borderLine(b.right, y0 - lw, y1, x1, false, dpr);
  }

  /**
   * One border line along a cell edge at `pos` (the gridline position: the
   * line covers the gridline and grows to both sides for wider widths),
   * from `a` to `b` on the other axis. Dashed, dotted and double styles.
   */
  private borderLine(line: BorderLineDto, a: number, b: number, pos: number, horizontal: boolean, dpr: number): void {
    const ctx = this.main;
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    const width = Math.max(lw, Math.round(Math.max(1, line.widthPx) * dpr) / dpr);
    const base = snapFill(pos, dpr) - lw - Math.floor((width - lw) / 2 / lw) * lw;
    this.setFill(cssColor(line.color));
    const rect = (s: number, len: number, off: number, w: number): void => {
      if (horizontal) ctx.fillRect(snapFill(s, dpr), base + off, len, w);
      else ctx.fillRect(base + off, snapFill(s, dpr), w, len);
    };
    const len = snapFill(b, dpr) - snapFill(a, dpr);
    if (len <= 0) return;
    switch (line.style) {
      case "double": {
        // Two hairlines with a gap: at least 3 device px overall.
        const total = Math.max(3 * lw, width);
        const start = snapFill(pos, dpr) - lw - Math.floor((total - lw) / 2 / lw) * lw;
        const off = start - base;
        rect(a, len, off, lw);
        rect(a, len, off + total - lw, lw);
        return;
      }
      case "dashed":
      case "dotted": {
        const on = line.style === "dashed" ? Math.max(3, 3 * width) : width;
        const gap = line.style === "dashed" ? Math.max(2, 2 * width) : width;
        for (let s = a; s < b; s += on + gap) rect(s, Math.min(on, b - s), 0, width);
        return;
      }
      default:
        rect(a, len, 0, width);
    }
  }

  /** The ▾ button at the right of an AutoFilter header cell. */
  private drawFilterButton(geo: Geometry, r: number, c: number, active: boolean, theme: Theme, dpr: number): void {
    const ctx = this.main;
    const b = filterButtonRect(geo, r, c);
    if (b.w <= 0) return;
    ctx.save();
    ctx.beginPath();
    roundRect(ctx, b.x + 0.5, b.y + 0.5, b.w - 1, b.h - 1, 3 * geo.zoom);
    ctx.fillStyle = active ? theme.colors.accent : theme.colors.headerBg;
    ctx.fill();
    ctx.lineWidth = 1 / dpr;
    ctx.strokeStyle = active ? theme.colors.accent : theme.colors.headerLine;
    ctx.stroke();
    // The caret (a funnel would be busier at this size).
    const cx = b.x + b.w / 2;
    const cy = b.y + b.h / 2 + 0.5 * geo.zoom;
    const s = Math.max(2.5, 3.2 * geo.zoom);
    ctx.beginPath();
    ctx.moveTo(cx - s, cy - s / 2);
    ctx.lineTo(cx + s, cy - s / 2);
    ctx.lineTo(cx, cy + s / 2 + 0.5);
    ctx.closePath();
    ctx.fillStyle = active ? "#ffffff" : theme.colors.headerText;
    ctx.fill();
    ctx.restore();
    this.curFill = "";
  }

  /** The ▾ of a list-validation cell: a subtle chevron (the active cell gets a button in the overlay). */
  private drawDropdownButton(geo: Geometry, r: number, c: number, theme: Theme, _dpr: number): void {
    const ctx = this.main;
    const b = dropdownButtonRect(geo, r, c);
    if (b.w <= 4) return;
    ctx.save();
    ctx.globalAlpha = 0.55;
    const cx = b.x + b.w / 2;
    const cy = b.y + b.h / 2 + 0.5 * geo.zoom;
    const s = Math.max(2.5, 3 * geo.zoom);
    ctx.beginPath();
    ctx.moveTo(cx - s, cy - s / 2);
    ctx.lineTo(cx, cy + s / 2);
    ctx.lineTo(cx + s, cy - s / 2);
    ctx.lineWidth = Math.max(1.2, 1.4 * geo.zoom);
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.strokeStyle = theme.colors.headerText;
    ctx.stroke();
    ctx.restore();
    this.curFill = "";
  }

  /** Dashed page-break lines (manual ones bolder) and the print range outline, clipped to the pane. */
  private drawPageBreaks(f: MainFrame, pane: Pane, vis: RangeDto): void {
    const ctx = this.main;
    const { geo, theme } = f;
    const top = pane.clip.y;
    const left = pane.clip.x;
    const bottom = pane.clip.y + pane.clip.h;
    const right = pane.clip.x + pane.clip.w;
    ctx.save();
    ctx.strokeStyle = theme.colors.pageBreak;
    const line = (manual: boolean): void => {
      ctx.lineWidth = manual ? 2 : 1.25;
      ctx.globalAlpha = manual ? 0.95 : 0.7;
      ctx.setLineDash(manual ? [7, 3] : [4, 4]);
    };
    for (const b of f.pageBreaks?.rows ?? []) {
      if (b.at <= vis.r0 || b.at > vis.r1 + 1) continue;
      const y = Math.round(geo.rowY(b.at)) - 0.5;
      line(b.manual);
      ctx.beginPath();
      ctx.moveTo(left, y);
      ctx.lineTo(right, y);
      ctx.stroke();
    }
    for (const b of f.pageBreaks?.cols ?? []) {
      if (b.at <= vis.c0 || b.at > vis.c1 + 1) continue;
      const x = Math.round(geo.colX(b.at)) - 0.5;
      line(b.manual);
      ctx.beginPath();
      ctx.moveTo(x, top);
      ctx.lineTo(x, bottom);
      ctx.stroke();
    }
    const pr = f.printRange;
    if (pr && pr.r0 <= vis.r1 && pr.r1 >= vis.r0 && pr.c0 <= vis.c1 && pr.c1 >= vis.c0) {
      const r = geo.rangeRect(pr, false);
      ctx.setLineDash([]);
      ctx.globalAlpha = 0.9;
      ctx.lineWidth = 2;
      ctx.strokeRect(Math.round(r.x) + 0.5, Math.round(r.y) + 0.5, Math.max(0, r.w - 2), Math.max(0, r.h - 2));
    }
    if (f.pageBreakPreview && f.pageBreaks) this.drawPageWatermarks(f, f.pageBreaks, vis);
    ctx.restore();
    this.curFill = "";
  }

  /**
   * Page Break Preview stub (wave 5 seam): "Page N" in the middle of every
   * page rectangle formed by the break lines, numbered down the first column
   * of pages, then the next (AOO's default order). Pages start at the print
   * range's origin (else A1) and end at the last break (or the print range).
   */
  private drawPageWatermarks(f: MainFrame, breaks: PageBreaks, vis: RangeDto): void {
    const ctx = this.main;
    const { geo, theme } = f;
    const pr = f.printRange ?? null;
    const r0 = pr?.r0 ?? 0;
    const c0 = pr?.c0 ?? 0;
    const rowEdges = [r0, ...breaks.rows.map((b) => b.at).filter((at) => at > r0).sort((a, b) => a - b)];
    const colEdges = [c0, ...breaks.cols.map((b) => b.at).filter((at) => at > c0).sort((a, b) => a - b)];
    if (pr) {
      rowEdges.push(pr.r1 + 1);
      colEdges.push(pr.c1 + 1);
    }
    const rowPages = rowEdges.length - 1;
    const colPages = colEdges.length - 1;
    if (rowPages < 1 || colPages < 1) return;
    ctx.save();
    ctx.setLineDash([]);
    ctx.globalAlpha = 0.14;
    this.setFill(theme.colors.textMuted);
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    this.setFont(`700 ${Math.round(44 * geo.zoom)}px ${theme.fonts.family}`);
    for (let ci = 0; ci < colPages; ci++) {
      const pc0 = colEdges[ci] ?? 0;
      const pc1 = (colEdges[ci + 1] ?? pc0 + 1) - 1;
      if (pc1 < pc0 || pc1 < vis.c0 || pc0 > vis.c1) continue;
      for (let ri = 0; ri < rowPages; ri++) {
        const prr0 = rowEdges[ri] ?? 0;
        const prr1 = (rowEdges[ri + 1] ?? prr0 + 1) - 1;
        if (prr1 < prr0 || prr1 < vis.r0 || prr0 > vis.r1) continue;
        const rect = geo.rangeRect({ r0: prr0, c0: pc0, r1: prr1, c1: pc1 }, false);
        if (rect.w <= 0 || rect.h <= 0) continue;
        ctx.fillText(`Page ${ci * rowPages + ri + 1}`, rect.x + rect.w / 2, rect.y + rect.h / 2);
      }
    }
    ctx.restore();
    this.curFill = "";
    this.curFont = "";
  }

  /** Draws one cell's text, overflowing into empty neighbours. Returns 1 if drawn. */
  private drawCellText(
    f: MainFrame,
    r: number,
    c: number,
    cell: CellView,
    top: number,
    h: number,
    getRowCell: (c: number) => CellView | undefined,
    vis: RangeDto,
    bounds: { cMin: number; cMax: number },
    styleArg?: StyleDto,
    /** Horizontal extent override (a merged area); no overflow then. */
    box?: { x: number; w: number },
  ): number {
    const { geo, theme, dpr } = f;
    const ctx = this.main;
    const cw = box ? box.w : geo.colW(c);
    if (cw <= 0) return 0;
    const style = styleArg ?? f.source.getStyle(drawStyleId(cell));
    const zoom = geo.zoom;
    const font = cellFont(style, theme, zoom);
    const pad = CELL_PADDING * zoom;
    // A conditional-format icon sits at the left; the text keeps clear of it.
    const iconSpace = cell.cfIcon ? cfIconSize(h, zoom) + 4 * zoom : 0;
    const avail = Math.max(0, cw - 2 * pad - iconSpace);
    const x = box ? box.x : geo.colX(c);
    const isLink = (cell.flags & CELL_FLAGS.link) !== 0;
    const align = effectiveAlign(cell, style);
    const chip = style?.display?.type === "chip" ? style.display : null;

    let text = cell.text;
    let width = this.measurer.measure(font, text);
    if (cell.kind === "number" && width > avail) {
      // Numbers never overflow: shrink General, otherwise ###.
      const sep = f.decimalSep ?? ".";
      const general = (style?.numberFormat ?? "General") === "General" || isGeneralNumberText(text, sep);
      const key = `${font}\u0000${text}\u0000${Math.floor(avail)}\u0000${sep}`;
      let fitted = this.fitCache.get(key);
      if (fitted === undefined) {
        fitted =
          general && cell.num !== null
            ? fitNumber(cell.num, avail, (s) => this.measurer.measure(font, s), text, sep)
            : "###";
        this.fitCache.set(key, fitted);
      }
      text = fitted;
      width = this.measurer.measure(font, text);
    }

    // Horizontal span the text may occupy.
    let spanL = x;
    let spanR = x + cw;
    const overflow = !box && cell.kind !== "number" && !chip && !iconSpace && width > avail && !(style?.wrap ?? false);
    if (overflow) {
      const need = width + 2 * pad;
      const maxC = Math.min(geo.cols.count - 1, bounds.cMax, vis.c1 + 1);
      const minC = Math.max(0, bounds.cMin, vis.c0 - 1);
      if (align === "left") {
        for (let n = c + 1; n <= maxC && spanR - x < need; n++) {
          if (!isEmptyCell(getRowCell(n))) break;
          spanR = geo.colRight(n);
        }
      } else if (align === "right") {
        for (let n = c - 1; n >= minC && x + cw - spanL < need; n--) {
          if (!isEmptyCell(getRowCell(n))) break;
          spanL = geo.colX(n);
        }
      } else {
        let canL = true;
        let canR = true;
        let nl = c - 1;
        let nr = c + 1;
        const half = (need - cw) / 2;
        while ((canL || canR) && (x - spanL < half || spanR - (x + cw) < half)) {
          if (canR && spanR - (x + cw) < half) {
            if (nr > maxC || !isEmptyCell(getRowCell(nr))) canR = false;
            else spanR = geo.colRight(nr++);
          } else if (canL && x - spanL < half) {
            if (nl < minC || !isEmptyCell(getRowCell(nl))) canL = false;
            else spanL = geo.colX(nl--);
          } else break;
        }
      }
    }

    // Skip cells whose span is entirely off-screen.
    if (spanR < geo.headerW || spanL > geo.width) return 0;

    const fpx = fontPx(style, theme, zoom);
    let tx: number;
    if (chip) {
      const chipPadX = 8 * zoom;
      const chipH = Math.min(h - 6 * zoom, fpx + 8 * zoom);
      const chipW = Math.min(cw - 2 * 4 * zoom, width + 2 * chipPadX);
      const cx = align === "right" ? x + cw - 4 * zoom - chipW : align === "center" ? x + (cw - chipW) / 2 : x + 4 * zoom;
      const cy = top + (h - chipH) / 2;
      ctx.save();
      ctx.beginPath();
      ctx.rect(x, top, cw, h);
      ctx.clip();
      this.setFill(cssColor(chip.bg));
      ctx.beginPath();
      roundRect(ctx, cx, cy, chipW, chipH, Math.min(theme.radii.pill, chipH / 2));
      ctx.fill();
      this.setFont(font);
      this.setFill(cssColor(chip.fg));
      ctx.textBaseline = "middle";
      ctx.fillText(text, cx + chipPadX, cy + chipH / 2 + 0.5 * zoom);
      ctx.restore();
      this.curFill = "";
      this.curFont = "";
      return 1;
    }

    if (align === "left") tx = x + pad + iconSpace;
    else if (align === "right") tx = x + cw - pad - width;
    else tx = x + iconSpace + (cw - iconSpace - width) / 2;

    // Vertical position (alphabetic baseline). In a default-height row all
    // three alignments centre the cap height; taller rows show the difference.
    // In a lower row bottom-aligned text comes down to a descent above the
    // bottom (as AOO draws it) or to centred caps, so its top is not cut.
    const vAlign = style?.vAlign ?? "bottom";
    const capH = CAP_HEIGHT * fpx;
    const rowRef = DEFAULT_ROW_REF * zoom;
    let ty: number;
    if (vAlign === "top") ty = top + (rowRef + capH) / 2;
    else if (vAlign === "middle") ty = top + (h + capH) / 2;
    else ty = top + h - Math.min(Math.max(2 * zoom, (rowRef - capH) / 2), Math.max(DESCENT * fpx, (h - capH) / 2));

    if (style?.wrap && cell.kind !== "number" && width > avail) {
      return this.drawWrapped(f, cell, style, font, fpx, x, top, cw, h, align, vAlign);
    }

    const clip = overflow || width > avail;
    if (overflow && (spanL < x || spanR > x + cw)) {
      // Cover the gridlines under the overflowing text.
      this.setFill(theme.colors.bg);
      const lw = Math.max(1, Math.round(dpr)) / dpr;
      if (spanL < x) this.main.fillRect(spanL, top, x - spanL, h - lw);
      if (spanR > x + cw) this.main.fillRect(x + cw - lw, top, spanR - (x + cw), h - lw);
    }
    if (clip) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(spanL, top, spanR - spanL, h);
      ctx.clip();
    }
    this.setFont(font);
    this.setFill(isLink && cell.color === null && (style?.color == null || (style.color & 255) === 0) ? theme.colors.link : textColor(cell, style, theme));
    ctx.textBaseline = "alphabetic";
    ctx.fillText(text, tx, ty);
    if (style?.underline || style?.strike || isLink) {
      const lh = Math.max(1, Math.round(fpx / 14));
      if (style?.underline || isLink) ctx.fillRect(tx, Math.round(ty + 2 * zoom), width, lh);
      if (style?.strike) ctx.fillRect(tx, Math.round(ty - capH * 0.45), width, lh);
    }
    if (clip) {
      ctx.restore();
      this.curFill = "";
      this.curFont = "";
    }
    return 1;
  }

  /** Word-wrapped text (style.wrap), clipped to the cell. */
  private drawWrapped(
    f: MainFrame,
    cell: CellView,
    style: StyleDto,
    font: string,
    fpx: number,
    x: number,
    top: number,
    cw: number,
    h: number,
    align: EffectiveAlign,
    vAlign: string,
  ): number {
    const ctx = this.main;
    const zoom = f.geo.zoom;
    const pad = CELL_PADDING * zoom;
    const avail = Math.max(1, cw - 2 * pad);
    const lines = wrapLines(cell.text, avail, (t) => this.measurer.measure(font, t));
    const lineH = Math.round(fpx * 1.3);
    const total = lines.length * lineH;
    const capH = CAP_HEIGHT * fpx;
    let y0: number;
    if (vAlign === "top") y0 = top + 3 * zoom;
    else if (vAlign === "middle") y0 = top + (h - total) / 2;
    else y0 = top + h - total - 3 * zoom;
    ctx.save();
    ctx.beginPath();
    ctx.rect(x, top, cw, h);
    ctx.clip();
    this.setFont(font);
    this.setFill(textColor(cell, style, f.theme));
    ctx.textBaseline = "alphabetic";
    lines.forEach((line, i) => {
      const w = this.measurer.measure(font, line);
      const tx = align === "right" ? x + cw - pad - w : align === "center" ? x + (cw - w) / 2 : x + pad;
      ctx.fillText(line, tx, y0 + i * lineH + (lineH + capH) / 2);
    });
    ctx.restore();
    this.curFill = "";
    this.curFont = "";
    return 1;
  }

  private drawHeaders(f: MainFrame): void {
    const ctx = this.main;
    const { geo, theme, dpr } = f;
    const c = theme.colors;
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    const font = headerFont(theme, geo.zoom, false);
    if (f.showHeaders === false) {
      // View ▸ Column and row headers off: only the outline gutters remain.
      if (f.outline && (geo.gutterW > 0 || geo.gutterH > 0)) {
        this.setFill(c.headerBg);
        if (geo.gutterH > 0) ctx.fillRect(0, 0, geo.width, geo.gutterH);
        if (geo.gutterW > 0) ctx.fillRect(0, 0, geo.gutterW, geo.height);
        this.curFill = "";
        this.drawOutlineGutter(f, f.outline);
      }
      return;
    }

    // Header strips.

    this.setFill(c.headerBg);
    ctx.fillRect(geo.headerW, 0, geo.width - geo.headerW, geo.headerH);
    ctx.fillRect(0, geo.headerH, geo.headerW, geo.height - geo.headerH);

    // Column letters, per column span (frozen columns, then the scrolling ones).
    for (const span of geo.colSpans()) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(span.x0, 0, span.x1 - span.x0, geo.headerH);
      ctx.clip();
      this.setFont(font);
      ctx.textBaseline = "middle";
      ctx.textAlign = "center";
      for (let col = span.c0; col <= span.c1; col++) {
        const x0 = geo.colX(col);
        const x1 = geo.colRight(col);
        if (x1 - x0 <= 0) continue;
        this.setFill(c.headerText);
        // Centre the label on the visible part (a column half under a frozen pane keeps its letter).
        const vx0 = Math.max(x0, span.x0);
        const vx1 = Math.min(x1, span.x1);
        if (vx1 - vx0 > 8) ctx.fillText(colToLetters(col), (vx0 + vx1) / 2, (geo.gutterH + geo.headerH) / 2 + 0.5);
        this.setFill(c.gridLine);
        ctx.fillRect(snapFill(x1, dpr) - lw, geo.gutterH, lw, geo.headerH - geo.gutterH);
        // Hidden columns before this one: a small marker on the boundary.
        if (col > 0 && geo.cols.isHidden(col - 1)) this.hiddenMarker(x0, (geo.gutterH + geo.headerH) / 2, true, geo, theme, dpr);
      }
      ctx.restore();
      this.curFill = "";
      this.curFont = "";
    }

    // Row numbers, per row span.
    for (const span of geo.rowSpans()) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(0, span.y0, geo.headerW, span.y1 - span.y0);
      ctx.clip();
      this.setFont(font);
      ctx.textBaseline = "middle";
      ctx.textAlign = "center";
      for (let row = span.r0; row <= span.r1; row++) {
        const y0 = geo.rowY(row);
        const y1 = geo.rowBottom(row);
        if (y1 - y0 <= 0) continue;
        this.setFill(c.headerText);
        const vy0 = Math.max(y0, span.y0);
        const vy1 = Math.min(y1, span.y1);
        if (vy1 - vy0 > 6) ctx.fillText(String(row + 1), (geo.gutterW + geo.headerW) / 2, (vy0 + vy1) / 2 + 0.5);
        this.setFill(c.gridLine);
        ctx.fillRect(geo.gutterW, snapFill(y1, dpr) - lw, geo.headerW - geo.gutterW, lw);
        if (row > 0 && geo.rows.isHidden(row - 1)) this.hiddenMarker((geo.gutterW + geo.headerW) / 2, y0, false, geo, theme, dpr);
      }
      ctx.restore();
      ctx.textAlign = "left";
      this.curFill = "";
      this.curFont = "";
    }

    // The select-all corner: header background and a small triangle.
    this.setFill(c.headerBg);
    ctx.fillRect(0, 0, geo.headerW, geo.headerH);
    this.setFill(c.headerLine);
    const t = Math.round(8 * geo.zoom);
    ctx.beginPath();
    ctx.moveTo(geo.headerW - 4, geo.headerH - 4 - t);
    ctx.lineTo(geo.headerW - 4, geo.headerH - 4);
    ctx.lineTo(geo.headerW - 4 - t, geo.headerH - 4);
    ctx.closePath();
    ctx.fill();

    // Strip edges and the corner.
    this.setFill(c.headerLine);
    ctx.fillRect(0, snapFill(geo.headerH, dpr) - lw, geo.width, lw);
    ctx.fillRect(snapFill(geo.headerW, dpr) - lw, 0, lw, geo.height);

    if (f.outline && (geo.gutterW > 0 || geo.gutterH > 0)) this.drawOutlineGutter(f, f.outline);
  }

  /**
   * Outline gutter stub (wave 5 seam): level buttons in the corner strips
   * and, per group, a line along the group with a +/− box at its first
   * row/column. W5-6B refines the look; the hit test is `outlineHit`.
   */
  private drawOutlineGutter(f: MainFrame, outline: OutlineProps): void {
    const ctx = this.main;
    const { geo, theme, dpr } = f;
    const c = theme.colors;
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    const levelPx = (outline.levelPx ?? 13) * geo.zoom;
    const box = Math.max(6, Math.round(levelPx - 4));
    const font = `${Math.max(8, Math.round(9 * geo.zoom))}px ${theme.fonts.family}`;
    ctx.save();
    this.setFont(font);
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    const button = (x: number, y: number, label: string, filled: boolean): void => {
      this.setFill(filled ? c.headerActiveBg : c.bg);
      ctx.fillRect(x, y, box, box);
      this.setFill(c.headerLine);
      ctx.fillRect(x, y, box, lw);
      ctx.fillRect(x, y + box - lw, box, lw);
      ctx.fillRect(x, y, lw, box);
      ctx.fillRect(x + box - lw, y, lw, box);
      this.setFill(c.headerText);
      ctx.fillText(label, x + box / 2, y + box / 2 + 0.5);
    };
    if (geo.gutterW > 0) {
      const levels = Math.round(geo.gutterW / levelPx);
      for (let l = 1; l <= levels; l++) button((l - 1) * levelPx + 2, geo.gutterH + 2, String(l), false);
      ctx.beginPath();
      ctx.rect(0, geo.headerH, geo.gutterW, geo.height - geo.headerH);
      ctx.clip();
      for (const g of outline.rows) {
        const x = (g.level - 1) * levelPx + 2;
        const y0 = geo.rowY(g.start);
        const y1 = g.collapsed ? y0 + box : geo.rowBottom(g.end);
        if (y1 < geo.headerH || y0 > geo.height) continue;
        this.setFill(c.headerLine);
        if (!g.collapsed) ctx.fillRect(x + box / 2, y0 + box, lw, Math.max(0, y1 - y0 - box));
        button(x, y0 + 1, g.collapsed ? "+" : "−", g.collapsed);
      }
    }
    if (geo.gutterH > 0) {
      const levels = Math.round(geo.gutterH / levelPx);
      for (let l = 1; l <= levels; l++) button(geo.gutterW + 2, (l - 1) * levelPx + 2, String(l), false);
      ctx.beginPath();
      ctx.rect(geo.headerW, 0, geo.width - geo.headerW, geo.gutterH);
      ctx.clip();
      for (const g of outline.cols) {
        const y = (g.level - 1) * levelPx + 2;
        const x0 = geo.colX(g.start);
        const x1 = g.collapsed ? x0 + box : geo.colRight(g.end);
        if (x1 < geo.headerW || x0 > geo.width) continue;
        this.setFill(c.headerLine);
        if (!g.collapsed) ctx.fillRect(x0 + box, y + box / 2, Math.max(0, x1 - x0 - box), lw);
        button(x0 + 1, y, g.collapsed ? "+" : "−", g.collapsed);
      }
    }
    ctx.restore();
    this.curFill = "";
    this.curFont = "";
  }

  /**
   * Detective arrows stub (wave 5 seam): a line from the centre of `from`
   * to the centre of `to` with an arrowhead; precedents in the accent
   * colour, dependents in the link colour, errors in red.
   */
  private drawTraces(f: OverlayFrame, traces: readonly TraceArrow[]): void {
    const ctx = this.overlay;
    const { geo } = f;
    const c = f.theme.colors;
    ctx.save();
    ctx.lineWidth = Math.max(1, 1.25 * geo.zoom);
    for (const t of traces) {
      const a = geo.cellRect(t.from.r, t.from.c);
      const b = geo.cellRect(t.to.r, t.to.c);
      const x0 = a.x + a.w / 2;
      const y0 = a.y + a.h / 2;
      const x1 = b.x + b.w / 2;
      const y1 = b.y + b.h / 2;
      const color = t.kind === "error" ? c.chipRedFg : t.kind === "dependent" ? c.link : c.accent;
      ctx.strokeStyle = color;
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.moveTo(x0, y0);
      ctx.lineTo(x1, y1);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(x0, y0, 2.5 * geo.zoom, 0, Math.PI * 2);
      ctx.fill();
      const ang = Math.atan2(y1 - y0, x1 - x0);
      const len = 7 * geo.zoom;
      ctx.beginPath();
      ctx.moveTo(x1, y1);
      ctx.lineTo(x1 - len * Math.cos(ang - 0.4), y1 - len * Math.sin(ang - 0.4));
      ctx.lineTo(x1 - len * Math.cos(ang + 0.4), y1 - len * Math.sin(ang + 0.4));
      ctx.closePath();
      ctx.fill();
    }
    ctx.restore();
  }

  /** Detective "Mark invalid data": a red ellipse around each listed cell. */
  private drawInvalidCells(f: OverlayFrame, cells: readonly CellPos[]): void {
    const ctx = this.overlay;
    const { geo } = f;
    const vis = geo.visibleRange();
    ctx.save();
    ctx.strokeStyle = f.theme.colors.chipRedFg;
    ctx.lineWidth = Math.max(1, 1.5 * geo.zoom);
    for (const cell of cells) {
      if (cell.r < vis.r0 - 1 || cell.r > vis.r1 + 1 || cell.c < vis.c0 - 1 || cell.c > vis.c1 + 1) continue;
      const r = geo.cellRect(cell.r, cell.c);
      if (r.w <= 0 || r.h <= 0) continue;
      ctx.beginPath();
      ctx.ellipse(r.x + r.w / 2, r.y + r.h / 2, Math.max(4, r.w / 2 - 1), Math.max(4, r.h / 2 - 1), 0, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.restore();
  }

  /** The live position of a page-break line while it is dragged. */
  private drawBreakDrag(f: OverlayFrame, drag: { axis: "row" | "col"; pos: number }): void {
    const ctx = this.overlay;
    const { geo } = f;
    ctx.save();
    ctx.strokeStyle = f.theme.colors.pageBreak;
    ctx.lineWidth = 2;
    ctx.setLineDash([7, 3]);
    ctx.beginPath();
    if (drag.axis === "row") {
      const y = Math.round(drag.pos) - 0.5;
      ctx.moveTo(geo.headerW, y);
      ctx.lineTo(geo.width, y);
    } else {
      const x = Math.round(drag.pos) - 0.5;
      ctx.moveTo(x, geo.headerH);
      ctx.lineTo(x, geo.height);
    }
    ctx.stroke();
    ctx.restore();
  }

  /**
   * Marks hidden rows/columns in the header: a short double tick across
   * the boundary where they were (centred at `x`, `y`).
   */
  private hiddenMarker(x: number, y: number, vertical: boolean, geo: Geometry, theme: Theme, dpr: number): void {
    const ctx = this.main;
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    const len = Math.round((vertical ? geo.headerH : geo.headerW) * 0.36);
    this.setFill(theme.colors.accent);
    ctx.globalAlpha = 0.7;
    if (vertical) {
      const sx = snapFill(x, dpr);
      ctx.fillRect(sx - 2 * lw, y - len / 2, lw, len);
      ctx.fillRect(sx + lw, y - len / 2, lw, len);
    } else {
      const sy = snapFill(y, dpr);
      ctx.fillRect(x - len / 2, sy - 2 * lw, len, lw);
      ctx.fillRect(x - len / 2, sy + lw, len, lw);
    }
    ctx.globalAlpha = 1;
  }

  // -------------------------------------------------------------------------
  // Overlay layer
  // -------------------------------------------------------------------------

  /** The ▾ of the active list cell: a caret on a rounded button. */
  private drawDropdownActive(b: Rect, zoom: number, theme: Theme): void {
    const ctx = this.overlay;
    ctx.save();
    ctx.beginPath();
    roundRect(ctx, b.x + 0.5, b.y + 0.5, b.w - 1, b.h - 1, 3 * zoom);
    ctx.fillStyle = theme.colors.bg;
    ctx.fill();
    ctx.lineWidth = 1;
    ctx.strokeStyle = theme.colors.headerLine;
    ctx.stroke();
    const cx = b.x + b.w / 2;
    const cy = b.y + b.h / 2 + 0.5 * zoom;
    const s = Math.max(2.5, 3 * zoom);
    ctx.beginPath();
    ctx.moveTo(cx - s, cy - s / 2);
    ctx.lineTo(cx, cy + s / 2);
    ctx.lineTo(cx + s, cy - s / 2);
    ctx.lineWidth = Math.max(1.3, 1.5 * zoom);
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.strokeStyle = theme.colors.text;
    ctx.stroke();
    ctx.restore();
  }

  /** The ↗ of the active link cell. */
  private drawLinkButton(b: Rect, zoom: number, theme: Theme): void {
    const ctx = this.overlay;
    ctx.save();
    ctx.beginPath();
    roundRect(ctx, b.x + 0.5, b.y + 0.5, b.w - 1, b.h - 1, 3 * zoom);
    ctx.fillStyle = theme.colors.accent;
    ctx.fill();
    // ↗: a diagonal with an arrow head.
    const p = b.w * 0.3;
    ctx.strokeStyle = "#ffffff";
    ctx.lineWidth = Math.max(1.3, 1.5 * zoom);
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.beginPath();
    ctx.moveTo(b.x + p, b.y + b.h - p);
    ctx.lineTo(b.x + b.w - p, b.y + p);
    ctx.moveTo(b.x + b.w - p - b.w * 0.28, b.y + p);
    ctx.lineTo(b.x + b.w - p, b.y + p);
    ctx.lineTo(b.x + b.w - p, b.y + p + b.h * 0.28);
    ctx.stroke();
    ctx.restore();
  }

  /** A padlock badge (a refused edit on a protected cell). */
  private drawLockBadge(right: number, top: number, zoom: number, theme: Theme): void {
    const ctx = this.overlay;
    const s = 18 * zoom;
    const x = right - s - 2 * zoom;
    const y = top + 2 * zoom;
    ctx.save();
    ctx.beginPath();
    roundRect(ctx, x, y, s, s, 4 * zoom);
    ctx.fillStyle = theme.colors.noteMarker;
    ctx.fill();
    // Padlock: shackle and body.
    ctx.strokeStyle = "#ffffff";
    ctx.fillStyle = "#ffffff";
    ctx.lineWidth = Math.max(1.2, 1.4 * zoom);
    ctx.beginPath();
    ctx.arc(x + s / 2, y + s * 0.42, s * 0.18, Math.PI, 0);
    ctx.stroke();
    ctx.fillRect(x + s * 0.27, y + s * 0.44, s * 0.46, s * 0.34);
    ctx.restore();
  }

  drawOverlay(f: OverlayFrame): void {
    const ctx = this.overlay;
    const { geo, theme, dpr, selection } = f;
    const c = theme.colors;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, geo.width, geo.height);
    const range = selection.range;
    // While a chart or picture is selected the cell cursor is not shown (as in Calc).
    const hideSel = f.objectSelected === true || f.hideSelection === true;
    const others = hideSel ? [] : (selection.others ?? []);
    const ranges = hideSel ? [] : [...others, range];
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    const rect = geo.rangeRect(range);

    // Header highlight (redraws the labels over the main layer).
    const font = headerFont(theme, geo.zoom, true);
    ctx.font = font;
    ctx.textBaseline = "middle";
    ctx.textAlign = "center";
    ctx.save();
    ctx.beginPath();
    ctx.rect(geo.headerW, 0, geo.cellsW, geo.headerH);
    ctx.clip();
    for (const span of geo.colSpans()) {
      for (const rg of ranges) {
        for (let col = Math.max(span.c0, rg.c0); col <= Math.min(span.c1, rg.c1); col++) {
          const x0 = geo.colX(col);
          const w = geo.colW(col);
          if (w <= 0) continue;
          ctx.fillStyle = c.headerActiveBg;
          ctx.fillRect(x0, 0, w - lw, geo.headerH - lw);
          ctx.fillStyle = c.headerActiveText;
          ctx.fillText(colToLetters(col), x0 + w / 2, geo.headerH / 2 + 0.5);
        }
      }
    }
    if (!hideSel && rect.w > 0) {
      ctx.fillStyle = c.selectionBorder;
      ctx.fillRect(rect.x, geo.headerH - 2, rect.w, 2);
    }
    ctx.restore();
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, geo.headerH, geo.headerW, geo.cellsH);
    ctx.clip();
    for (const span of geo.rowSpans()) {
      for (const rg of ranges) {
        for (let row = Math.max(span.r0, rg.r0); row <= Math.min(span.r1, rg.r1); row++) {
          const y0 = geo.rowY(row);
          const h = geo.rowH(row);
          if (h <= 0) continue;
          ctx.fillStyle = c.headerActiveBg;
          ctx.fillRect(0, y0, geo.headerW - lw, h - lw);
          ctx.fillStyle = c.headerActiveText;
          ctx.fillText(String(row + 1), geo.headerW / 2, y0 + h / 2 + 0.5);
        }
      }
    }
    if (!hideSel && rect.h > 0) {
      ctx.fillStyle = c.selectionBorder;
      ctx.fillRect(geo.headerW - 2, rect.y, 2, rect.h);
    }
    ctx.restore();
    ctx.textAlign = "left";

    // Hovered resize gutter.
    if (f.hoverGutter !== null && f.resizeGuide === null) {
      if (f.hoverGutter.axis === "col") {
        const gx = geo.colRight(f.hoverGutter.index);
        if (gx >= geo.headerW && gx <= geo.width) {
          ctx.fillStyle = c.resizeGuide;
          ctx.fillRect(snapFill(gx, dpr) - 1.5, 3, 2, geo.headerH - 6);
        }
      } else {
        const gy = geo.rowBottom(f.hoverGutter.index);
        if (gy >= geo.headerH && gy <= geo.height) {
          ctx.fillStyle = c.resizeGuide;
          ctx.fillRect(3, snapFill(gy, dpr) - 1.5, geo.headerW - 6, 2);
        }
      }
    }

    // Cell area.
    ctx.save();
    ctx.beginPath();
    ctx.rect(geo.headerW, geo.headerH, geo.cellsW, geo.cellsH);
    ctx.clip();
    if (f.traces && f.traces.length > 0) this.drawTraces(f, f.traces);
    if (f.invalidCells && f.invalidCells.length > 0) this.drawInvalidCells(f, f.invalidCells);
    if (f.breakDrag) this.drawBreakDrag(f, f.breakDrag);
    const box = (rg: RangeDto): { x0: number; y0: number; x1: number; y1: number } | null => {
      const r = geo.rangeRect(rg);
      if (r.w <= 0 || r.h <= 0) return null;
      return { x0: snapFill(r.x, dpr) - lw, y0: snapFill(r.y, dpr) - lw, x1: snapFill(r.x + r.w, dpr), y1: snapFill(r.y + r.h, dpr) };
    };
    const frame = (b: { x0: number; y0: number; x1: number; y1: number }, bw: number): void => {
      ctx.fillRect(b.x0, b.y0, b.x1 - b.x0, bw);
      ctx.fillRect(b.x0, b.y1 - bw, b.x1 - b.x0, bw);
      ctx.fillRect(b.x0, b.y0, bw, b.y1 - b.y0);
      ctx.fillRect(b.x1 - bw, b.y0, bw, b.y1 - b.y0);
    };

    // The array-formula block around the active cell: a thin outline.
    if (f.arrayRange) {
      const b = box(f.arrayRange);
      if (b) {
        ctx.fillStyle = c.accent;
        ctx.globalAlpha = 0.85;
        frame({ x0: b.x0 - 1, y0: b.y0 - 1, x1: b.x1 + 1, y1: b.y1 + 1 }, 1);
        ctx.globalAlpha = 1;
      }
    }

    // Earlier ranges of a multi-selection: tint and a thin border.
    for (const rg of others) {
      const r = geo.rangeRect(rg);
      if (r.w <= 0 || r.h <= 0) continue;
      ctx.fillStyle = c.selection;
      ctx.fillRect(r.x, r.y, r.w, r.h);
      const b = box(rg);
      if (b) {
        ctx.fillStyle = c.selectionBorder;
        frame(b, 1);
      }
    }
    if (!hideSel && rect.w > 0 && rect.h > 0) {
      const activeMerge = geo.mergeAt(selection.active.r, selection.active.c);
      const single =
        (range.r0 === range.r1 && range.c0 === range.c1) ||
        (activeMerge !== undefined && activeMerge.r0 === range.r0 && activeMerge.c0 === range.c0 && activeMerge.r1 === range.r1 && activeMerge.c1 === range.c1);
      if (!single || others.length > 0) {
        ctx.fillStyle = c.selection;
        if (!single) ctx.fillRect(rect.x, rect.y, rect.w, rect.h);
        // The active cell (or its merged area) stays clear.
        const a = geo.rangeRect(activeMerge ?? { r0: selection.active.r, c0: selection.active.c, r1: selection.active.r, c1: selection.active.c });
        if (!single && a.w > 0 && a.h > 0) ctx.clearRect(a.x, a.y, a.w - lw, a.h - lw);
      }
      // 2 px border, drawn inside the range's top-left gridlines.
      const b = box(range);
      if (b) {
        ctx.fillStyle = c.selectionBorder;
        frame(b, 2);
        // Fill handle at the bottom-right corner (only if that corner is on screen).
        const cornerVisible = geo.colRight(range.c1) <= geo.width + 4 && geo.rowBottom(range.r1) <= geo.height + 4;
        if (!f.editing && cornerVisible && others.length === 0) {
          const s = 7;
          const hx = b.x1 - 1 - s / 2;
          const hy = b.y1 - 1 - s / 2;
          ctx.fillStyle = c.bg;
          ctx.fillRect(hx - 1, hy - 1, s + 2, s + 2);
          ctx.fillStyle = c.fillHandle;
          ctx.fillRect(hx, hy, s, s);
        }
      }
    }

    const activeCell = f.source.getCell(selection.active.r, selection.active.c);
    // The active list cell's ▾ as a button.
    if (!hideSel && !f.editing && activeCell && activeCell.flags & CELL_FLAGS.dropdown) {
      const db = dropdownButtonRect(geo, selection.active.r, selection.active.c);
      if (db.w > 4) this.drawDropdownActive(db, geo.zoom, theme);
    }

    // The active cell's link ↗ (a click on it opens the link).
    if (!f.editing && activeCell && activeCell.flags & CELL_FLAGS.link && activeCell.link) {
      const lb = linkButtonRect(geo, selection.active.r, selection.active.c, (activeCell.flags & CELL_FLAGS.dropdown) !== 0);
      if (lb.w > 4) this.drawLinkButton(lb, geo.zoom, theme);
    }

    // A refused edit on a protected cell: a lock badge at the cell's top-right.
    if (f.lockedCell) {
      const r = geo.rangeRect({ r0: f.lockedCell.r, c0: f.lockedCell.c, r1: f.lockedCell.r, c1: f.lockedCell.c });
      if (r.w > 0 && r.h > 0) this.drawLockBadge(r.x + r.w, r.y, geo.zoom, theme);
    }

    // Copied / cut range: a dashed ("marching ants") outline.
    if (f.clipRange) {
      const b = box(f.clipRange);
      if (b) {
        ctx.save();
        ctx.strokeStyle = c.selectionBorder;
        ctx.lineWidth = 1.5;
        ctx.setLineDash([4, 3]);
        ctx.strokeRect(b.x0 + 0.75, b.y0 + 0.75, b.x1 - b.x0 - 1.5, b.y1 - b.y0 - 1.5);
        ctx.restore();
      }
    }

    // Fill-handle drag preview: a dashed outline of the target.
    if (f.fillPreview) {
      const b = box(f.fillPreview);
      if (b) {
        ctx.save();
        ctx.strokeStyle = c.selectionBorder;
        ctx.lineWidth = 1.5;
        ctx.setLineDash([5, 3]);
        ctx.strokeRect(b.x0 + 0.75, b.y0 + 0.75, b.x1 - b.x0 - 1.5, b.y1 - b.y0 - 1.5);
        ctx.restore();
      }
    }

    // Formula reference outlines (each reference in its own colour).
    for (const h of f.refHighlights ?? []) {
      const r = geo.rangeRect(h.range);
      if (r.w <= 0 || r.h <= 0) continue;
      ctx.save();
      ctx.globalAlpha = 0.1;
      ctx.fillStyle = h.color;
      ctx.fillRect(r.x, r.y, r.w, r.h);
      ctx.restore();
      const b = box(h.range);
      if (b) {
        ctx.fillStyle = h.color;
        frame(b, 2);
        // Small corner squares, like the fill handle, mark it as a reference.
        const s = 5;
        for (const [x, y] of [
          [b.x0, b.y0],
          [b.x1, b.y0],
          [b.x0, b.y1],
          [b.x1, b.y1],
        ] as const) {
          ctx.fillRect(x - s / 2 + (x === b.x0 ? 1 : -1), y - s / 2 + (y === b.y0 ? 1 : -1), s, s);
        }
      }
    }
    ctx.restore();

    // Live resize guide.
    if (f.resizeGuide) {
      ctx.fillStyle = c.resizeGuide;
      if (f.resizeGuide.axis === "col") ctx.fillRect(snapFill(f.resizeGuide.pos, dpr) - 1, 0, 2, geo.height);
      else ctx.fillRect(0, snapFill(f.resizeGuide.pos, dpr) - 1, geo.width, 2);
    }
  }
}

/** Relative luminance (0..1) of a `0xRRGGBBAA` colour. */
export function luminance(rgba: number): number {
  const n = rgba >>> 0;
  const ch = (v: number): number => {
    const x = v / 255;
    return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * ch((n >>> 24) & 255) + 0.7152 * ch((n >>> 16) & 255) + 0.0722 * ch((n >>> 8) & 255);
}

/**
 * The text colour of a cell: the format's colour ([RED]), else the style's,
 * else the theme text, except on a filled cell, where theme text that would
 * be illegible (dark theme text on a light fill, or the reverse) flips to a
 * contrasting dark or light grey.
 */
export function textColor(cell: CellView, style: StyleDto | undefined, theme: Theme): string {
  if (cell.color !== null) return cssColor(cell.color);
  // An alpha-0 colour is the "Automatic" sentinel (StylePatch cannot reset a colour to none).
  if (style?.color != null && (style.color & 255) > 0) return cssColor(style.color);
  const fill = style?.fill;
  if (fill != null && (fill & 255) > 0) return luminance(fill) > 0.4 ? "#111827" : "#f3f4f6";
  return theme.colors.text;
}

/** Greedy word wrap of `text` into lines no wider than `max` (long words are split). */
export function wrapLines(text: string, max: number, measure: (s: string) => number): string[] {
  const out: string[] = [];
  for (const para of text.split("\n")) {
    const words = para.split(/(\s+)/).filter((w) => w !== "");
    let line = "";
    for (const w of words) {
      const next = line + w;
      if (line === "" || measure(next.trimEnd()) <= max) {
        line = line === "" ? next.trimStart() : next;
      } else {
        out.push(line.trimEnd());
        line = /^\s+$/.test(w) ? "" : w;
      }
      // Split a single word that is wider than the cell.
      while (line.trimEnd().length > 1 && measure(line.trimEnd()) > max) {
        let k = line.length - 1;
        while (k > 1 && measure(line.slice(0, k)) > max) k--;
        out.push(line.slice(0, k));
        line = line.slice(k);
      }
    }
    out.push(line.trimEnd());
  }
  return out;
}

export function headerFont(theme: Theme, zoom: number, active: boolean): string {
  const px = Math.round(theme.fonts.headerSizePx * zoom * 100) / 100;
  return `${active ? "600" : "500"} ${px}px ${theme.fonts.family}`;
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  const rr = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.moveTo(x + rr, y);
  ctx.arcTo(x + w, y, x + w, y + h, rr);
  ctx.arcTo(x + w, y + h, x, y + h, rr);
  ctx.arcTo(x, y + h, x, y, rr);
  ctx.arcTo(x, y, x + w, y, rr);
  ctx.closePath();
}

function now(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}
