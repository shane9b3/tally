/**
 * `Grid`: the framework-agnostic spreadsheet grid.
 *
 *     const grid = new Grid(container, props);
 *     grid.update({ selection });   // push prop changes
 *     grid.destroy();
 *
 * DOM: a root div holding the main and overlay canvases (fixed to the
 * viewport), a transparent native scroll container on top (a spacer gives
 * it the content size, scaled past {@link MAX_SCROLL_EXTENT}) and the editor
 * textarea. Every change goes through `invalidate()`, coalesced into one
 * requestAnimationFrame.
 */
import type { CellPos, CellView, RangeDto, SheetLayout, StyleDto } from "@tally/bridge";
import { CellEditor } from "./editor";
import { Geometry, HEADER_HEIGHT, MIN_ROW_HEADER_WIDTH, NO_FROZEN, type Frozen } from "./geometry";
import { Interaction, type InteractionHost, type ResizeGuide } from "./interaction";
import { AxisModel, ScrollAxisMap, clamp } from "./layout";
import { CELL_PADDING, Renderer, cellFont, drawStyleId, dropdownButtonRect, filterButtonRect, headerFont, linkButtonRect, type FrameStats } from "./renderer";
import { sameSelection, snapToMerges, type Limits } from "./selection";
import { TileTracker } from "./tiles";
import { CELL_FLAGS, MAX_COLS, MAX_ROWS, type CellButtonKind, type ClientRect, type DirtySet, type GridProps, type Selection, type Viewport } from "./types";

/** Font the grid waits for before its first paint. */
const FONT_PROBE = '13px "Inter Variable"';
const FONT_TIMEOUT_MS = 1500;

export interface GridOptions {
  rows?: number;
  cols?: number;
}

export class Grid {
  private props: GridProps;
  private readonly root: HTMLDivElement;
  private readonly scroller: HTMLDivElement;
  private readonly spacer: HTMLDivElement;
  private readonly renderer: Renderer;
  private readonly editor: CellEditor;
  private readonly interaction: Interaction;
  private readonly tracker: TileTracker;
  private readonly rowsCount: number;
  private readonly colsCount: number;
  private rows: AxisModel;
  private cols: AxisModel;
  private layout: SheetLayout | null = null;
  private selection: Selection;
  private unsubscribe: (() => void) | null = null;
  private resizeObserver: ResizeObserver | null = null;

  // Scroll state: exact content offsets in screen px (zoomed).
  private contentX = 0;
  private contentY = 0;
  private mapX = new ScrollAxisMap(0, 0);
  private mapY = new ScrollAxisMap(0, 0);
  private expectedScroll: { left: number; top: number } | null = null;

  private width = 0;
  private height = 0;
  private dpr = 1;
  private headerW = MIN_ROW_HEADER_WIDTH;
  private headerH = HEADER_HEIGHT;
  private geo: Geometry;

  private raf = 0;
  private dirtyMain = true;
  private dirtyOverlay = true;
  private fontsReady = false;
  private destroyed = false;
  private hoverGutter: { axis: "col" | "row"; index: number } | null = null;
  private resizeGuide: ResizeGuide | null = null;
  private fillPreview: RangeDto | null = null;
  /** A cell whose edit was refused (lock badge), and its timer. */
  private lockedCell: CellPos | null = null;
  private lockedTimer: ReturnType<typeof setTimeout> | null = null;
  /** The note cell under the pointer (for `onNoteHover`). */
  private hoverNote: CellPos | null = null;

  /** Recent frame durations (ms between rAF callbacks while drawing). */
  private readonly frameTimes: number[] = [];
  private lastFrameAt = 0;
  private lastStats: FrameStats = { cells: 0, textDraws: 0, ms: 0 };

  constructor(container: HTMLElement, props: GridProps, opts: GridOptions = {}) {
    this.props = { ...props };
    this.selection = props.selection;
    this.rowsCount = opts.rows ?? MAX_ROWS;
    this.colsCount = opts.cols ?? MAX_COLS;
    const doc = container.ownerDocument;

    const root = doc.createElement("div");
    root.className = "tally-grid";
    Object.assign(root.style, {
      position: "relative",
      width: "100%",
      height: "100%",
      overflow: "hidden",
      userSelect: "none",
      webkitUserSelect: "none",
      background: props.theme.colors.bg,
      contain: "strict",
    } satisfies Partial<CSSStyleDeclaration>);
    const main = doc.createElement("canvas");
    const overlay = doc.createElement("canvas");
    for (const c of [main, overlay]) Object.assign(c.style, { position: "absolute", left: "0", top: "0", pointerEvents: "none" });
    main.style.zIndex = "0";
    overlay.style.zIndex = "1";
    const scroller = doc.createElement("div");
    Object.assign(scroller.style, {
      position: "absolute",
      inset: "0",
      overflow: "auto",
      zIndex: "2",
      overscrollBehavior: "none",
    } satisfies Partial<CSSStyleDeclaration>);
    const spacer = doc.createElement("div");
    Object.assign(spacer.style, { width: "1px", height: "1px", pointerEvents: "none" });
    scroller.appendChild(spacer);
    this.editor = new CellEditor(doc);
    root.append(main, overlay, scroller);
    this.editor.mount(root);
    container.appendChild(root);
    this.root = root;
    this.scroller = scroller;
    this.spacer = spacer;

    this.renderer = new Renderer(main, overlay, doc.createElement("canvas"));
    this.rows = new AxisModel(this.rowsCount, 24);
    this.cols = new AxisModel(this.colsCount, 100);
    this.geo = this.makeGeometry();
    this.tracker = new TileTracker({ ensureRanges: (r) => this.props.source.ensureRanges(r) }, this.rowsCount, this.colsCount);
    this.tracker.reset(props.sheet);
    this.attachSource();
    this.reloadLayout(true);

    const self = this;
    const host: InteractionHost = {
      root,
      scroller,
      editor: this.editor,
      geometry: () => self.geo,
      limits: () => self.limits(),
      selection: () => self.selection,
      events: () => self.props,
      select: (sel, o) => self.applySelection(sel, o.emit, o.reveal),
      scrollBy: (dx, dy) => self.setContentOffset(self.contentX + dx, self.contentY + dy),
      reveal: (cell) => self.scrollCellIntoView(cell),
      previewColWidth: (start, end, px) => {
        self.cols.setSize(start, end, px);
        self.relayout();
      },
      previewRowHeight: (start, end, px) => {
        self.rows.setSize(start, end, px);
        self.relayout();
      },
      setHoverGutter: (g) => {
        const cur = self.hoverGutter;
        if (g?.axis !== cur?.axis || g?.index !== cur?.index) {
          self.hoverGutter = g;
          self.invalidate("overlay");
        }
      },
      setFillPreview: (range) => {
        self.fillPreview = range;
        self.invalidate("overlay");
      },
      pointMode: () => (self.props.pointMode ?? false) && self.editor.open,
      setResizeGuide: (g) => {
        self.resizeGuide = g;
        self.invalidate("overlay");
      },
      invalidateOverlay: () => self.invalidate("overlay"),
      filterButtonAt: (x, y) => self.filterButtonAt(x, y),
      cellButtonAt: (x, y) => self.cellButtonAt(x, y),
      linkAt: (r, c) => {
        const v = self.props.source.getCell(r, c);
        return v && v.flags & CELL_FLAGS.link && v.link ? v.link : null;
      },
      hoverCell: (cell) => self.setHoverCell(cell),
      editorChanged: () => {
        // Show/grow the editor immediately, not a frame later.
        self.layoutEditor();
        self.invalidate("overlay");
      },
    };
    this.interaction = new Interaction(host);

    scroller.addEventListener("scroll", this.onScroll, { passive: true });
    scroller.addEventListener("wheel", this.onWheel, { passive: false });
    const win = doc.defaultView;
    if (win && "ResizeObserver" in win) {
      this.resizeObserver = new win.ResizeObserver(() => this.measure());
      this.resizeObserver.observe(root);
    }
    this.measure();
    if (props.edit) this.editor.sync(props.edit, null);
    this.waitForFonts(doc);
  }

  // -------------------------------------------------------------------------
  // Public API
  // -------------------------------------------------------------------------

  /** Pushes changed props. Only keys present in `partial` are considered. */
  update(partial: Partial<GridProps>): void {
    if (this.destroyed) return;
    const prev = this.props;
    this.props = { ...prev, ...partial };
    let relayout = false;
    if ("source" in partial && partial.source !== prev.source) {
      this.attachSource();
      this.tracker.reset(this.props.sheet);
      relayout = true;
      this.layout = null;
    }
    if ("sheet" in partial && partial.sheet !== prev.sheet) {
      this.tracker.reset(this.props.sheet);
      this.layout = null;
      relayout = true;
      this.contentX = 0;
      this.contentY = 0;
    }
    if (("theme" in partial && partial.theme !== prev.theme) || ("zoom" in partial && partial.zoom !== prev.zoom)) {
      this.renderer.clearCaches();
      this.root.style.background = this.props.theme.colors.bg;
      if (partial.zoom !== undefined && partial.zoom !== prev.zoom && prev.zoom > 0) {
        const k = partial.zoom / prev.zoom;
        this.contentX *= k;
        this.contentY *= k;
      }
      relayout = true;
    }
    if (relayout) {
      if (this.layout === null) this.reloadLayout(true);
      else this.relayout();
    }
    if ("selection" in partial && partial.selection && !sameSelection(partial.selection, this.selection)) {
      this.selection = partial.selection;
      this.invalidate("overlay");
    }
    if ("edit" in partial && partial.edit !== prev.edit) {
      const opened = this.editor.sync(this.props.edit, prev.edit);
      if (opened && this.props.edit) this.scrollCellIntoView(this.props.edit.cell);
      if (!this.props.edit) this.interaction.resetPoint();
      this.layoutEditor();
      this.invalidate("overlay");
    }
    if ("pointMode" in partial && !partial.pointMode && prev.pointMode) this.interaction.resetPoint();
    if ("refHighlights" in partial && partial.refHighlights !== prev.refHighlights) this.invalidate("overlay");
    if ("arrayRange" in partial && partial.arrayRange !== prev.arrayRange) this.invalidate("overlay");
    if ("clipRange" in partial && partial.clipRange !== prev.clipRange) this.invalidate("overlay");
    if ("filterButtons" in partial && partial.filterButtons !== prev.filterButtons) this.invalidate("main");
    if ("pageBreaks" in partial && partial.pageBreaks !== prev.pageBreaks) this.invalidate("main");
    if ("printRange" in partial && partial.printRange !== prev.printRange) this.invalidate("main");
    if ("editHighlights" in partial && partial.editHighlights !== prev.editHighlights) {
      this.editor.setHighlights(this.props.editHighlights ?? []);
    }
  }

  /**
   * Width (CSS px at zoom 100%) that fits the widest text in column `c`:
   * the visible cells the source has, plus `extra` cells (e.g. fetched
   * further down the column). `null` if there is no text.
   */
  measureColumnWidth(c: number, extra: readonly CellView[] = []): number | null {
    const theme = this.props.theme;
    const source = this.props.source;
    const vis = this.geo.visibleRange();
    let best = 0;
    const consider = (cell: CellView | undefined): void => {
      if (!cell || cell.text === "") return;
      const style = source.getStyle(drawStyleId(cell));
      const w = this.renderer.measurer.measure(cellFont(style, theme, 1), cell.text) + (cell.cfIcon ? 18 : 0);
      const chip = style?.display?.type === "chip" ? 16 + 8 : 0;
      best = Math.max(best, w + chip);
    };
    for (let r = vis.r0; r <= vis.r1; r++) consider(source.getCell(r, c));
    for (const cell of extra) if (cell.c === c) consider(cell);
    if (best === 0) return null;
    return Math.ceil(best + 2 * CELL_PADDING + 6);
  }

  /** The in-cell editor's client rect while it is open (popups anchor to it). */
  editorRect(): DOMRect | null {
    return this.editor.open ? this.editor.el.getBoundingClientRect() : null;
  }

  /**
   * Feedback for an edit the engine refused because the cell is locked on
   * a protected sheet: a lock badge on the cell and a "not allowed" cursor
   * for a moment.
   */
  flashLocked(cell: CellPos, ms = 1600): void {
    this.lockedCell = { ...cell };
    this.scroller.style.cursor = "not-allowed";
    this.root.dataset.locked = `${cell.r},${cell.c}`;
    if (this.lockedTimer) clearTimeout(this.lockedTimer);
    this.lockedTimer = setTimeout(() => {
      this.lockedTimer = null;
      this.lockedCell = null;
      delete this.root.dataset.locked;
      if (this.scroller.style.cursor === "not-allowed") this.scroller.style.cursor = "default";
      this.invalidate("overlay");
    }, ms);
    this.invalidate("overlay");
  }

  /** Client rect of a cell (popover anchors), or `null` when it is off screen. */
  cellClientRect(cell: CellPos): ClientRect | null {
    const r = this.geo.rangeRect({ r0: cell.r, c0: cell.c, r1: cell.r, c1: cell.c });
    if (r.w <= 0 || r.h <= 0) return null;
    const o = this.root.getBoundingClientRect();
    return { left: o.left + r.x, top: o.top + r.y, right: o.left + r.x + r.w, bottom: o.top + r.y + r.h };
  }

  /** Shows a selection without emitting `onSelectionChange`. */
  setSelection(sel: Selection, reveal = false): void {
    this.applySelection(sel, false, reveal ? sel.active : undefined);
  }

  getSelection(): Selection {
    return this.selection;
  }

  /** Scrolls so that `cell` is at the top-left of the viewport. */
  scrollTo(cell: CellPos): void {
    const g = this.geo;
    this.setContentOffset(this.cols.indexToPx(cell.c) * this.zoom - g.frozenW, this.rows.indexToPx(cell.r) * this.zoom - g.frozenH);
  }

  /** Scrolls the minimum amount that makes `cell` fully visible. */
  scrollCellIntoView(cell: CellPos): void {
    const next = this.geo.scrollToReveal(cell);
    if (next) this.setContentOffset(next.x, next.y);
  }

  /** The visible cells and scroll offsets. */
  getViewport(): Viewport {
    const v = this.geo.visibleRange();
    return { ...v, scrollX: this.contentX, scrollY: this.contentY, width: this.width, height: this.height };
  }

  /** Focuses the grid (its hidden textarea). */
  focus(): void {
    this.interaction.focus();
  }

  /** Schedules a repaint (coalesced into one animation frame). */
  invalidate(what: "all" | "main" | "overlay" = "all"): void {
    if (this.destroyed) return;
    if (what !== "overlay") this.dirtyMain = true;
    this.dirtyOverlay = true;
    if (this.raf) return;
    if (typeof requestAnimationFrame === "undefined") return;
    this.raf = requestAnimationFrame((t) => this.frame(t));
  }

  /** Draws synchronously (tests, screenshots). */
  flush(): void {
    if (this.raf) cancelAnimationFrame(this.raf);
    this.raf = 0;
    this.frame(typeof performance !== "undefined" ? performance.now() : 0, true);
  }

  /** Frame timing for the perf demo: durations between drawn frames, and the last frame's work. */
  getStats(): { frameTimes: number[]; last: FrameStats; measureHits: number; measureMisses: number } {
    return {
      frameTimes: [...this.frameTimes],
      last: this.lastStats,
      measureHits: this.renderer.measurer.hits,
      measureMisses: this.renderer.measurer.misses,
    };
  }

  resetStats(): void {
    this.frameTimes.length = 0;
    this.lastFrameAt = 0;
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    if (this.raf) cancelAnimationFrame(this.raf);
    if (this.lockedTimer) clearTimeout(this.lockedTimer);
    this.unsubscribe?.();
    this.resizeObserver?.disconnect();
    this.interaction.destroy();
    this.scroller.removeEventListener("scroll", this.onScroll);
    this.scroller.removeEventListener("wheel", this.onWheel);
    this.root.remove();
  }

  // -------------------------------------------------------------------------
  // Internals
  // -------------------------------------------------------------------------

  private get zoom(): number {
    return this.props.zoom > 0 ? this.props.zoom : 1;
  }

  private limits(): Limits {
    return {
      rows: this.rowsCount,
      cols: this.colsCount,
      pageRows: this.geo.pageRows(),
      pageCols: this.geo.pageCols(),
      used: this.layout?.used ?? null,
    };
  }

  private attachSource(): void {
    this.unsubscribe?.();
    this.unsubscribe = this.props.source.subscribe((dirty) => this.onSourceChange(dirty));
  }

  private onSourceChange(dirty: DirtySet): void {
    this.tracker.invalidate(dirty);
    if (dirty === "all") this.reloadLayout(false);
    this.invalidate("main");
  }

  private reloadLayout(force: boolean): void {
    const layout = this.props.source.getLayout(this.props.sheet);
    if (!force && layout === this.layout) return;
    this.layout = layout;
    const axes = AxisModel.fromLayout(layout, this.rowsCount, this.colsCount);
    this.rows = axes.rows;
    this.cols = axes.cols;
    this.relayout();
  }

  private waitForFonts(doc: Document): void {
    const fonts = (doc as Document & { fonts?: FontFaceSet }).fonts;
    if (!fonts || typeof fonts.load !== "function") {
      this.fontsReady = true;
      this.invalidate();
      return;
    }
    let done = false;
    const ready = (): void => {
      if (done || this.destroyed) return;
      done = true;
      this.fontsReady = true;
      this.renderer.clearCaches();
      this.invalidate();
    };
    fonts.load(FONT_PROBE).then(ready, ready);
    setTimeout(ready, FONT_TIMEOUT_MS);
  }

  /** Reads the container size and DPR. */
  private measure(): void {
    const w = this.scroller.clientWidth || this.root.clientWidth;
    const h = this.scroller.clientHeight || this.root.clientHeight;
    const dpr = this.root.ownerDocument.defaultView?.devicePixelRatio ?? 1;
    if (w === this.width && h === this.height && dpr === this.dpr) return;
    this.width = w;
    this.height = h;
    this.dpr = dpr;
    this.renderer.resize(w, h, dpr);
    this.relayout();
  }

  /** Recomputes header sizes, scroll maps and the spacer. */
  private relayout(): void {
    const zoom = this.zoom;
    this.headerH = Math.round(HEADER_HEIGHT * zoom);
    // Row header fits the widest row number currently visible (≥ 3 digits).
    const lastRow = Math.min(this.rowsCount, this.rows.pxToIndex((this.contentY + this.height) / zoom) + 1);
    const digits = Math.max(3, String(lastRow).length);
    const digitW = this.renderer.measurer.measure(headerFont(this.props.theme, zoom, true), "0".repeat(digits));
    this.headerW = Math.max(Math.round(MIN_ROW_HEADER_WIDTH * zoom), Math.ceil(digitW + 20 * zoom));

    const cellsW = Math.max(0, this.width - this.headerW);
    const cellsH = Math.max(0, this.height - this.headerH);
    this.mapX = new ScrollAxisMap(this.cols.total() * zoom, cellsW);
    this.mapY = new ScrollAxisMap(this.rows.total() * zoom, cellsH);
    this.spacer.style.width = `${Math.ceil(this.headerW + this.mapX.extent)}px`;
    this.spacer.style.height = `${Math.ceil(this.headerH + this.mapY.extent)}px`;
    this.contentX = clamp(this.contentX, 0, this.mapX.maxContent);
    this.contentY = clamp(this.contentY, 0, this.mapY.maxContent);
    this.syncScrollPosition();
    this.geo = this.makeGeometry();
    this.invalidate("all");
  }

  private makeGeometry(): Geometry {
    return new Geometry(
      this.rows,
      this.cols,
      this.zoom,
      this.headerW,
      this.headerH,
      Math.round(this.contentX * this.dpr) / this.dpr,
      Math.round(this.contentY * this.dpr) / this.dpr,
      this.width,
      this.height,
      this.frozen(),
      this.layout?.merges ?? [],
    );
  }

  /** Frozen rows/columns from the sheet layout. */
  private frozen(): Frozen {
    const f = this.layout?.frozen;
    return f ? { rows: f.r, cols: f.c } : NO_FROZEN;
  }

  /** The AutoFilter button column under (x, y) (canvas px), or `null`. */
  private filterButtonAt(x: number, y: number): { col: number; rect: { left: number; top: number; right: number; bottom: number } } | null {
    const fb = this.props.filterButtons;
    if (!fb) return null;
    const geo = this.geo;
    if (x < geo.headerW || y < geo.headerH) return null;
    const r = geo.rowAt(y);
    const c = geo.colAt(x);
    if (r !== fb.row || c < fb.c0 || c > fb.c1) return null;
    const b = filterButtonRect(geo, r, c);
    const pad = 2;
    if (x < b.x - pad || x > b.x + b.w + pad || y < b.y - pad || y > b.y + b.h + pad) return null;
    const o = this.root.getBoundingClientRect();
    return { col: c, rect: { left: o.left + b.x, top: o.top + b.y, right: o.left + b.x + b.w, bottom: o.top + b.y + b.h } };
  }

  /**
   * The cell decoration under (x, y) (canvas px): the validation ▾ of a
   * list cell, or the ↗ of the active link cell.
   */
  private cellButtonAt(x: number, y: number): { kind: CellButtonKind; cell: CellPos; rect: ClientRect } | null {
    const geo = this.geo;
    if (x < geo.headerW || y < geo.headerH) return null;
    const r = geo.rowAt(y);
    const c = geo.colAt(x);
    const v = this.props.source.getCell(r, c);
    if (!v || (v.flags & (CELL_FLAGS.dropdown | CELL_FLAGS.link)) === 0) return null;
    const fb = this.props.filterButtons;
    const underFilter = !!fb && fb.row === r && c >= fb.c0 && c <= fb.c1;
    const pad = 2;
    const within = (b: { x: number; y: number; w: number; h: number }): boolean =>
      b.w > 4 && x >= b.x - pad && x <= b.x + b.w + pad && y >= b.y - pad && y <= b.y + b.h + pad;
    const o = this.root.getBoundingClientRect();
    const client = (b: { x: number; y: number; w: number; h: number }): ClientRect => ({ left: o.left + b.x, top: o.top + b.y, right: o.left + b.x + b.w, bottom: o.top + b.y + b.h });
    const dropdown = (v.flags & CELL_FLAGS.dropdown) !== 0 && !underFilter;
    if (dropdown) {
      const b = dropdownButtonRect(geo, r, c);
      if (within(b)) return { kind: "dropdown", cell: { r, c }, rect: client(b) };
    }
    const a = this.selection.active;
    if (v.flags & CELL_FLAGS.link && v.link && a.r === r && a.c === c && !this.editor.open) {
      const b = linkButtonRect(geo, r, c, dropdown);
      if (within(b)) return { kind: "link", cell: { r, c }, rect: client(b) };
    }
    return null;
  }

  /** Tracks the note cell under the pointer and reports changes (`onNoteHover`). */
  private setHoverCell(cell: CellPos | null): void {
    const v = cell ? this.props.source.getCell(cell.r, cell.c) : undefined;
    const note = cell && v && v.flags & CELL_FLAGS.note ? cell : null;
    const cur = this.hoverNote;
    if (note?.r === cur?.r && note?.c === cur?.c) return;
    this.hoverNote = note;
    this.props.onNoteHover?.(note, note ? this.cellClientRect(note) : null);
  }

  /** Sets the exact content offset and derives the native scroll position. */
  private setContentOffset(x: number, y: number): void {
    const nx = clamp(x, 0, this.mapX.maxContent);
    const ny = clamp(y, 0, this.mapY.maxContent);
    if (nx === this.contentX && ny === this.contentY) return;
    this.contentX = nx;
    this.contentY = ny;
    this.syncScrollPosition();
    this.afterScroll();
  }

  private syncScrollPosition(): void {
    const left = this.mapX.toScroll(this.contentX);
    const top = this.mapY.toScroll(this.contentY);
    this.expectedScroll = { left, top };
    if (Math.abs(this.scroller.scrollLeft - left) >= 1) this.scroller.scrollLeft = left;
    if (Math.abs(this.scroller.scrollTop - top) >= 1) this.scroller.scrollTop = top;
  }

  private afterScroll(): void {
    const prevHeaderW = this.headerW;
    this.geo = this.makeGeometry();
    // Row numbers may need a wider header after a long jump.
    const digits = Math.max(3, String(this.geo.visibleRange().r1 + 1).length);
    const digitW = this.renderer.measurer.measure(headerFont(this.props.theme, this.zoom, true), "0".repeat(digits));
    const want = Math.max(Math.round(MIN_ROW_HEADER_WIDTH * this.zoom), Math.ceil(digitW + 20 * this.zoom));
    if (want !== prevHeaderW) this.relayout();
    else this.invalidate("all");
  }

  private readonly onScroll = (): void => {
    const left = this.scroller.scrollLeft;
    const top = this.scroller.scrollTop;
    const exp = this.expectedScroll;
    // Positions we set ourselves keep the exact content offset; anything
    // else (scrollbar drag, native wheel) is mapped back.
    const x = exp && Math.abs(left - exp.left) < 1 ? this.contentX : this.mapX.toContent(left);
    const y = exp && Math.abs(top - exp.top) < 1 ? this.contentY : this.mapY.toContent(top);
    if (x === this.contentX && y === this.contentY) return;
    this.contentX = x;
    this.contentY = y;
    this.expectedScroll = null;
    this.afterScroll();
  };

  /**
   * When an axis is scaled, wheel deltas are applied in content px (so the
   * wheel speed stays 1:1); otherwise the browser scrolls natively.
   */
  private readonly onWheel = (e: WheelEvent): void => {
    if (!this.mapX.scaled && !this.mapY.scaled) return;
    if (e.ctrlKey) return; // pinch-zoom gesture: leave to the app
    e.preventDefault();
    const line = this.rows.defaultSize * this.zoom;
    const unit = e.deltaMode === 1 ? line : e.deltaMode === 2 ? this.height : 1;
    let dx = e.deltaX * unit;
    let dy = e.deltaY * unit;
    if (e.shiftKey && dx === 0) {
      dx = dy;
      dy = 0;
    }
    this.setContentOffset(this.contentX + dx, this.contentY + dy);
  };

  private applySelection(sel: Selection, emit: boolean, reveal?: CellPos): void {
    const merges = this.layout?.merges ?? [];
    if (merges.length > 0) {
      // Only keyboard moves (which reveal the new cell) jump over a merged area.
      sel = snapToMerges(sel, reveal ? this.selection : null, merges, { rows: this.rowsCount, cols: this.colsCount });
      if (reveal) reveal = sel.active;
    }
    const changed = !sameSelection(sel, this.selection);
    this.selection = sel;
    if (reveal) this.scrollCellIntoView(reveal);
    if (changed) {
      this.invalidate("overlay");
      if (emit) this.props.onSelectionChange?.(sel);
    }
  }

  private frame(t: number, sync = false): void {
    this.raf = 0;
    if (this.destroyed) return;
    if (!this.fontsReady && !sync) return;
    if (!sync && this.dirtyMain) {
      // Only consecutive drawn frames count (an idle gap is not a slow frame).
      const dt = t - this.lastFrameAt;
      if (this.lastFrameAt > 0 && dt < 250) {
        this.frameTimes.push(dt);
        if (this.frameTimes.length > 2000) this.frameTimes.shift();
      }
      this.lastFrameAt = t;
    }
    const dpr = this.root.ownerDocument.defaultView?.devicePixelRatio ?? 1;
    if (dpr !== this.dpr) this.measure();

    const base = { geo: this.geo, source: this.props.source, theme: this.props.theme, dpr: this.dpr };
    if (this.dirtyMain) {
      this.dirtyMain = false;
      this.lastStats = this.renderer.drawMain({
        ...base,
        filterButtons: this.props.filterButtons ?? null,
        pageBreaks: this.props.pageBreaks ?? null,
        printRange: this.props.printRange ?? null,
      });
      const panes = this.geo.panes();
      // The scrolling pane first (it is the largest), then frozen ones.
      for (let i = panes.length - 1; i >= 0; i--) {
        const p = panes[i];
        if (p) this.tracker.ensureVisible({ r0: p.r0, c0: p.c0, r1: p.r1, c1: p.c1 });
      }
      // Anchors of merged areas that start above/left of the view.
      const vis = this.geo.visibleRange();
      for (const m of this.geo.merges) {
        if (m.r0 <= vis.r1 && m.r1 >= vis.r0 && m.c0 <= vis.c1 && m.c1 >= vis.c0 && (m.r0 < vis.r0 || m.c0 < vis.c0)) {
          this.tracker.ensureVisible({ r0: m.r0, c0: m.c0, r1: m.r0, c1: m.c0 });
        }
      }
    }
    if (this.dirtyOverlay) {
      this.dirtyOverlay = false;
      this.renderer.drawOverlay({
        ...base,
        selection: this.selection,
        editing: this.editor.open,
        hoverGutter: this.hoverGutter,
        resizeGuide: this.resizeGuide,
        fillPreview: this.fillPreview,
        refHighlights: this.editor.open ? (this.props.refHighlights ?? []) : [],
        arrayRange: this.props.arrayRange ?? null,
        clipRange: this.editor.open ? null : (this.props.clipRange ?? null),
        lockedCell: this.lockedCell,
      });
      this.layoutEditor();
    }
  }

  private layoutEditor(): void {
    const ed = this.editor;
    const cell = ed.open && ed.cell ? ed.cell : this.selection.active;
    const view = this.props.source.getCell(cell.r, cell.c);
    const style: StyleDto | undefined = this.props.source.getStyle(view?.style ?? 0);
    const h = style?.hAlign;
    const align = h === "right" ? "right" : h === "center" ? "center" : "left";
    ed.layout(this.geo, this.selection.active, { style, theme: this.props.theme, align }, this.renderer.measurer);
  }
}
