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
import type { CellView, RangeDto, StyleDto } from "@tally/bridge";
import type { Theme } from "@tally/tokens";
import { colToLetters } from "./colLetters";
import { fitNumber, isGeneralNumberText } from "./fitNumber";
import { snapFill, type Geometry, type Rect } from "./geometry";
import { Lru } from "./lru";
import type { GridDataSource, RefHighlight, Selection } from "./types";

/** Horizontal text padding inside a cell at zoom 100%. */
export const CELL_PADDING = 6;
/** How far (in columns) to look outside the viewport for text overflowing into it. */
export const OVERFLOW_SCAN = 32;

const MEASURE_CACHE = 5000;
/** Inter's cap height as a fraction of the font size. */
const CAP_HEIGHT = 0.727;
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

/** The `ctx.font` string for a cell. */
export function cellFont(style: StyleDto | undefined, theme: Theme, zoom: number): string {
  const px = Math.round(fontPx(style, theme, zoom) * 100) / 100;
  const family = style?.fontFamily ? `"${style.fontFamily}", ${theme.fonts.family}` : theme.fonts.family;
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

function isEmptyCell(cell: CellView | undefined): boolean {
  return cell === undefined || cell.kind === "empty" || cell.text === "";
}

export interface MainFrame {
  geo: Geometry;
  source: GridDataSource;
  theme: Theme;
  dpr: number;
}

export interface OverlayFrame extends MainFrame {
  selection: Selection;
  /** Hide the fill handle and active-cell clearing while editing. */
  editing: boolean;
  /** Header gutter under the pointer. */
  hoverGutter: { axis: "col" | "row"; index: number } | null;
  /** Live resize: the axis, index and its new far edge (screen px). */
  resizeGuide: { axis: "col" | "row"; index: number; pos: number } | null;
  /** Fill-handle drag preview: the target range. */
  fillPreview?: RangeDto | null;
  /** Coloured reference outlines while editing a formula. */
  refHighlights?: readonly RefHighlight[];
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

    const vis = geo.visibleRange();
    const nr = vis.r1 - vis.r0 + 1;
    const nc = vis.c1 - vis.c0 + 1;
    // Column/row edges for the visible span.
    const xs = new Float64Array(nc + 1);
    const ys = new Float64Array(nr + 1);
    for (let i = 0; i <= nc; i++) xs[i] = geo.colX(vis.c0 + i);
    for (let i = 0; i <= nr; i++) ys[i] = geo.rowY(vis.r0 + i);

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
        styles[i * nc + j] = cell.style === 0 ? f.source.getStyle(0) : f.source.getStyle(cell.style);
        count++;
      }
    }

    ctx.save();
    ctx.beginPath();
    ctx.rect(geo.headerW, geo.headerH, geo.cellsW, geo.cellsH);
    ctx.clip();

    // 1. Fills.
    for (let i = 0; i < nr; i++) {
      for (let j = 0; j < nc; j++) {
        const st = styles[i * nc + j];
        if (!st || st.fill === null || (st.fill & 255) === 0) continue;
        this.setFill(cssColor(st.fill));
        const x0 = snapFill(xs[j] ?? 0, dpr);
        const y0 = snapFill(ys[i] ?? 0, dpr);
        ctx.fillRect(x0, y0, snapFill(xs[j + 1] ?? 0, dpr) - x0, snapFill(ys[i + 1] ?? 0, dpr) - y0);
      }
    }

    // 2. Gridlines: one device-aligned 1 CSS px line on the right/bottom edge of each cell.
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    this.setFill(theme.colors.gridLine);
    const bottom = Math.min(geo.height, ys[nr] ?? geo.height);
    const right = Math.min(geo.width, xs[nc] ?? geo.width);
    for (let j = 1; j <= nc; j++) {
      const x = xs[j] ?? 0;
      if (x === xs[j - 1]) continue;
      ctx.fillRect(snapFill(x, dpr) - lw, geo.headerH, lw, bottom - geo.headerH);
    }
    for (let i = 1; i <= nr; i++) {
      const y = ys[i] ?? 0;
      if (y === ys[i - 1]) continue;
      ctx.fillRect(geo.headerW, snapFill(y, dpr) - lw, right - geo.headerW, lw);
    }

    // 3. Text (with overflow), per row.
    let textDraws = 0;
    for (let i = 0; i < nr; i++) {
      const top = ys[i] ?? 0;
      const h = (ys[i + 1] ?? 0) - top;
      if (h <= 0) continue;
      const r = vis.r0 + i;
      const getRowCell = (c: number): CellView | undefined =>
        c >= vis.c0 && c <= vis.c1 ? cells[i * nc + (c - vis.c0)] : f.source.getCell(r, c);
      // Cells left of the viewport whose text may overflow into it.
      for (let c = vis.c0 - 1; c >= Math.max(0, vis.c0 - OVERFLOW_SCAN); c--) {
        const cell = f.source.getCell(r, c);
        if (isEmptyCell(cell)) continue;
        if (cell) textDraws += this.drawCellText(f, r, c, cell, top, h, getRowCell, vis);
        break;
      }
      for (let j = 0; j < nc; j++) {
        const cell = cells[i * nc + j];
        if (isEmptyCell(cell) || !cell) continue;
        textDraws += this.drawCellText(f, r, vis.c0 + j, cell, top, h, getRowCell, vis, styles[i * nc + j]);
      }
      // Cells right of the viewport (right-aligned or centred overflow).
      const maxC = geo.cols.count - 1;
      for (let c = vis.c1 + 1; c <= Math.min(maxC, vis.c1 + OVERFLOW_SCAN); c++) {
        const cell = f.source.getCell(r, c);
        if (isEmptyCell(cell)) continue;
        if (cell) textDraws += this.drawCellText(f, r, c, cell, top, h, getRowCell, vis);
        break;
      }
    }

    // 4. Borders.
    for (let i = 0; i < nr; i++) {
      for (let j = 0; j < nc; j++) {
        const st = styles[i * nc + j];
        if (!st) continue;
        const b = st.borders;
        if (!b.top && !b.bottom && !b.left && !b.right) continue;
        const x0 = xs[j] ?? 0;
        const x1 = xs[j + 1] ?? 0;
        const y0 = ys[i] ?? 0;
        const y1 = ys[i + 1] ?? 0;
        const edge = (line: typeof b.top, rect: Rect): void => {
          if (!line) return;
          this.setFill(cssColor(line.color));
          ctx.fillRect(snapFill(rect.x, dpr), snapFill(rect.y, dpr), Math.max(lw, snapFill(rect.w, dpr)), Math.max(lw, snapFill(rect.h, dpr)));
        };
        const w = (line: typeof b.top): number => Math.max(1, line?.widthPx ?? 1);
        edge(b.top, { x: x0 - lw, y: y0 - lw, w: x1 - x0 + lw, h: w(b.top) });
        edge(b.bottom, { x: x0 - lw, y: y1 - w(b.bottom), w: x1 - x0 + lw, h: w(b.bottom) });
        edge(b.left, { x: x0 - lw, y: y0 - lw, w: w(b.left), h: y1 - y0 + lw });
        edge(b.right, { x: x1 - w(b.right), y: y0 - lw, w: w(b.right), h: y1 - y0 + lw });
      }
    }
    ctx.restore();
    this.curFill = "";
    this.curFont = "";

    this.drawHeaders(f, vis, xs, ys);
    const stats = { cells: count, textDraws, ms: now() - t0 };
    this.lastStats = stats;
    return stats;
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
    styleArg?: StyleDto,
  ): number {
    const { geo, theme, dpr } = f;
    const ctx = this.main;
    const cw = geo.colW(c);
    if (cw <= 0) return 0;
    const style = styleArg ?? f.source.getStyle(cell.style);
    const zoom = geo.zoom;
    const font = cellFont(style, theme, zoom);
    const pad = CELL_PADDING * zoom;
    const avail = Math.max(0, cw - 2 * pad);
    const x = geo.colX(c);
    const align = effectiveAlign(cell, style);
    const chip = style?.display?.type === "chip" ? style.display : null;

    let text = cell.text;
    let width = this.measurer.measure(font, text);
    if (cell.kind === "number" && width > avail) {
      // Numbers never overflow: shrink General, otherwise ###.
      const general = (style?.numberFormat ?? "General") === "General" || isGeneralNumberText(text);
      const key = `${font}\u0000${text}\u0000${Math.floor(avail)}`;
      let fitted = this.fitCache.get(key);
      if (fitted === undefined) {
        fitted =
          general && cell.num !== null
            ? fitNumber(cell.num, avail, (s) => this.measurer.measure(font, s), text)
            : "###";
        this.fitCache.set(key, fitted);
      }
      text = fitted;
      width = this.measurer.measure(font, text);
    }

    // Horizontal span the text may occupy.
    let spanL = x;
    let spanR = x + cw;
    const overflow = cell.kind !== "number" && !chip && width > avail && !(style?.wrap ?? false);
    if (overflow) {
      const need = width + 2 * pad;
      const maxC = geo.cols.count - 1;
      if (align === "left") {
        for (let n = c + 1; n <= maxC && spanR - x < need && n <= vis.c1 + 1; n++) {
          if (!isEmptyCell(getRowCell(n))) break;
          spanR = geo.colX(n + 1);
        }
      } else if (align === "right") {
        for (let n = c - 1; n >= 0 && x + cw - spanL < need && n >= vis.c0 - 1; n--) {
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
            if (nr > maxC || nr > vis.c1 + 1 || !isEmptyCell(getRowCell(nr))) canR = false;
            else spanR = geo.colX(++nr);
          } else if (canL && x - spanL < half) {
            if (nl < 0 || nl < vis.c0 - 1 || !isEmptyCell(getRowCell(nl))) canL = false;
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

    if (align === "left") tx = x + pad;
    else if (align === "right") tx = x + cw - pad - width;
    else tx = x + (cw - width) / 2;

    // Vertical position (alphabetic baseline). In a default-height row all
    // three alignments centre the cap height; taller rows show the difference.
    const vAlign = style?.vAlign ?? "bottom";
    const capH = CAP_HEIGHT * fpx;
    const rowRef = DEFAULT_ROW_REF * zoom;
    let ty: number;
    if (vAlign === "top") ty = top + (rowRef + capH) / 2;
    else if (vAlign === "middle") ty = top + (h + capH) / 2;
    else ty = top + h - Math.max(2 * zoom, (rowRef - capH) / 2);

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
    this.setFill(textColor(cell, style, theme));
    ctx.textBaseline = "alphabetic";
    ctx.fillText(text, tx, ty);
    if (style?.underline || style?.strike) {
      const lh = Math.max(1, Math.round(fpx / 14));
      if (style.underline) ctx.fillRect(tx, Math.round(ty + 2 * zoom), width, lh);
      if (style.strike) ctx.fillRect(tx, Math.round(ty - capH * 0.45), width, lh);
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

  private drawHeaders(f: MainFrame, vis: RangeDto, xs: Float64Array, ys: Float64Array): void {
    const ctx = this.main;
    const { geo, theme, dpr } = f;
    const c = theme.colors;
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    const font = headerFont(theme, geo.zoom, false);
    const nc = vis.c1 - vis.c0 + 1;
    const nr = vis.r1 - vis.r0 + 1;

    // Column header strip.
    this.setFill(c.headerBg);
    ctx.fillRect(geo.headerW, 0, geo.width - geo.headerW, geo.headerH);
    ctx.fillRect(0, geo.headerH, geo.headerW, geo.height - geo.headerH);

    ctx.save();
    ctx.beginPath();
    ctx.rect(geo.headerW, 0, geo.cellsW, geo.headerH);
    ctx.clip();
    this.setFont(font);
    ctx.textBaseline = "middle";
    ctx.textAlign = "center";
    for (let j = 0; j < nc; j++) {
      const x0 = xs[j] ?? 0;
      const x1 = xs[j + 1] ?? 0;
      if (x1 - x0 <= 0) continue;
      this.setFill(c.headerText);
      ctx.fillText(colToLetters(vis.c0 + j), (x0 + x1) / 2, geo.headerH / 2 + 0.5);
      this.setFill(c.gridLine);
      ctx.fillRect(snapFill(x1, dpr) - lw, 0, lw, geo.headerH);
    }
    ctx.restore();
    this.curFill = "";
    this.curFont = "";

    // Row header strip.
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, geo.headerH, geo.headerW, geo.cellsH);
    ctx.clip();
    this.setFont(font);
    ctx.textBaseline = "middle";
    ctx.textAlign = "center";
    for (let i = 0; i < nr; i++) {
      const y0 = ys[i] ?? 0;
      const y1 = ys[i + 1] ?? 0;
      if (y1 - y0 <= 0) continue;
      this.setFill(c.headerText);
      ctx.fillText(String(vis.r0 + i + 1), geo.headerW / 2, (y0 + y1) / 2 + 0.5);
      this.setFill(c.gridLine);
      ctx.fillRect(0, snapFill(y1, dpr) - lw, geo.headerW, lw);
    }
    ctx.restore();
    ctx.textAlign = "left";
    this.curFill = "";
    this.curFont = "";

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
  }

  // -------------------------------------------------------------------------
  // Overlay layer
  // -------------------------------------------------------------------------

  drawOverlay(f: OverlayFrame): void {
    const ctx = this.overlay;
    const { geo, theme, dpr, selection } = f;
    const c = theme.colors;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, geo.width, geo.height);
    const range = selection.range;
    const others = selection.others ?? [];
    const ranges = [...others, range];
    const vis = geo.visibleRange();
    const lw = Math.max(1, Math.round(dpr)) / dpr;
    const rect = geo.rangeRect(range);

    // Header highlight (redraws the labels over the main layer).
    const font = headerFont(theme, geo.zoom, true);
    ctx.font = font;
    ctx.textBaseline = "middle";
    ctx.textAlign = "center";
    const colsOn = new Set<number>();
    const rowsOn = new Set<number>();
    for (const rg of ranges) {
      for (let col = Math.max(vis.c0, rg.c0); col <= Math.min(vis.c1, rg.c1); col++) colsOn.add(col);
      for (let row = Math.max(vis.r0, rg.r0); row <= Math.min(vis.r1, rg.r1); row++) rowsOn.add(row);
    }
    ctx.save();
    ctx.beginPath();
    ctx.rect(geo.headerW, 0, geo.cellsW, geo.headerH);
    ctx.clip();
    for (const col of colsOn) {
      const x0 = geo.colX(col);
      const w = geo.colW(col);
      if (w <= 0) continue;
      ctx.fillStyle = c.headerActiveBg;
      ctx.fillRect(x0, 0, w - lw, geo.headerH - lw);
      ctx.fillStyle = c.headerActiveText;
      ctx.fillText(colToLetters(col), x0 + w / 2, geo.headerH / 2 + 0.5);
    }
    if (rect.w > 0) {
      ctx.fillStyle = c.selectionBorder;
      ctx.fillRect(rect.x, geo.headerH - 2, rect.w, 2);
    }
    ctx.restore();
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, geo.headerH, geo.headerW, geo.cellsH);
    ctx.clip();
    for (const row of rowsOn) {
      const y0 = geo.rowY(row);
      const h = geo.rowH(row);
      if (h <= 0) continue;
      ctx.fillStyle = c.headerActiveBg;
      ctx.fillRect(0, y0, geo.headerW - lw, h - lw);
      ctx.fillStyle = c.headerActiveText;
      ctx.fillText(String(row + 1), geo.headerW / 2, y0 + h / 2 + 0.5);
    }
    if (rect.h > 0) {
      ctx.fillStyle = c.selectionBorder;
      ctx.fillRect(geo.headerW - 2, rect.y, 2, rect.h);
    }
    ctx.restore();
    ctx.textAlign = "left";

    // Hovered resize gutter.
    if (f.hoverGutter !== null && f.resizeGuide === null) {
      if (f.hoverGutter.axis === "col") {
        const gx = geo.colX(f.hoverGutter.index + 1);
        if (gx >= geo.headerW && gx <= geo.width) {
          ctx.fillStyle = c.resizeGuide;
          ctx.fillRect(snapFill(gx, dpr) - 1.5, 3, 2, geo.headerH - 6);
        }
      } else {
        const gy = geo.rowY(f.hoverGutter.index + 1);
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
    if (rect.w > 0 && rect.h > 0) {
      const single = range.r0 === range.r1 && range.c0 === range.c1;
      if (!single || others.length > 0) {
        ctx.fillStyle = c.selection;
        if (!single) ctx.fillRect(rect.x, rect.y, rect.w, rect.h);
        // The active cell stays clear.
        const a = geo.cellRect(selection.active.r, selection.active.c);
        if (!single) ctx.clearRect(a.x, a.y, a.w - lw, a.h - lw);
      }
      // 2 px border, drawn inside the range's top-left gridlines.
      const b = box(range);
      if (b) {
        ctx.fillStyle = c.selectionBorder;
        frame(b, 2);
        // Fill handle at the bottom-right corner (only if that corner is on screen).
        const cornerVisible = geo.colX(range.c1 + 1) <= geo.width + 4 && geo.rowY(range.r1 + 1) <= geo.height + 4;
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
  if (style?.color != null) return cssColor(style.color);
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
