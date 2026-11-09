/**
 * Object overlay layer (wave 5 seam, W5-0b): an absolutely positioned HTML
 * layer over the grid's scrolling pane that scrolls with the canvas and hosts
 * cell-anchored objects (charts, pictures). W5-5 mounts ECharts instances
 * into `elementFor(id)` and implements drag/resize on top of the handle
 * events; this module only owns placement, selection handles and hit tests.
 *
 * - Anchors are `ChartAnchor`s: a cell plus mm offsets and a size in mm
 *   (`PX_PER_MM` at zoom 100%). {@link anchorToPx} / {@link pxToAnchor}
 *   convert between an anchor and content pixels (CSS px, zoomed, measured
 *   from the sheet's origin, headers excluded).
 * - The layer itself has `pointer-events: none`, so clicks outside an object
 *   fall through to the scroller and select cells; only object nodes (and
 *   their handles) take pointer events.
 * - Objects live in the scrolling pane only (not in frozen panes): the layer
 *   is clipped to the pane below/right of the frozen rows/columns.
 */
import type { RangeDto } from "@tally/bridge";
import type { Geometry, Rect } from "./geometry";
import type { AxisModel } from "./layout";

/** CSS px per mm at zoom 100% (96 dpi). */
export const PX_PER_MM = 96 / 25.4;

/** A cell-anchored object placement (same fields as the bridge `ChartAnchor`). */
export interface ObjectAnchor {
  r: number;
  c: number;
  dxMm: number;
  dyMm: number;
  widthMm: number;
  heightMm: number;
}

/** What the anchor maths needs from the grid. */
export interface AnchorAxes {
  rows: Pick<AxisModel, "indexToPx" | "pxToIndex" | "count">;
  cols: Pick<AxisModel, "indexToPx" | "pxToIndex" | "count">;
  /** 1 = 100%. */
  zoom: number;
}

/** Content rect (CSS px, zoomed, from the sheet origin) of an anchor. */
export function anchorToPx(a: ObjectAnchor, axes: AnchorAxes): Rect {
  const k = PX_PER_MM * axes.zoom;
  return {
    x: axes.cols.indexToPx(a.c) * axes.zoom + a.dxMm * k,
    y: axes.rows.indexToPx(a.r) * axes.zoom + a.dyMm * k,
    w: a.widthMm * k,
    h: a.heightMm * k,
  };
}

/**
 * The anchor of a content rect: the cell under its top-left corner and the
 * mm offsets inside it (inverse of {@link anchorToPx}).
 */
export function pxToAnchor(rect: Rect, axes: AnchorAxes): ObjectAnchor {
  const k = PX_PER_MM * axes.zoom;
  const z = axes.zoom > 0 ? axes.zoom : 1;
  const x = Math.max(0, rect.x);
  const y = Math.max(0, rect.y);
  const c = Math.min(axes.cols.count - 1, Math.max(0, axes.cols.pxToIndex(x / z)));
  const r = Math.min(axes.rows.count - 1, Math.max(0, axes.rows.pxToIndex(y / z)));
  return {
    r,
    c,
    dxMm: (x - axes.cols.indexToPx(c) * z) / k,
    dyMm: (y - axes.rows.indexToPx(r) * z) / k,
    widthMm: Math.max(0, rect.w) / k,
    heightMm: Math.max(0, rect.h) / k,
  };
}

/** The cells an object covers (for repaint/refetch bookkeeping). */
export function anchorRange(a: ObjectAnchor, axes: AnchorAxes): RangeDto {
  const px = anchorToPx(a, axes);
  const z = axes.zoom > 0 ? axes.zoom : 1;
  const c1 = Math.max(a.c, axes.cols.pxToIndex((px.x + Math.max(0, px.w - 1)) / z));
  const r1 = Math.max(a.r, axes.rows.pxToIndex((px.y + Math.max(0, px.h - 1)) / z));
  return { r0: a.r, c0: a.c, r1, c1 };
}

/** An object the layer places. `kind` is free-form ("chart", "image"). */
export interface GridObject {
  id: string;
  kind: string;
  anchor: ObjectAnchor;
  /** Accessible name (the chart title / picture name). */
  label?: string;
}

/** The 8 resize handles, clockwise from the top-left corner. */
export type ObjectHandle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
export const OBJECT_HANDLES: readonly ObjectHandle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

/** Callbacks of the layer (all optional). */
export interface ObjectLayerEvents {
  /** The selected object changed (`null` = none). */
  onObjectSelect?(id: string | null): void;
  /**
   * Pointer down on an object's body or one of its handles (the layer has
   * already selected it). W5-5 starts a move/resize drag from here.
   */
  onObjectPointerDown?(id: string, part: "body" | ObjectHandle, e: PointerEvent): void;
  /** Double-click on an object (open the editor). */
  onObjectDoubleClick?(id: string): void;
  /** Delete / Backspace / Escape while an object is selected. */
  onObjectKey?(id: string, key: string, e: KeyboardEvent): void;
}

/**
 * Tools ▸ Options ▸ View: how selected objects are marked (AOO "Simple
 * handles", "Large handles", "Anchor").
 */
export interface ObjectLayerOptions {
  /** Flat handles (AOO "Simple handles"); off = shaded ones. Default `false`. */
  simpleHandles: boolean;
  /** 9 px handles (AOO "Large handles", on by default); off = 7 px. */
  bigHandles: boolean;
  /** Mark the anchor cell of the selected object (AOO "Anchor", on by default). */
  showAnchor: boolean;
}

export const DEFAULT_OBJECT_LAYER_OPTIONS: ObjectLayerOptions = { simpleHandles: false, bigHandles: true, showAnchor: true };

/** Handle edge in px for AOO's large / normal handles. */
export const BIG_HANDLE_SIZE = 9;
export const SMALL_HANDLE_SIZE = 7;

/** The anchor marker (an anchor glyph, 12 × 12). Static markup, no input. */
const ANCHOR_SVG =
  '<svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><circle cx="6" cy="2.2" r="1.3"/><path d="M6 3.5v7.3M3.6 5.4h4.8M1.6 7.4c.6 2 2.3 3.4 4.4 3.4s3.8-1.4 4.4-3.4"/></g></svg>';

export class ObjectLayer {
  /** The clipped layer (sized to the scrolling pane). */
  readonly el: HTMLDivElement;
  /** Scrolls with the content (a CSS translate); object nodes are its children. */
  readonly inner: HTMLDivElement;
  private readonly nodes = new Map<string, HTMLDivElement>();
  private objects: readonly GridObject[] = [];
  private selectedId: string | null = null;
  private axes: AnchorAxes | null = null;
  private events: ObjectLayerEvents;
  private options: ObjectLayerOptions = DEFAULT_OBJECT_LAYER_OPTIONS;
  /** The anchor marker of the selected object (when shown). */
  private anchorEl: HTMLDivElement | null = null;
  /** Guide lines while an object is dragged (Tools ▸ Options ▸ View ▸ Guides while moving). */
  private guideEls: HTMLDivElement[] = [];
  private guideRect: Rect | null = null;
  /** The content translation of `inner` (px). */
  private tx = 0;
  private ty = 0;

  /** Sets how selected objects are marked (re-marks the selection). */
  setOptions(options: ObjectLayerOptions): void {
    const o = this.options;
    if (o.simpleHandles === options.simpleHandles && o.bigHandles === options.bigHandles && o.showAnchor === options.showAnchor) return;
    this.options = { ...options };
    const id = this.selectedId;
    const node = id === null ? undefined : this.nodes.get(id);
    if (node && id !== null) {
      node.querySelectorAll("[data-handle]").forEach((h) => h.remove());
      this.addHandles(node, id);
    }
    this.placeAnchor();
  }

  /**
   * Shows guide lines along the edges of `rect` (content px, as the object
   * nodes are placed) across the whole pane, or hides them (`null`).
   */
  setGuides(rect: Rect | null): void {
    this.guideRect = rect;
    if (!rect) {
      for (const g of this.guideEls) g.remove();
      this.guideEls = [];
      return;
    }
    const doc = this.el.ownerDocument;
    while (this.guideEls.length < 4) {
      const g = doc.createElement("div");
      g.className = "tally-grid-guide";
      g.dataset.guide = String(this.guideEls.length);
      g.setAttribute("aria-hidden", "true");
      Object.assign(g.style, { position: "absolute", pointerEvents: "none", zIndex: "2" } satisfies Partial<CSSStyleDeclaration>);
      this.el.appendChild(g);
      this.guideEls.push(g);
    }
    this.placeGuides();
  }

  private placeGuides(): void {
    const r = this.guideRect;
    if (!r) return;
    const line = "1px dashed var(--tally-selection-border, #2563eb)";
    this.guideEls.forEach((g, i) => {
      const s = g.style;
      const vertical = i < 2;
      if (vertical) {
        s.left = `${(i === 0 ? r.x : r.x + r.w) + this.tx}px`;
        s.top = "0";
        s.width = "0";
        s.height = "100%";
        s.borderLeft = line;
        s.borderTop = "";
      } else {
        s.top = `${(i === 2 ? r.y : r.y + r.h) + this.ty}px`;
        s.left = "0";
        s.height = "0";
        s.width = "100%";
        s.borderTop = line;
        s.borderLeft = "";
      }
    });
  }

  /** Places (or removes) the anchor marker of the selected object. */
  private placeAnchor(): void {
    const o = this.selectedId === null ? undefined : this.objects.find((x) => x.id === this.selectedId);
    const axes = this.axes;
    if (!o || !axes || !this.options.showAnchor) {
      this.anchorEl?.remove();
      this.anchorEl = null;
      return;
    }
    if (!this.anchorEl) {
      const a = this.el.ownerDocument.createElement("div");
      a.className = "tally-grid-anchor";
      a.setAttribute("aria-hidden", "true");
      a.innerHTML = ANCHOR_SVG;
      Object.assign(a.style, {
        position: "absolute",
        width: "12px",
        height: "12px",
        pointerEvents: "none",
        color: "var(--tally-selection-border, #2563eb)",
        zIndex: "2",
      } satisfies Partial<CSSStyleDeclaration>);
      this.inner.appendChild(a);
      this.anchorEl = a;
    }
    this.anchorEl.dataset.anchorCell = `${o.anchor.r},${o.anchor.c}`;
    this.anchorEl.style.left = `${axes.cols.indexToPx(o.anchor.c) * axes.zoom}px`;
    this.anchorEl.style.top = `${axes.rows.indexToPx(o.anchor.r) * axes.zoom}px`;
  }

  /** Relabels the group (translated `aria-label`). */
  setLabel(label: string): void {
    this.el.setAttribute("aria-label", label);
  }

  constructor(doc: Document, events: ObjectLayerEvents = {}) {
    this.events = events;
    const el = doc.createElement("div");
    el.className = "tally-grid-objects";
    el.setAttribute("role", "group");
    el.setAttribute("aria-label", "Objects");
    Object.assign(el.style, {
      position: "absolute",
      left: "0",
      top: "0",
      width: "0",
      height: "0",
      overflow: "hidden",
      pointerEvents: "none",
      zIndex: "3",
    } satisfies Partial<CSSStyleDeclaration>);
    const inner = doc.createElement("div");
    Object.assign(inner.style, { position: "absolute", left: "0", top: "0", pointerEvents: "none" } satisfies Partial<CSSStyleDeclaration>);
    el.appendChild(inner);
    this.el = el;
    this.inner = inner;
  }

  /**
   * Appends the layer to `host`: the grid's container, as a sibling *after*
   * the grid root (not inside it), so object nodes are not "in the grid" for
   * the app's cell shortcuts (`closest(".tally-grid")`): a selected chart
   * owns Delete, Escape and the arrows through `onObjectKey`.
   */
  mount(host: HTMLElement): void {
    host.appendChild(this.el);
  }

  setEvents(events: ObjectLayerEvents): void {
    this.events = events;
  }

  get selected(): string | null {
    return this.selectedId;
  }

  /** The objects shown, by id. */
  list(): readonly GridObject[] {
    return this.objects;
  }

  /** The host element of an object (mount a chart renderer in it), if shown. */
  elementFor(id: string): HTMLDivElement | undefined {
    return this.nodes.get(id);
  }

  /** Replaces the object list; nodes are reused by id. */
  setObjects(objects: readonly GridObject[]): void {
    this.objects = objects;
    const keep = new Set(objects.map((o) => o.id));
    for (const [id, node] of this.nodes) {
      if (!keep.has(id)) {
        node.remove();
        this.nodes.delete(id);
      }
    }
    for (const o of objects) {
      let node = this.nodes.get(o.id);
      if (!node) {
        node = this.makeNode(o.id);
        this.nodes.set(o.id, node);
        this.inner.appendChild(node);
      }
      node.dataset.kind = o.kind;
      node.setAttribute("aria-label", o.label ?? o.kind);
    }
    if (this.selectedId !== null && !keep.has(this.selectedId)) this.select(null);
    this.place();
  }

  /** Selects an object (shows its handles) or clears the selection. */
  select(id: string | null, emit = true): void {
    if (id !== null && !this.nodes.has(id)) id = null;
    if (id === this.selectedId) return;
    const prev = this.selectedId === null ? undefined : this.nodes.get(this.selectedId);
    if (prev) {
      prev.dataset.selected = "false";
      prev.setAttribute("aria-selected", "false");
      prev.style.outline = "none";
      prev.querySelectorAll("[data-handle]").forEach((h) => h.remove());
    }
    this.selectedId = id;
    const node = id === null ? undefined : this.nodes.get(id);
    if (node) {
      node.dataset.selected = "true";
      node.setAttribute("aria-selected", "true");
      // A visible frame around the selected chart/picture (theme accent).
      node.style.outline = "2px solid var(--tally-selection-border, #2563eb)";
      node.style.outlineOffset = "-1px";
      this.addHandles(node, id as string);
      node.focus({ preventScroll: true });
    }
    this.placeAnchor();
    if (emit) this.events.onObjectSelect?.(id);
  }

  /** Re-places every node for the geometry (called once per frame/scroll). */
  sync(geo: Geometry): void {
    this.axes = { rows: geo.rows, cols: geo.cols, zoom: geo.zoom };
    const left = geo.headerW + geo.frozenW;
    const top = geo.headerH + geo.frozenH;
    const s = this.el.style;
    s.left = `${left}px`;
    s.top = `${top}px`;
    s.width = `${Math.max(0, geo.width - left)}px`;
    s.height = `${Math.max(0, geo.height - top)}px`;
    this.tx = -(geo.frozenW + geo.scrollX);
    this.ty = -(geo.frozenH + geo.scrollY);
    this.inner.style.transform = `translate(${this.tx}px, ${this.ty}px)`;
    this.place();
    this.placeGuides();
  }

  /** The object under a canvas point (CSS px from the grid's top-left), topmost first. */
  hitTest(x: number, y: number): string | null {
    const axes = this.axes;
    if (!axes) return null;
    const left = parseFloat(this.el.style.left) || 0;
    const top = parseFloat(this.el.style.top) || 0;
    if (x < left || y < top) return null;
    const m = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(this.inner.style.transform);
    const tx = m ? parseFloat(m[1] ?? "0") : 0;
    const ty = m ? parseFloat(m[2] ?? "0") : 0;
    const cx = x - left - tx;
    const cy = y - top - ty;
    for (let i = this.objects.length - 1; i >= 0; i--) {
      const o = this.objects[i];
      if (!o) continue;
      const r = anchorToPx(o.anchor, axes);
      if (cx >= r.x && cx <= r.x + r.w && cy >= r.y && cy <= r.y + r.h) return o.id;
    }
    return null;
  }

  destroy(): void {
    this.nodes.clear();
    this.el.remove();
  }

  private place(): void {
    const axes = this.axes;
    if (!axes) return;
    for (const o of this.objects) {
      const node = this.nodes.get(o.id);
      if (!node) continue;
      const r = anchorToPx(o.anchor, axes);
      const s = node.style;
      s.left = `${r.x}px`;
      s.top = `${r.y}px`;
      s.width = `${r.w}px`;
      s.height = `${r.h}px`;
    }
    this.placeAnchor();
  }

  private makeNode(id: string): HTMLDivElement {
    const doc = this.el.ownerDocument;
    const node = doc.createElement("div");
    node.className = "tally-grid-object";
    node.dataset.objectId = id;
    node.dataset.selected = "false";
    node.tabIndex = -1;
    node.setAttribute("role", "img");
    Object.assign(node.style, {
      position: "absolute",
      boxSizing: "border-box",
      pointerEvents: "auto",
      outline: "none",
    } satisfies Partial<CSSStyleDeclaration>);
    node.addEventListener("pointerdown", (e) => {
      const handle = (e.target as HTMLElement | null)?.dataset.handle as ObjectHandle | undefined;
      this.select(id);
      this.events.onObjectPointerDown?.(id, handle ?? "body", e);
      e.stopPropagation();
    });
    node.addEventListener("dblclick", (e) => {
      e.stopPropagation();
      this.events.onObjectDoubleClick?.(id);
    });
    node.addEventListener("keydown", (e) => {
      if (e.key === "Delete" || e.key === "Backspace" || e.key === "Escape" || e.key.startsWith("Arrow")) {
        this.events.onObjectKey?.(id, e.key, e);
      }
    });
    return node;
  }

  private addHandles(node: HTMLDivElement, id: string): void {
    const doc = node.ownerDocument;
    const { simpleHandles, bigHandles } = this.options;
    const size = bigHandles ? BIG_HANDLE_SIZE : SMALL_HANDLE_SIZE;
    for (const h of OBJECT_HANDLES) {
      const el = doc.createElement("div");
      el.dataset.handle = h;
      el.dataset.objectId = id;
      el.dataset.handleStyle = simpleHandles ? "simple" : "shaded";
      el.setAttribute("aria-hidden", "true");
      const s = el.style;
      s.position = "absolute";
      s.width = `${size}px`;
      s.height = `${size}px`;
      s.boxSizing = "border-box";
      s.pointerEvents = "auto";
      s.cursor = `${h}-resize`;
      // Square handles above the chart canvas: flat (AOO "Simple handles")
      // or shaded with a light rim.
      s.background = simpleHandles ? "var(--tally-bg, #ffffff)" : "linear-gradient(135deg, #ffffff, #dbe4f3)";
      s.border = "1.5px solid var(--tally-selection-border, #2563eb)";
      s.borderRadius = simpleHandles ? "0" : "2px";
      s.boxShadow = simpleHandles ? "none" : "0 1px 2px rgba(0, 0, 0, 0.35)";
      s.zIndex = "1";
      const half = -size / 2;
      if (h.includes("n")) s.top = `${half}px`;
      if (h.includes("s")) s.bottom = `${half}px`;
      if (h.includes("w")) s.left = `${half}px`;
      if (h.includes("e")) s.right = `${half}px`;
      if (h === "n" || h === "s") s.left = `calc(50% + ${half}px)`;
      if (h === "e" || h === "w") s.top = `calc(50% + ${half}px)`;
      node.appendChild(el);
    }
  }
}
