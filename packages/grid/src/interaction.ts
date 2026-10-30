/**
 * Mouse, keyboard, editor and clipboard handling. All keyboard input arrives
 * through the focused editor textarea; selection logic is delegated to the
 * pure reducers in selection.ts.
 */
import type { CellPos } from "@tally/bridge";
import type { CellEditor } from "./editor";
import type { Geometry, Hit } from "./geometry";
import { fillTarget } from "./fillHandle";
import {
  addRange,
  applyEdge,
  cellSelection,
  edgeOrigin,
  extendTo,
  extentCorner,
  inRange,
  isWholeCols,
  isWholeRows,
  moveBy,
  reduceKey,
  selectAll,
  selectCols,
  selectRows,
  sheetEdge,
  stepWithin,
  type Limits,
  type Mods,
} from "./selection";
import type { CellButtonKind, ClientRect, CommitMove, ContextArea, Dir, GridEvents, Selection } from "./types";
import type { RangeDto } from "@tally/bridge";

/** Minimum column width a drag can set (zoom 100%). */
export const MIN_COL_WIDTH = 8;
/** Minimum row height a drag can set (zoom 100%). */
export const MIN_ROW_HEIGHT = 6;
/** Half-size of the fill handle's hit square (screen px). */
export const FILL_HANDLE_HIT = 5;

/** A live resize guide: the axis, the index and the new far edge (screen px). */
export interface ResizeGuide {
  axis: "col" | "row";
  index: number;
  pos: number;
}

/** What the interaction layer needs from the grid. */
export interface InteractionHost {
  readonly root: HTMLElement;
  readonly scroller: HTMLElement;
  readonly editor: CellEditor;
  geometry(): Geometry;
  limits(): Limits;
  selection(): Selection;
  events(): Partial<GridEvents>;
  /** Shows a selection, optionally emits it and scrolls `reveal` into view. */
  select(sel: Selection, opts: { emit: boolean; reveal?: CellPos }): void;
  /** Scrolls the content by (dx, dy) screen px. */
  scrollBy(dx: number, dy: number): void;
  /** Scrolls the minimum amount that makes `cell` visible. */
  reveal(cell: CellPos): void;
  /** Live column resize preview (CSS px at zoom 100%). */
  previewColWidth(start: number, end: number, px: number): void;
  /** Live row resize preview (CSS px at zoom 100%). */
  previewRowHeight(start: number, end: number, px: number): void;
  setHoverGutter(g: { axis: "col" | "row"; index: number } | null): void;
  setResizeGuide(guide: ResizeGuide | null): void;
  /** The fill-handle drag preview (the target range), or `null`. */
  setFillPreview(range: RangeDto | null): void;
  /** Whether point mode is on (the `pointMode` prop). */
  pointMode(): boolean;
  /** The open edit belongs to another sheet's cell (the `editElsewhere` prop): a commit leaves the selection alone. */
  editElsewhere?(): boolean;
  /** Whether range-pick mode is on without an edit (the `rangePick` prop or `Grid.pickRange()`). */
  rangePick(): boolean;
  /** A reference was picked (point mode or range pick); the grid forwards `onPointRange` and settles `pickRange()`. */
  pointRange(range: RangeDto, phase: "start" | "move" | "end", extend?: boolean): void;
  /** Escape in range-pick mode: the grid rejects a pending `pickRange()` with `null`. */
  cancelPick(): void;
  invalidateOverlay(): void;
  /** The AutoFilter button under a canvas point, with its client rect. */
  filterButtonAt(x: number, y: number): { col: number; rect: { left: number; top: number; right: number; bottom: number } } | null;
  /** Called when the editor opens/closes/grows. */
  editorChanged(): void;
  /** The validation ▾ / link ↗ under a canvas point. */
  cellButtonAt(x: number, y: number): { kind: CellButtonKind; cell: CellPos; rect: ClientRect } | null;
  /** The link of a cell (flag 32), or `null`. */
  linkAt(r: number, c: number): string | null;
  /** The cell under the pointer (note hover), or `null`. */
  hoverCell(cell: CellPos | null): void;
}

type Drag =
  | { kind: "cells" }
  | { kind: "rows"; anchor: number }
  | { kind: "cols"; anchor: number }
  | { kind: "resize"; axis: "col" | "row"; i: number; start: number; end: number; startPos: number; startSize: number; px: number; px0: number }
  | { kind: "fill"; source: RangeDto; target: RangeDto | null }
  | { kind: "point" };

const DIRS: Record<string, Dir> = { ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right" };

export class Interaction {
  private drag: Drag | null = null;
  private pointerId: number | null = null;
  private last = { x: 0, y: 0 };
  private autoScrollRaf = 0;
  private edgeSeq = 0;
  /** Point mode: the reference being picked (null until the first pick). */
  private pointSel: Selection | null = null;
  /** Set by ⇧⌘V keydown; the following DOM `paste` is "values only". */
  private plainPaste = false;
  private readonly disposers: (() => void)[] = [];

  constructor(private readonly host: InteractionHost) {
    const { scroller, root } = host;
    const ta = host.editor.el;
    this.on(scroller, "pointerdown", (e) => this.onPointerDown(e));
    this.on(scroller, "pointermove", (e) => this.onPointerMove(e));
    this.on(scroller, "pointerup", (e) => this.onPointerUp(e));
    this.on(scroller, "pointercancel", (e) => this.onPointerUp(e));
    this.on(scroller, "pointerleave", () => {
      if (!this.drag) host.setHoverGutter(null);
      host.hoverCell(null);
    });
    this.on(scroller, "mousedown", (e) => {
      // Keep focus in the textarea (and suppress text selection) unless on a scrollbar.
      if (!this.onScrollbar(e)) e.preventDefault();
    });
    this.on(scroller, "dblclick", (e) => this.onDoubleClick(e));
    this.on(scroller, "contextmenu", (e) => this.onContextMenu(e));
    this.on(ta, "keydown", (e) => this.onKeyDown(e));
    this.on(ta, "input", () => this.onInput());
    this.on(ta, "compositionstart", () => this.onCompositionStart());
    this.on(ta, "compositionend", () => {
      host.editor.composing = false;
      this.emitChange();
    });
    this.on(ta, "keyup", () => {
      host.editor.resetIdleValue();
      this.plainPaste = false;
      this.emitCaret();
    });
    this.on(ta, "mouseup", () => {
      // Clicking inside the editor text moves the caret: a new pick starts fresh.
      this.pointSel = null;
      this.emitCaret();
    });
    this.on(root, "copy", (e) => this.onClipboard(e, "copy"));
    this.on(root, "cut", (e) => this.onClipboard(e, "cut"));
    this.on(root, "paste", (e) => this.onClipboard(e, "paste"));
  }

  destroy(): void {
    for (const d of this.disposers) d();
    this.disposers.length = 0;
    if (this.autoScrollRaf) cancelAnimationFrame(this.autoScrollRaf);
  }

  private on<K extends keyof HTMLElementEventMap>(
    el: HTMLElement,
    type: K,
    fn: (e: HTMLElementEventMap[K]) => void,
  ): void {
    const h = fn as EventListener;
    el.addEventListener(type, h);
    this.disposers.push(() => el.removeEventListener(type, h));
  }

  private point(e: MouseEvent): { x: number; y: number } {
    const r = this.host.root.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  private onScrollbar(e: MouseEvent): boolean {
    const p = this.point(e);
    const s = this.host.scroller;
    return p.x >= s.clientWidth || p.y >= s.clientHeight;
  }

  focus(): void {
    this.host.editor.el.focus({ preventScroll: true });
  }

  // -------------------------------------------------------------------------
  // Pointer
  // -------------------------------------------------------------------------

  /** Forgets the point-mode reference (the next pick starts at the edited cell). */
  resetPoint(): void {
    this.pointSel = null;
  }

  /**
   * Whether cell clicks/drags pick references right now: point mode inside
   * an open formula edit, or range-pick mode without an edit (a dialog
   * asking for a range).
   */
  picking(): boolean {
    const host = this.host;
    return host.editor.open ? host.pointMode() : host.rangePick();
  }

  /** Whether `p` is on the fill handle of the selection. */
  private onFillHandle(p: { x: number; y: number }): boolean {
    const host = this.host;
    if (host.editor.open) return false;
    const sel = host.selection();
    if (sel.others && sel.others.length > 0) return false;
    const geo = host.geometry();
    const x = geo.colRight(sel.range.c1);
    const y = geo.rowBottom(sel.range.r1);
    if (x < geo.headerW || y < geo.headerH || x > geo.width + 4 || y > geo.height + 4) return false;
    return Math.abs(p.x - x) <= FILL_HANDLE_HIT && Math.abs(p.y - y) <= FILL_HANDLE_HIT;
  }

  private onPointerDown(e: PointerEvent): void {
    if (this.onScrollbar(e)) return;
    const host = this.host;
    const p = this.point(e);
    this.last = p;
    const geo = host.geometry();
    const lim = host.limits();
    const hit = geo.hit(p.x, p.y);
    const sel = host.selection();
    const add = (e.metaKey || e.ctrlKey) && !e.shiftKey;

    // Point mode: clicking cells picks references for the formula being edited
    // (or, in range-pick mode, for the dialog that asked).
    if (e.button === 0 && this.picking() && hit.area === "cell") {
      const cell = { r: hit.r, c: hit.c };
      const base = this.pointSel ?? cellSelection(host.editor.cell ?? sel.active);
      // Shift with nothing picked on this sheet yet: the app may extend another sheet's pick (3D).
      const extend = e.shiftKey && this.pointSel === null && host.editor.open;
      this.pointSel = e.shiftKey && this.pointSel ? extendTo(base, cell) : cellSelection(cell);
      if (extend) host.pointRange({ ...this.pointSel.range }, "start", true);
      else host.pointRange({ ...this.pointSel.range }, "start");
      this.drag = { kind: "point" };
      this.capture(e);
      return;
    }
    // A column / row header picks the whole column / row (Shift extends the pick), as in AOO.
    if (e.button === 0 && host.editor.open && this.picking() && (hit.area === "colHeader" || hit.area === "rowHeader")) {
      const whole = hit.area === "colHeader" ? selectCols(hit.c, hit.c, lim, hit.c, 0) : selectRows(hit.r, hit.r, lim, hit.r, 0);
      const prev = this.pointSel;
      const range =
        e.shiftKey && prev
          ? { r0: Math.min(prev.range.r0, whole.range.r0), c0: Math.min(prev.range.c0, whole.range.c0), r1: Math.max(prev.range.r1, whole.range.r1), c1: Math.max(prev.range.c1, whole.range.c1) }
          : whole.range;
      this.pointSel = { ...whole, range };
      host.pointRange({ ...range }, "start");
      host.pointRange({ ...range }, "end");
      e.preventDefault();
      return;
    }
    // Picking on another sheet than the edited cell's: nothing else here commits or selects.
    if (host.editElsewhere?.()) {
      e.preventDefault();
      return;
    }

    this.focus();

    if (e.button === 2) {
      if (hit.area === "cell" && !inRange(sel.range, hit) && !(sel.others ?? []).some((r) => inRange(r, hit))) {
        this.commitIfEditing("none");
        host.select(cellSelection(hit), { emit: true });
      }
      return;
    }
    if (e.button !== 0) return;

    if (this.onFillHandle(p)) {
      this.drag = { kind: "fill", source: { ...sel.range }, target: null };
      this.capture(e);
      return;
    }
    if (hit.area === "cell" && !host.editor.open) {
      const cb = host.cellButtonAt(p.x, p.y);
      if (cb) {
        e.preventDefault();
        host.select(cellSelection(cb.cell), { emit: true });
        host.events().onCellButton?.(cb.kind, cb.cell, cb.rect);
        return;
      }
      const url = add ? host.linkAt(hit.r, hit.c) : null;
      if (url) {
        // ⌘-click on a link opens it (a plain click only selects).
        e.preventDefault();
        host.events().onOpenLink?.({ r: hit.r, c: hit.c }, url);
        return;
      }
    }
    const button = hit.area === "cell" && !host.editor.open ? host.filterButtonAt(p.x, p.y) : null;
    if (button) {
      // No compatibility mousedown: the popover that opens must not see this click as "outside".
      e.preventDefault();
      host.events().onFilterButton?.(button.col, button.rect);
      return;
    }
    if (hit.area !== "colGutter" && hit.area !== "rowGutter") this.commitIfEditing("none");

    switch (hit.area) {
      case "corner":
        host.select(selectAll(lim), { emit: true });
        return;
      case "colGutter": {
        const whole = isWholeCols(sel.range, lim) && hit.c >= sel.range.c0 && hit.c <= sel.range.c1;
        const start = whole ? sel.range.c0 : hit.c;
        const end = whole ? sel.range.c1 : hit.c;
        const startW = geo.colW(hit.c);
        const px = geo.cols.sizeOf(hit.c);
        this.drag = { kind: "resize", axis: "col", i: hit.c, start, end, startPos: p.x, startSize: startW, px, px0: px };
        host.setResizeGuide({ axis: "col", index: hit.c, pos: geo.colX(hit.c) + startW });
        break;
      }
      case "rowGutter": {
        const whole = isWholeRows(sel.range, lim) && hit.r >= sel.range.r0 && hit.r <= sel.range.r1;
        const start = whole ? sel.range.r0 : hit.r;
        const end = whole ? sel.range.r1 : hit.r;
        const startH = geo.rowH(hit.r);
        const px = geo.rows.sizeOf(hit.r);
        this.drag = { kind: "resize", axis: "row", i: hit.r, start, end, startPos: p.y, startSize: startH, px, px0: px };
        host.setResizeGuide({ axis: "row", index: hit.r, pos: geo.rowY(hit.r) + startH });
        break;
      }
      case "colHeader": {
        const activeRow = geo.rowAt(geo.headerH);
        if (add) {
          const cols = selectCols(hit.c, hit.c, lim, hit.c, activeRow);
          host.select(addRange(sel, cols.active, cols.range), { emit: true });
        } else {
          const next = e.shiftKey ? selectCols(sel.anchor.c, hit.c, lim, sel.anchor.c, sel.anchor.r) : selectCols(hit.c, hit.c, lim, hit.c, activeRow);
          host.select(e.shiftKey ? { ...next, active: sel.active, anchor: sel.anchor } : next, { emit: true });
        }
        this.drag = { kind: "cols", anchor: e.shiftKey ? sel.anchor.c : hit.c };
        break;
      }
      case "rowHeader": {
        const activeCol = geo.colAt(geo.headerW);
        if (add) {
          const rows = selectRows(hit.r, hit.r, lim, hit.r, activeCol);
          host.select(addRange(sel, rows.active, rows.range), { emit: true });
        } else {
          const next = e.shiftKey ? selectRows(sel.anchor.r, hit.r, lim, sel.anchor.r, sel.anchor.c) : selectRows(hit.r, hit.r, lim, hit.r, activeCol);
          host.select(e.shiftKey ? { ...next, active: sel.active, anchor: sel.anchor } : next, { emit: true });
        }
        this.drag = { kind: "rows", anchor: e.shiftKey ? sel.anchor.r : hit.r };
        break;
      }
      case "cell": {
        const cell = { r: hit.r, c: hit.c };
        host.select(add ? addRange(sel, cell) : e.shiftKey ? extendTo(sel, cell) : cellSelection(cell), { emit: true });
        this.drag = { kind: "cells" };
        break;
      }
    }
    this.capture(e);
  }

  private capture(e: PointerEvent): void {
    this.pointerId = e.pointerId;
    try {
      this.host.scroller.setPointerCapture(e.pointerId);
    } catch {
      // Synthetic events in tests have no active pointer.
    }
  }

  private onPointerMove(e: PointerEvent): void {
    const host = this.host;
    const p = this.point(e);
    this.last = p;
    if (!this.drag) {
      const hit = host.geometry().hit(p.x, p.y);
      const gutter =
        hit.area === "colGutter" ? { axis: "col" as const, index: hit.c } : hit.area === "rowGutter" ? { axis: "row" as const, index: hit.r } : null;
      host.setHoverGutter(gutter);
      host.hoverCell(hit.area === "cell" ? { r: hit.r, c: hit.c } : null);
      if (host.scroller.style.cursor === "not-allowed") return; // a refused edit's feedback is showing
      const overButton = hit.area === "cell" && !host.editor.open && host.cellButtonAt(p.x, p.y) !== null;
      host.scroller.style.cursor = overButton
        ? "pointer"
        : gutter
        ? gutter.axis === "col"
          ? "col-resize"
          : "row-resize"
        : this.onFillHandle(p)
          ? "crosshair"
          : this.picking() && hit.area === "cell"
            ? "cell"
            : "default";
      return;
    }
    if (this.drag.kind === "resize") {
      const geo = host.geometry();
      const d = this.drag;
      if (d.axis === "col") {
        const w = Math.max(MIN_COL_WIDTH * geo.zoom, d.startSize + (p.x - d.startPos));
        d.px = Math.round(w / geo.zoom);
        host.previewColWidth(d.i, d.i, d.px);
        host.setResizeGuide({ axis: "col", index: d.i, pos: host.geometry().colX(d.i) + w });
      } else {
        const h = Math.max(MIN_ROW_HEIGHT * geo.zoom, d.startSize + (p.y - d.startPos));
        d.px = Math.round(h / geo.zoom);
        host.previewRowHeight(d.i, d.i, d.px);
        host.setResizeGuide({ axis: "row", index: d.i, pos: host.geometry().rowY(d.i) + h });
      }
      return;
    }
    this.dragTo(p);
    this.ensureAutoScroll();
  }

  private dragTo(p: { x: number; y: number }): void {
    const host = this.host;
    const geo = host.geometry();
    const lim = host.limits();
    const sel = host.selection();
    const x = Math.max(geo.headerW, Math.min(geo.width - 1, p.x));
    const y = Math.max(geo.headerH, Math.min(geo.height - 1, p.y));
    const drag = this.drag;
    if (!drag) return;
    const cell = { r: geo.rowAt(y), c: geo.colAt(x) };
    if (drag.kind === "fill") {
      const t = fillTarget(drag.source, cell);
      const target = t ? t.target : null;
      const same = (a: RangeDto | null, b: RangeDto | null): boolean =>
        a === b || (a !== null && b !== null && a.r0 === b.r0 && a.c0 === b.c0 && a.r1 === b.r1 && a.c1 === b.c1);
      if (!same(target, drag.target)) {
        drag.target = target;
        host.setFillPreview(target);
      }
      return;
    }
    if (drag.kind === "point") {
      const ps = this.pointSel;
      if (!ps) return;
      const next = extendTo(ps, cell);
      if (next.range.r0 !== ps.range.r0 || next.range.r1 !== ps.range.r1 || next.range.c0 !== ps.range.c0 || next.range.c1 !== ps.range.c1) {
        this.pointSel = next;
        host.pointRange({ ...next.range }, "move");
      }
      return;
    }
    let next: Selection | null = null;
    if (drag.kind === "cells") next = extendTo(sel, cell);
    else if (drag.kind === "cols") next = { ...selectCols(drag.anchor, geo.colAt(x), lim), active: sel.active, anchor: sel.anchor };
    else if (drag.kind === "rows") next = { ...selectRows(drag.anchor, geo.rowAt(y), lim), active: sel.active, anchor: sel.anchor };
    if (next) host.select(next, { emit: true });
  }

  private ensureAutoScroll(): void {
    if (this.autoScrollRaf || typeof requestAnimationFrame === "undefined") return;
    const step = (): void => {
      this.autoScrollRaf = 0;
      const drag = this.drag;
      if (!drag || drag.kind === "resize") return;
      const geo = this.host.geometry();
      const { x, y } = this.last;
      const speed = (d: number): number => Math.sign(d) * Math.min(80, Math.ceil(Math.abs(d) / 2) ** 1.2);
      let dx = 0;
      let dy = 0;
      if (drag.kind !== "rows") {
        if (x > geo.width) dx = speed(x - geo.width);
        else if (x < geo.headerW) dx = speed(x - geo.headerW);
      }
      if (drag.kind !== "cols") {
        if (y > geo.height) dy = speed(y - geo.height);
        else if (y < geo.headerH) dy = speed(y - geo.headerH);
      }
      if (dx === 0 && dy === 0) return;
      this.host.scrollBy(dx, dy);
      this.dragTo(this.last);
      this.autoScrollRaf = requestAnimationFrame(step);
    };
    this.autoScrollRaf = requestAnimationFrame(step);
  }

  private onPointerUp(e: PointerEvent): void {
    const host = this.host;
    const drag = this.drag;
    this.drag = null;
    if (this.pointerId !== null) {
      try {
        host.scroller.releasePointerCapture(this.pointerId);
      } catch {
        // Already released.
      }
      this.pointerId = null;
    }
    if (drag?.kind === "resize") {
      host.setResizeGuide(null);
      const changed = drag.px !== drag.px0 || drag.start !== drag.end;
      if (drag.axis === "col") host.previewColWidth(drag.start, drag.end, drag.px);
      else host.previewRowHeight(drag.start, drag.end, drag.px);
      if (changed) host.events().onResize?.(drag.axis, drag.start, drag.end, drag.px);
      const p = this.point(e);
      const hit = host.geometry().hit(p.x, p.y);
      host.setHoverGutter(
        hit.area === "colGutter" ? { axis: "col", index: hit.c } : hit.area === "rowGutter" ? { axis: "row", index: hit.r } : null,
      );
    } else if (drag?.kind === "fill") {
      host.setFillPreview(null);
      if (drag.target) {
        const target = drag.target;
        host.select({ range: { ...target }, active: host.selection().active, anchor: { r: target.r0, c: target.c0 } }, { emit: true });
        host.events().onFill?.({ ...drag.source }, { ...target });
      }
    } else if (drag?.kind === "point" && this.pointSel) {
      host.pointRange({ ...this.pointSel.range }, "end");
    }
  }

  private onDoubleClick(e: MouseEvent): void {
    if (this.onScrollbar(e)) return;
    const p = this.point(e);
    const host = this.host;
    const geo = host.geometry();
    const hit: Hit = geo.hit(p.x, p.y);
    const sel = host.selection();
    if (this.onFillHandle(p)) {
      e.preventDefault();
      host.events().onFillHandleDoubleClick?.({ ...sel.range });
      return;
    }
    if (hit.area === "colGutter") {
      e.preventDefault();
      const lim = host.limits();
      const whole = isWholeCols(sel.range, lim) && hit.c >= sel.range.c0 && hit.c <= sel.range.c1;
      host.events().onAutoFit?.("col", whole ? sel.range.c0 : hit.c, whole ? sel.range.c1 : hit.c);
      return;
    }
    if (hit.area !== "cell") return;
    if (this.picking()) return;
    e.preventDefault();
    this.startEdit("edit");
  }

  private onContextMenu(e: MouseEvent): void {
    if (this.onScrollbar(e)) return;
    e.preventDefault();
    const host = this.host;
    if (host.editElsewhere?.()) return;
    const p = this.point(e);
    const geo = host.geometry();
    const lim = host.limits();
    const hit = geo.hit(p.x, p.y);
    const sel = host.selection();
    let cell: CellPos = sel.active;
    let area: ContextArea = "cell";
    if (hit.area === "cell") {
      cell = { r: hit.r, c: hit.c };
      if (!inRange(sel.range, cell) && !(sel.others ?? []).some((r) => inRange(r, cell))) host.select(cellSelection(cell), { emit: true });
    } else if (hit.area === "colHeader" || hit.area === "colGutter") {
      area = "colHeader";
      cell = { r: sel.active.r, c: hit.c };
      if (!(isWholeCols(sel.range, lim) && hit.c >= sel.range.c0 && hit.c <= sel.range.c1)) {
        host.select(selectCols(hit.c, hit.c, lim, hit.c, geo.rowAt(geo.headerH)), { emit: true });
      }
    } else if (hit.area === "rowHeader" || hit.area === "rowGutter") {
      area = "rowHeader";
      cell = { r: hit.r, c: sel.active.c };
      if (!(isWholeRows(sel.range, lim) && hit.r >= sel.range.r0 && hit.r <= sel.range.r1)) {
        host.select(selectRows(hit.r, hit.r, lim, hit.r, geo.colAt(geo.headerW)), { emit: true });
      }
    } else if (hit.area === "corner") area = "corner";
    host.events().onContextMenu?.(cell, e.clientX, e.clientY, area);
  }

  // -------------------------------------------------------------------------
  // Editing
  // -------------------------------------------------------------------------

  /** Opens the editor at the active cell. */
  startEdit(mode: "enter" | "edit", seed?: string): void {
    const host = this.host;
    const ed = host.editor;
    const cell = host.selection().active;
    if (ed.open) return;
    ed.begin(cell, mode);
    if (seed !== undefined) {
      ed.noteEmitted(seed);
      if (ed.value !== seed) ed.setValue(seed);
    }
    host.select(host.selection(), { emit: false, reveal: cell });
    host.events().onStartEdit?.(cell, seed);
    host.editorChanged();
  }

  /** Commits an open edit and moves the selection. */
  commit(move: CommitMove): void {
    const host = this.host;
    const ed = host.editor;
    if (!ed.open) return;
    const elsewhere = host.editElsewhere?.() ?? false;
    host.events().onCommitEdit?.(move);
    ed.end();
    host.editorChanged();
    if (move === "none" || elsewhere) return;
    const sel = host.selection();
    const lim = host.limits();
    const key = this.lastCommitKey;
    if ((key === "Enter" || key === "Tab") && lim.protectedNav && host.events().onNextUnlocked && (move === "down" || move === "up" || move === "left" || move === "right")) {
      this.lastCommitKey = "";
      const seq = ++this.edgeSeq;
      void this.stepProtected(sel, move, lim).then((next) => {
        if (next && seq === this.edgeSeq) host.select(next, { emit: true, reveal: next.active });
      });
      return;
    }
    const next = move === "down" || move === "up" || move === "left" || move === "right" ? this.moveAfterCommit(sel, move, lim) : sel;
    host.select(next, { emit: true, reveal: next.active });
  }

  /** Protected sheet: the selection after Tab / Enter (`null` when the engine finds no cell or fails). */
  private async stepProtected(sel: Selection, dir: Dir, lim: Limits): Promise<Selection | null> {
    const handler = this.host.events().onNextUnlocked;
    if (!handler) return null;
    const rg = sel.range;
    const single = (rg.r0 === rg.r1 && rg.c0 === rg.c1) || (lim.merges ?? []).some((m) => m.r0 === rg.r0 && m.c0 === rg.c0 && m.r1 === rg.r1 && m.c1 === rg.c1);
    try {
      const cell = await handler(sel.active, dir, single ? null : { ...sel.range });
      if (!cell) return null;
      return single ? cellSelection(cell) : { range: { ...sel.range }, active: cell, anchor: { ...cell } };
    } catch {
      return null;
    }
  }

  private moveAfterCommit(sel: Selection, move: Dir, lim: Limits): Selection {
    const ed = this.lastCommitKey;
    this.lastCommitKey = "";
    if (ed === "Enter" || ed === "Tab") return stepWithin(sel, move, lim);
    const d = { up: [-1, 0], down: [1, 0], left: [0, -1], right: [0, 1] }[move];
    return moveBy(sel, d[0] ?? 0, d[1] ?? 0, false, lim);
  }

  private lastCommitKey = "";

  cancel(): void {
    const host = this.host;
    if (!host.editor.open) return;
    host.events().onCancelEdit?.();
    host.editor.end();
    host.editorChanged();
  }

  private commitIfEditing(move: CommitMove): void {
    if (this.host.editor.open) this.commit(move);
  }

  private lastCaret = -1;

  /** Reports caret moves (arrow keys in "edit" mode, clicks) without a text change. */
  private emitCaret(): void {
    const ed = this.host.editor;
    if (!ed.open || ed.composing) return;
    const caret = ed.caret;
    if (caret === this.lastCaret) return;
    this.lastCaret = caret;
    this.host.events().onEditChange?.(ed.value, caret);
  }

  private emitChange(): void {
    const ed = this.host.editor;
    if (!ed.open) return;
    const text = ed.value;
    ed.noteEmitted(text);
    this.lastCaret = ed.caret;
    this.host.events().onEditChange?.(text, ed.caret);
    this.host.editorChanged();
  }

  private onInput(): void {
    const ed = this.host.editor;
    this.pointSel = null;
    if (!ed.open) {
      const text = ed.value;
      if (text === "") return;
      this.startEdit("enter", text);
      return;
    }
    this.emitChange();
  }

  private onCompositionStart(): void {
    const ed = this.host.editor;
    if (!ed.open) {
      ed.el.value = "";
      this.startEdit("enter", "");
    }
    ed.composing = true;
  }

  // -------------------------------------------------------------------------
  // Keyboard
  // -------------------------------------------------------------------------

  private onKeyDown(e: KeyboardEvent): void {
    if (e.isComposing || e.keyCode === 229) return;
    const host = this.host;
    const ed = host.editor;
    const mods: Mods = { shift: e.shiftKey, mod: e.ctrlKey || e.metaKey, alt: e.altKey };

    if (ed.open) {
      this.onEditingKey(e, mods);
      return;
    }
    if (mods.mod && !mods.alt && (e.key === "c" || e.key === "C" || e.key === "x" || e.key === "X")) {
      ed.prepareClipboard();
      return;
    }
    if (mods.mod && (e.key === "v" || e.key === "V")) {
      this.plainPaste = mods.shift;
      return;
    }

    const sel = host.selection();
    const lim = host.limits();
    // Range-pick mode (no edit): arrows move/extend the pick, Enter settles it, Escape cancels.
    if (host.rangePick()) {
      if (e.key === "Enter") {
        e.preventDefault();
        host.pointRange({ ...sel.range }, "end");
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        host.cancelPick();
        return;
      }
    }
    if (lim.protectedNav && host.events().onNextUnlocked && (e.key === "Tab" || e.key === "Enter") && !mods.mod && !mods.alt) {
      // Protected sheet: the engine knows which cell is next (unlocked, visible, not covered).
      e.preventDefault();
      const dir: Dir = e.key === "Tab" ? (mods.shift ? "left" : "right") : mods.shift ? "up" : "down";
      const seq = ++this.edgeSeq;
      void this.stepProtected(sel, dir, lim).then((next) => {
        if (next && seq === this.edgeSeq) host.select(next, { emit: true, reveal: next.active });
      });
      return;
    }
    const action = reduceKey(sel, e.key, mods, lim);
    switch (action.type) {
      case "select": {
        e.preventDefault();
        const extending = mods.shift && e.key !== "Tab" && e.key !== "Enter";
        host.select(action.selection, {
          emit: true,
          reveal: extending ? extentCorner(action.selection) : action.selection.active,
        });
        if (host.rangePick()) host.pointRange({ ...action.selection.range }, "move");
        return;
      }
      case "navigateEdge": {
        e.preventDefault();
        const from = edgeOrigin(sel, action.extend);
        const seq = ++this.edgeSeq;
        const handler = host.events().onNavigateEdge;
        const p = handler ? handler(from, action.dir) : Promise.resolve(sheetEdge(from, action.dir, lim));
        p.then(
          (target) => {
            if (seq !== this.edgeSeq) return; // a newer jump superseded this one
            const next = applyEdge(host.selection(), target, action.extend, host.limits());
            host.select(next, { emit: true, reveal: action.extend ? extentCorner(next) : next.active });
          },
          () => undefined,
        );
        return;
      }
      case "startEdit":
        e.preventDefault();
        this.startEdit("edit");
        return;
      case "delete":
        e.preventDefault();
        host.events().onDeleteContents?.({ ...sel.range });
        return;
      case "none":
        return;
    }
  }

  private onEditingKey(e: KeyboardEvent, mods: Mods): void {
    const host = this.host;
    const ed = host.editor;
    if (host.events().onEditorKeyDown?.(e)) {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    const dir = DIRS[e.key];
    if (dir && ed.mode === "enter" && host.pointMode() && !mods.mod && !mods.alt) {
      // Point mode: arrows move (Shift extends) the reference being picked.
      e.preventDefault();
      const lim = host.limits();
      const base = this.pointSel ?? cellSelection(ed.cell ?? host.selection().active);
      const [dr, dc] = { up: [-1, 0], down: [1, 0], left: [0, -1], right: [0, 1] }[dir] as [number, number];
      const next = moveBy(base, dr, dc, mods.shift, lim);
      this.pointSel = next;
      host.pointRange({ ...next.range }, "end");
      host.reveal(extentCorner(next));
      return;
    }
    switch (e.key) {
      case "Enter":
        e.preventDefault();
        if (mods.alt || mods.mod) {
          ed.el.setRangeText("\n", ed.el.selectionStart, ed.el.selectionEnd, "end");
          this.emitChange();
          return;
        }
        this.lastCommitKey = "Enter";
        this.commit(mods.shift ? "up" : "down");
        return;
      case "Tab":
        // ⌃Tab / ⌘Tab: not for the editor; left alone so the native menu
        // (Window ▸ Show Next/Previous Tab) gets it.
        if (mods.mod) return;
        e.preventDefault();
        this.lastCommitKey = "Tab";
        this.commit(mods.shift ? "left" : "right");
        return;
      case "Escape":
        e.preventDefault();
        this.cancel();
        return;
      default: {
        const dir = DIRS[e.key];
        if (dir && ed.mode === "enter" && !mods.shift && !mods.mod && !mods.alt) {
          e.preventDefault();
          this.lastCommitKey = "";
          this.commit(dir);
        }
      }
    }
  }

  // -------------------------------------------------------------------------
  // Clipboard
  // -------------------------------------------------------------------------

  private onClipboard(e: ClipboardEvent, kind: "copy" | "cut" | "paste"): void {
    const ed = this.host.editor;
    if (ed.open) return; // native editing inside the textarea
    e.preventDefault();
    const ev = this.host.events();
    const data = e.clipboardData;
    if (kind === "copy") ev.onCopy?.(data);
    else if (kind === "cut") ev.onCut?.(data);
    else ev.onPaste?.(data, { plain: this.plainPaste });
    this.plainPaste = false;
    ed.resetIdleValue();
  }
}
