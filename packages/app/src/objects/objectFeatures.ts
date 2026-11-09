/**
 * Cell-anchored objects on the grid (W5-5): keeps the chart/picture lists
 * of the active sheet in `useChartStore`, handles the object layer's events
 * (select, drag to move/resize, arrow nudges, Delete, double-click) and
 * Insert ▸ Image. Charts' own commands live in `charts/chartFeatures.ts`.
 *
 * A drag previews with the node's CSS box and sends exactly one
 * `moveResizeChart` / `moveResizeImage` on pointer-up.
 */
import type { ApplyResult, ChartAnchor, ChartDataDto, ChartDto, ImageDto, Op } from "@tally/bridge";
import { PX_PER_MM, pxToAnchor, type ObjectHandle } from "@tally/grid";
import type { AppController } from "../controller";
import { nodeVisible } from "../charts/nodeVisible";
import { objectId, parseObjectId, useChartStore } from "../charts/chartStore";
import { useUiStore } from "../stores/uiStore";
import { useW56Store } from "../data/store";
import { anchorChanged, dragRect, nudgeAnchor, roundAnchor, type Rect } from "./objectDrag";
import type { W5Shell } from "./shell";
import { clientV4 } from "./shell";
import { t } from "../i18n";

/** Charts whose data is fetched at once after a refresh. */
export const DATA_FETCH_CAP = 50;
/** Charts fetched per refresh (only those in the viewport), and requests in flight at once. */
export const MAX_CHART_FETCH = 20;
export const MAX_FETCHES_IN_FLIGHT = 4;
/** Coalescing window for `chartsChanged` refreshes. */
export const REFRESH_DEBOUNCE_MS = 120;
/** Widest default size of an inserted picture (mm). */
export const MAX_IMAGE_WIDTH_MM = 160;

interface Drag {
  id: string;
  part: "body" | ObjectHandle;
  node: HTMLElement;
  start: Rect;
  x0: number;
  y0: number;
  pointerId: number;
  moved: boolean;
}

export class ObjectFeatures {
  private drag: Drag | null = null;
  private refreshSeq = 0;

  constructor(private readonly ctl: AppController) {}

  private get shell(): W5Shell {
    return this.ctl.shell as unknown as W5Shell;
  }

  private toast(kind: "info" | "error", message: string): void {
    useUiStore.getState().pushToast(kind, message);
  }

  // -- lists --------------------------------------------------------------------

  /** Re-reads the charts and pictures of the active sheet, then their data. */
  async refreshObjects(): Promise<void> {
    const info = this.ctl.info;
    if (!info) return;
    const seq = ++this.refreshSeq;
    const sheet = this.ctl.sheet;
    const doc = info.doc;
    const [charts, images] = await Promise.all([
      clientV4(this.ctl).charts(doc, sheet).catch((): ChartDto[] => []),
      clientV4(this.ctl).images(doc, sheet).catch((): ImageDto[] => []),
    ]);
    if (seq !== this.refreshSeq || sheet !== this.ctl.sheet || this.ctl.info?.doc !== doc) return;
    this.stale.clear();
    useChartStore.getState().setObjects(sheet, charts, images);
    void this.loadImageData(doc, sheet, images);
    await this.refreshData(charts.slice(0, DATA_FETCH_CAP).map((c) => c.id));
  }

  /** `(doc, sheet, id)` of picture bytes being fetched. */
  private readonly imageFetches = new Set<string>();

  /**
   * Pictures from files come without bytes (`images()` leaves `data` empty):
   * fetch each one once with `image_data` and cache it as a data URL in the
   * chart store (until the sheet or document changes).
   */
  private async loadImageData(doc: number, sheet: number, images: readonly ImageDto[]): Promise<void> {
    const have = useChartStore.getState().imageData;
    await Promise.all(
      images
        .filter((im) => !im.data && have[im.id] === undefined)
        .map(async (im) => {
          const key = `${doc}:${sheet}:${im.id}`;
          if (this.imageFetches.has(key)) return;
          this.imageFetches.add(key);
          try {
            const data = await clientV4(this.ctl).imageData(doc, sheet, im.id);
            const st = useChartStore.getState();
            if (data && this.ctl.info?.doc === doc && this.ctl.sheet === sheet && st.sheet === sheet && st.images.some((x) => x.id === im.id)) st.setImageData(im.id, `data:${im.mime || "image/png"};base64,${data}`);
          } catch {
            // Gone or unsupported: the frame keeps its placeholder.
          } finally {
            this.imageFetches.delete(key);
          }
        }),
    );
  }

  /** Chart ids whose data is stale because they were off screen when the sheet changed. */
  private readonly stale = new Set<number>();
  private pending = new Set<number>();
  private flushTimer: ReturnType<typeof setTimeout> | null = null;
  private waiters: (() => void)[] = [];

  /** Whether chart `id` intersects the primary grid's viewport (true when that cannot be told). */
  private isVisible(id: number): boolean {
    const grid = this.ctl.grid;
    if (!grid) return true;
    try {
      const rect = grid.objectRect(objectId("chart", id));
      return rect ? nodeVisible(rect, grid.getGeometry()) : true;
    } catch {
      return true;
    }
  }

  /**
   * Debounced `refreshData` for engine results: one flush per {@link REFRESH_DEBOUNCE_MS}
   * however many `chartsChanged` arrive (a hostile file can list every chart after each edit).
   */
  scheduleRefresh(ids: readonly number[]): Promise<void> {
    for (const id of ids) this.pending.add(id);
    return new Promise((resolve) => {
      this.waiters.push(resolve);
      if (this.flushTimer !== null) return;
      this.flushTimer = setTimeout(() => {
        this.flushTimer = null;
        const batch = [...this.pending];
        const done = this.waiters;
        this.pending = new Set();
        this.waiters = [];
        void this.refreshData(batch).finally(() => done.forEach((w) => w()));
      }, REFRESH_DEBOUNCE_MS);
    });
  }

  /** Fetches the data of stale charts that scrolled into view. */
  fetchStaleVisible(): void {
    if (this.stale.size === 0) return;
    const ids = [...this.stale].filter((id) => this.isVisible(id));
    if (ids.length > 0) void this.refreshData(ids);
  }

  /**
   * Re-fetches `chart_data` for charts of the active sheet: only charts in the
   * viewport (the rest are marked stale and fetched when they scroll in), at
   * most {@link MAX_CHART_FETCH} per call and {@link MAX_FETCHES_IN_FLIGHT} at once.
   */
  async refreshData(ids: readonly number[]): Promise<void> {
    const info = this.ctl.info;
    if (!info || ids.length === 0) return;
    const sheet = this.ctl.sheet;
    const wanted: number[] = [];
    for (const id of new Set(ids)) {
      if (wanted.length < MAX_CHART_FETCH && this.isVisible(id)) {
        wanted.push(id);
        this.stale.delete(id);
      } else this.stale.add(id);
    }
    let next = 0;
    const worker = async (): Promise<void> => {
      for (;;) {
        const id = wanted[next++];
        if (id === undefined) return;
        try {
          const data = await clientV4(this.ctl).chartData(info.doc, sheet, id);
          if (sheet === this.ctl.sheet && useChartStore.getState().sheet === sheet) useChartStore.getState().setData(id, data);
        } catch {
          // Unsupported or gone: the chart shows empty.
          if (sheet === this.ctl.sheet) useChartStore.getState().setData(id, { chart: { sheet, id }, version: 0, categories: [], series: [], truncated: false, problems: [] } satisfies ChartDataDto);
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(MAX_FETCHES_IN_FLIGHT, wanted.length) }, worker));
  }

  /** After an engine result: re-read objects or data as the result says. */
  afterResult(res: ApplyResult, ops: readonly Op[] | null): void {
    const sheet = this.ctl.sheet;
    const objectOps: Op["type"][] = ["addChart", "updateChart", "setChartDataTable", "moveResizeChart", "deleteChart", "moveChartToSheet", "addImage", "deleteImage", "moveResizeImage"];
    // Undo/redo and `doc-changed` carry no ops: re-read the lists (cheap) in case an object op was undone.
    const touched = ops === null ? true : ops.some((o) => objectOps.includes(o.type));
    if (touched || (res.objectsChanged ?? []).includes(sheet)) {
      void this.refreshObjects();
      return;
    }
    const changed = (res.chartsChanged ?? []).filter((c) => c.sheet === sheet).map((c) => c.id);
    if (changed.length > 0) void this.scheduleRefresh(changed);
    else if (res.changed.some((c) => c.sheet === sheet) && useChartStore.getState().charts.length > 0 && !res.chartsChanged) {
      // An engine without `chartsChanged` (the mock, older builds): refresh every chart.
      void this.scheduleRefresh(useChartStore.getState().charts.map((c) => c.id));
    }
  }

  // -- layer events -------------------------------------------------------------

  onObjectSelect = (id: string | null): void => {
    useChartStore.getState().select(id);
  };

  onObjectDoubleClick = (id: string): void => {
    const p = parseObjectId(id);
    if (p?.kind === "chart") {
      useChartStore.getState().select(id);
      useChartStore.getState().setEditorOpen(true);
    }
  };

  onObjectKey = (id: string, key: string, e: KeyboardEvent): void => {
    const grid = this.ctl.grid;
    if (key === "Escape") {
      grid?.objectLayer.select(null);
      this.ctl.focusGrid();
      e.preventDefault();
      return;
    }
    if (key === "Delete" || key === "Backspace") {
      e.preventDefault();
      void this.deleteObject(id);
      return;
    }
    if (key.startsWith("Arrow") && grid) {
      const o = grid.objectLayer.list().find((x) => x.id === id);
      if (!o) return;
      const next = nudgeAnchor(o.anchor, key, e.shiftKey, grid.anchorAxes());
      if (next) {
        e.preventDefault();
        void this.moveResize(id, next);
      }
    }
  };

  /** Starts a move (body) or resize (handle) drag; one op on pointer-up. */
  onObjectPointerDown = (id: string, part: "body" | ObjectHandle, e: PointerEvent): void => {
    if (e.button !== 0) return;
    const grid = this.ctl.grid;
    const node = grid?.objectLayer.elementFor(id);
    const start = grid?.objectRect(id);
    if (!grid || !node || !start) return;
    this.drag = { id, part, node, start, x0: e.clientX, y0: e.clientY, pointerId: e.pointerId, moved: false };
    try {
      node.setPointerCapture(e.pointerId);
    } catch {
      // Synthetic events in tests have no active pointer.
    }
    node.addEventListener("pointermove", this.onDragMove);
    node.addEventListener("pointerup", this.onDragUp);
    node.addEventListener("pointercancel", this.onDragUp);
    node.style.cursor = part === "body" ? "move" : `${part}-resize`;
  };

  private readonly onDragMove = (e: PointerEvent): void => {
    const d = this.drag;
    if (!d) return;
    const zoom = this.ctl.grid?.anchorAxes().zoom ?? 1;
    void zoom;
    const rect = dragRect(d.start, d.part, e.clientX - d.x0, e.clientY - d.y0);
    if (Math.abs(e.clientX - d.x0) + Math.abs(e.clientY - d.y0) > 2) d.moved = true;
    Object.assign(d.node.style, { left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.w}px`, height: `${rect.h}px` });
    // Tools ▸ Options ▸ View ▸ Guides while moving: lines from the object's edges across the sheet.
    if (d.moved && useW56Store.getState().prefs.view.helpLines) this.ctl.grid?.objectLayer.setGuides(rect);
  };

  private readonly onDragUp = (e: PointerEvent): void => {
    const d = this.drag;
    if (!d) return;
    this.drag = null;
    d.node.removeEventListener("pointermove", this.onDragMove);
    d.node.removeEventListener("pointerup", this.onDragUp);
    d.node.removeEventListener("pointercancel", this.onDragUp);
    d.node.style.cursor = "";
    try {
      d.node.releasePointerCapture(d.pointerId);
    } catch {
      // ignore
    }
    const grid = this.ctl.grid;
    grid?.objectLayer.setGuides(null);
    if (!grid || e.type === "pointercancel" || !d.moved) {
      Object.assign(d.node.style, { left: `${d.start.x}px`, top: `${d.start.y}px`, width: `${d.start.w}px`, height: `${d.start.h}px` });
      return;
    }
    const rect = dragRect(d.start, d.part, e.clientX - d.x0, e.clientY - d.y0);
    const next = pxToAnchor(rect, grid.anchorAxes());
    void this.moveResize(d.id, next);
  };

  /** Sends `moveResizeChart` / `moveResizeImage` when the anchor changed. */
  async moveResize(id: string, anchor: ChartAnchor): Promise<ApplyResult | null> {
    const p = parseObjectId(id);
    const cur = useChartStore.getState();
    const current = p?.kind === "chart" ? cur.charts.find((c) => c.id === p.id)?.anchor : p?.kind === "image" ? cur.images.find((i) => i.id === p.id)?.anchor : undefined;
    if (!p || !current || !anchorChanged(current, anchor)) {
      // Snap the preview back to the stored anchor.
      this.ctl.grid?.objectLayer.setObjects([...this.ctl.grid.objectLayer.list()]);
      return null;
    }
    const a = roundAnchor(anchor);
    const resized = a.widthMm !== roundAnchor(current).widthMm || a.heightMm !== roundAnchor(current).heightMm;
    const label = p.kind === "chart" ? (resized ? "Resize chart" : "Move chart") : resized ? "Resize picture" : "Move picture";
    const op: Op = p.kind === "chart" ? { type: "moveResizeChart", sheet: this.ctl.sheet, id: p.id, anchor: a } : { type: "moveResizeImage", sheet: this.ctl.sheet, id: p.id, anchor: a };
    const res = await this.ctl.apply(label, [op]);
    if (res) {
      // Keep it selected across the refresh.
      useChartStore.getState().select(id);
    }
    return res;
  }

  /** Deletes a chart or picture by layer id. */
  async deleteObject(id: string = useChartStore.getState().selected ?? ""): Promise<void> {
    const p = parseObjectId(id);
    if (!p) return;
    const op: Op = p.kind === "chart" ? { type: "deleteChart", sheet: this.ctl.sheet, id: p.id } : { type: "deleteImage", sheet: this.ctl.sheet, id: p.id };
    useChartStore.getState().select(null);
    await this.ctl.apply(p.kind === "chart" ? t("cmd.chart.delete", "Delete chart") : t("msg.undo.deletePicture", "Delete picture"), [op]);
    this.ctl.focusGrid();
  }

  /** The anchor for a new object at the active cell, sized in mm. */
  anchorAtActive(widthMm: number, heightMm: number): ChartAnchor {
    const a = this.ctl.selection.active;
    return { r: a.r, c: a.c, dxMm: 2, dyMm: 2, widthMm, heightMm };
  }

  // -- pictures -----------------------------------------------------------------

  /** Insert ▸ Image…: a native file dialog (desktop) or an `<input type=file>` (browser). */
  async insertImage(): Promise<void> {
    try {
      const picked = this.shell.pickImage ? await this.shell.pickImage() : await pickImageInBrowser();
      if (!picked) return;
      const wPx = picked.width ?? 400;
      const hPx = picked.height ?? 300;
      let widthMm = wPx / PX_PER_MM;
      let heightMm = hPx / PX_PER_MM;
      if (widthMm > MAX_IMAGE_WIDTH_MM) {
        heightMm *= MAX_IMAGE_WIDTH_MM / widthMm;
        widthMm = MAX_IMAGE_WIDTH_MM;
      }
      const image: ImageDto = { id: 0, name: picked.name, anchor: this.anchorAtActive(widthMm, heightMm), mime: picked.mime, data: picked.data };
      const res = await this.ctl.apply(t("msg.undo.insertPicture", "Insert picture"), [{ type: "addImage", sheet: this.ctl.sheet, image }]);
      const id = res?.created?.[0]?.id;
      if (res && res.errors.length === 0 && id !== undefined) {
        useChartStore.getState().setImageData(id, `data:${picked.mime};base64,${picked.data}`);
        await this.refreshObjects();
        useChartStore.getState().select(objectId("image", id));
        this.ctl.grid?.objectLayer.select(objectId("image", id), false);
      }
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }
}

/** Magic bytes → MIME (PNG, JPEG), else `null`. */
export function sniffImageMime(bytes: Uint8Array): "image/png" | "image/jpeg" | null {
  if (bytes.length >= 8 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47 && bytes[4] === 0x0d && bytes[5] === 0x0a && bytes[6] === 0x1a && bytes[7] === 0x0a) return "image/png";
  if (bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return "image/jpeg";
  return null;
}

/** Browser fallback for `pick_image`: a file input, size-checked and sniffed like the shell. */
export async function pickImageInBrowser(): Promise<import("./shell").PickedImage | null> {
  if (typeof document === "undefined") return null;
  const file = await new Promise<File | null>((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png,image/jpeg";
    input.style.display = "none";
    input.addEventListener("change", () => {
      resolve(input.files?.[0] ?? null);
      input.remove();
    });
    input.addEventListener("cancel", () => {
      resolve(null);
      input.remove();
    });
    document.body.appendChild(input);
    input.click();
  });
  if (!file) return null;
  if (file.size > 16 * 1024 * 1024) throw { code: "invalid_argument", message: t("msg.error.pictureTooLarge", "The picture is larger than 16 MB.") };
  const bytes = new Uint8Array(await file.arrayBuffer());
  const mime = sniffImageMime(bytes);
  if (!mime) throw { code: "invalid_argument", message: t("msg.error.pictureType", "Only PNG and JPEG pictures can be inserted.") };
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  const data = btoa(binary);
  const size = await new Promise<{ width: number; height: number } | null>((resolve) => {
    const img = new Image();
    img.onload = () => resolve({ width: img.naturalWidth, height: img.naturalHeight });
    img.onerror = () => resolve(null);
    img.src = `data:${mime};base64,${data}`;
  });
  return { name: file.name, mime, data, width: size?.width ?? null, height: size?.height ?? null };
}
