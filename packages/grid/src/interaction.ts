/**
 * Mouse, keyboard, editor and clipboard handling. All keyboard input arrives
 * through the focused editor textarea; selection logic is delegated to the
 * pure reducers in selection.ts.
 */
import type { CellPos } from "@tally/bridge";
import type { CellEditor } from "./editor";
import type { Geometry, Hit } from "./geometry";
import {
  applyEdge,
  cellSelection,
  edgeOrigin,
  extendTo,
  extentCorner,
  inRange,
  isWholeCols,
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
import type { CommitMove, Dir, GridEvents, Selection } from "./types";

/** Minimum column width a drag can set (zoom 100%). */
export const MIN_COL_WIDTH = 8;

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
  /** Live column resize preview (CSS px at zoom 100%). */
  previewColWidth(start: number, end: number, px: number): void;
  setHoverGutter(c: number | null): void;
  setResizeGuide(guide: { c: number; x: number } | null): void;
  invalidateOverlay(): void;
  /** Called when the editor opens/closes/grows. */
  editorChanged(): void;
}

type Drag =
  | { kind: "cells" }
  | { kind: "rows"; anchor: number }
  | { kind: "cols"; anchor: number }
  | { kind: "resize"; c: number; start: number; end: number; startX: number; startW: number; px: number };

const DIRS: Record<string, Dir> = { ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right" };

export class Interaction {
  private drag: Drag | null = null;
  private pointerId: number | null = null;
  private last = { x: 0, y: 0 };
  private autoScrollRaf = 0;
  private edgeSeq = 0;
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
    this.on(ta, "keyup", () => host.editor.resetIdleValue());
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

  private onPointerDown(e: PointerEvent): void {
    if (this.onScrollbar(e)) return;
    const host = this.host;
    const p = this.point(e);
    this.last = p;
    const geo = host.geometry();
    const lim = host.limits();
    const hit = geo.hit(p.x, p.y);
    const sel = host.selection();
    this.focus();

    if (e.button === 2) {
      if (hit.area === "cell" && !inRange(sel.range, hit)) {
        this.commitIfEditing("none");
        host.select(cellSelection(hit), { emit: true });
      }
      return;
    }
    if (e.button !== 0) return;
    if (hit.area !== "colGutter") this.commitIfEditing("none");

    switch (hit.area) {
      case "corner":
        host.select(selectAll(lim), { emit: true });
        return;
      case "colGutter": {
        const whole = isWholeCols(sel.range, lim) && hit.c >= sel.range.c0 && hit.c <= sel.range.c1;
        const start = whole ? sel.range.c0 : hit.c;
        const end = whole ? sel.range.c1 : hit.c;
        const startW = geo.colW(hit.c);
        this.drag = { kind: "resize", c: hit.c, start, end, startX: p.x, startW, px: geo.cols.sizeOf(hit.c) };
        host.setResizeGuide({ c: hit.c, x: geo.colX(hit.c) + startW });
        break;
      }
      case "colHeader": {
        const next =
          e.shiftKey ? selectCols(sel.anchor.c, hit.c, lim, sel.anchor.c, sel.anchor.r) : selectCols(hit.c, hit.c, lim, hit.c, geo.rowAt(geo.headerH));
        this.drag = { kind: "cols", anchor: e.shiftKey ? sel.anchor.c : hit.c };
        host.select(e.shiftKey ? { ...next, active: sel.active, anchor: sel.anchor } : next, { emit: true });
        break;
      }
      case "rowHeader": {
        const next =
          e.shiftKey ? selectRows(sel.anchor.r, hit.r, lim, sel.anchor.r, sel.anchor.c) : selectRows(hit.r, hit.r, lim, hit.r, geo.colAt(geo.headerW));
        this.drag = { kind: "rows", anchor: e.shiftKey ? sel.anchor.r : hit.r };
        host.select(e.shiftKey ? { ...next, active: sel.active, anchor: sel.anchor } : next, { emit: true });
        break;
      }
      case "cell": {
        const cell = { r: hit.r, c: hit.c };
        host.select(e.shiftKey ? extendTo(sel, cell) : cellSelection(cell), { emit: true });
        this.drag = { kind: "cells" };
        break;
      }
    }
    this.pointerId = e.pointerId;
    try {
      host.scroller.setPointerCapture(e.pointerId);
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
      const gutter = hit.area === "colGutter" ? hit.c : null;
      host.setHoverGutter(gutter);
      host.scroller.style.cursor = gutter !== null ? "col-resize" : "default";
      return;
    }
    if (this.drag.kind === "resize") {
      const geo = host.geometry();
      const d = this.drag;
      const w = Math.max(MIN_COL_WIDTH * geo.zoom, d.startW + (p.x - d.startX));
      d.px = Math.round(w / geo.zoom);
      host.previewColWidth(d.c, d.c, d.px);
      host.setResizeGuide({ c: d.c, x: host.geometry().colX(d.c) + w });
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
    let next: Selection | null = null;
    if (drag.kind === "cells") next = extendTo(sel, { r: geo.rowAt(y), c: geo.colAt(x) });
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
      host.previewColWidth(drag.start, drag.end, drag.px);
      host.events().onResize?.("col", drag.start, drag.end, drag.px);
      const p = this.point(e);
      const hit = host.geometry().hit(p.x, p.y);
      host.setHoverGutter(hit.area === "colGutter" ? hit.c : null);
    }
  }

  private onDoubleClick(e: MouseEvent): void {
    if (this.onScrollbar(e)) return;
    const p = this.point(e);
    const hit: Hit = this.host.geometry().hit(p.x, p.y);
    if (hit.area !== "cell") return;
    e.preventDefault();
    this.startEdit("edit");
  }

  private onContextMenu(e: MouseEvent): void {
    if (this.onScrollbar(e)) return;
    e.preventDefault();
    const p = this.point(e);
    const geo = this.host.geometry();
    const hit = geo.hit(p.x, p.y);
    const sel = this.host.selection();
    let cell: CellPos = sel.active;
    if (hit.area === "cell") {
      cell = { r: hit.r, c: hit.c };
      if (!inRange(sel.range, cell)) this.host.select(cellSelection(cell), { emit: true });
    } else if (hit.area === "colHeader") cell = { r: sel.active.r, c: hit.c };
    else if (hit.area === "rowHeader") cell = { r: hit.r, c: sel.active.c };
    this.host.events().onContextMenu?.(cell, e.clientX, e.clientY);
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
    host.events().onCommitEdit?.(move);
    ed.end();
    host.editorChanged();
    if (move === "none") return;
    const sel = host.selection();
    const lim = host.limits();
    const next = move === "down" || move === "up" || move === "left" || move === "right" ? this.moveAfterCommit(sel, move, lim) : sel;
    host.select(next, { emit: true, reveal: next.active });
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

  private emitChange(): void {
    const ed = this.host.editor;
    if (!ed.open) return;
    const text = ed.value;
    ed.noteEmitted(text);
    this.host.events().onEditChange?.(text, ed.caret);
    this.host.editorChanged();
  }

  private onInput(): void {
    const ed = this.host.editor;
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
    if (mods.mod && (e.key === "v" || e.key === "V")) return;

    const sel = host.selection();
    const lim = host.limits();
    const action = reduceKey(sel, e.key, mods, lim);
    switch (action.type) {
      case "select": {
        e.preventDefault();
        const extending = mods.shift && e.key !== "Tab" && e.key !== "Enter";
        host.select(action.selection, {
          emit: true,
          reveal: extending ? extentCorner(action.selection) : action.selection.active,
        });
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
    const ed = this.host.editor;
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
    else ev.onPaste?.(data);
    ed.resetIdleValue();
  }
}
