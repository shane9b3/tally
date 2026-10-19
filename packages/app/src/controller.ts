/**
 * AppController: the only place that talks to the {@link EngineClient}.
 * Components call its methods (or run CommandRegistry commands); results
 * flow back into the zustand stores and the grid's data source.
 */
import {
  CANCELLED,
  isApiError,
  type ApiError,
  type ApplyResult,
  type CellPos,
  type CellView,
  type EngineClient,
  type FileFormat,
  type HAlign,
  type ShellClient,
  type Op,
  type RangeDto,
  type StyleDto,
  type StylePatch,
  type Transaction,
  type WorkbookInfo,
} from "@tally/bridge";
import {
  cellSelection,
  EngineClientSource,
  MAX_COLS,
  MAX_ROWS,
  type CommitMove,
  type Dir,
  type Grid as GridCore,
  type PointPhase,
  type Selection,
} from "@tally/grid";
import { CommandRegistry } from "./registry";
import { EMPTY_PATCH } from "./formats";
import { fillDownEnd, fillRange } from "./fill";
import { applyCompletion, applyPoint, assistFor, assistKey } from "./formulaEdit";
import { chipColorsFor } from "./palette";
import { useDocStore } from "./stores/docStore";
import { useEditStore } from "./stores/editStore";
import { useFunctionsStore } from "./stores/functionsStore";
import { fromSelectionDto, selectionRanges, toSelectionDto, useSelectionStore } from "./stores/selectionStore";
import { toastWarnings, useUiStore } from "./stores/uiStore";
import { detectSumRange, sumFormula, sumScanRanges } from "./sumRange";

export interface ControllerOptions {
  /** Inside Tauri: the native menu owns its accelerators. */
  nativeMenu: boolean;
}

const MOVES: Record<Dir, [number, number]> = { up: [-1, 0], down: [1, 0], left: [0, -1], right: [0, 1] };
const LAST_ROW = MAX_ROWS - 1;
const LAST_COL = MAX_COLS - 1;
/** Rows of a column fetched to auto-fit its width. */
export const AUTOFIT_ROWS = 2000;
/** Rows scanned below the source for a fill-handle double-click. */
export const FILL_SCAN_ROWS = 20_000;
/** Column width bounds for auto-fit (px at 100%). */
const AUTOFIT_MIN = 24;
const AUTOFIT_MAX = 600;

/** Toggle-able style fields. */
export type StyleToggle = "bold" | "italic" | "underline" | "strike" | "wrap";

const TOGGLE_LABELS: Record<StyleToggle, string> = {
  bold: "Bold",
  italic: "Italic",
  underline: "Underline",
  strike: "Strikethrough",
  wrap: "Wrap text",
};

function errorOf(e: unknown): ApiError {
  if (isApiError(e)) return e;
  return { code: "internal", message: e instanceof Error ? e.message : String(e) };
}

export class AppController {
  readonly registry = new CommandRegistry();
  grid: GridCore | null = null;
  private infoSeq = 0;
  /** The committed input of the cell being edited (skip no-op commits). */
  private editOriginal: string | null = null;
  private readonly offs: (() => void)[] = [];
  /** The last internal copy (context-menu paste when the system clipboard is unreadable). */
  private lastCopy: { token: string; tsv: string } | null = null;

  constructor(
    readonly client: EngineClient,
    readonly shell: ShellClient,
    readonly opts: ControllerOptions,
  ) {}

  // -- lifecycle --------------------------------------------------------------

  /** Subscribes to stores and engine events; returns the cleanup. */
  attach(): () => void {
    this.offs.push(
      useSelectionStore.subscribe((s, prev) => {
        if (s.selection.active.r !== prev.selection.active.r || s.selection.active.c !== prev.selection.active.c) {
          void this.refreshActive();
        }
      }),
      this.client.onMenuAction((e) => {
        if (!this.registry.run(e.id)) console.warn(`menu-action: unknown or disabled command ${e.id}`);
      }),
      this.client.onDocChanged((e) => {
        if (e.doc === useDocStore.getState().info?.doc) {
          useDocStore.getState().applyResult(e.result);
          void this.refreshActive();
        }
      }),
    );
    return () => {
      this.offs.splice(0).forEach((f) => f());
      useDocStore.getState().source?.dispose();
    };
  }

  async start(template?: string | null): Promise<void> {
    this.client.listFunctions().then(
      (fns) => useFunctionsStore.getState().set(fns),
      () => undefined,
    );
    try {
      const info = await this.client.newWorkbook(template ?? null);
      this.load(info);
    } catch (e) {
      this.notifyError(e);
    }
  }

  /** Shows a workbook (replacing the current one). */
  load(info: WorkbookInfo): void {
    const doc = useDocStore.getState();
    const prev = doc.info;
    doc.source?.dispose();
    if (prev && prev.doc !== info.doc) void this.client.close(prev.doc).catch(() => undefined);
    const source = new EngineClientSource(this.client, info.doc, info.activeSheet, {
      onError: (e) => console.warn("tally: fetch failed", e),
    });
    useEditStore.getState().reset();
    useSelectionStore.getState().reset();
    doc.setInfo(info);
    useDocStore.getState().setActiveSheet(info.activeSheet);
    doc.setSource(source);
    toastWarnings(info.warnings);
    void this.refreshActive();
  }

  setGrid = (grid: GridCore | null): void => {
    this.grid = grid;
  };

  focusGrid(): void {
    this.grid?.focus();
  }

  // -- helpers ----------------------------------------------------------------

  get info(): WorkbookInfo | null {
    return useDocStore.getState().info;
  }

  get sheet(): number {
    return useDocStore.getState().activeSheet;
  }

  get selection(): Selection {
    return useSelectionStore.getState().selection;
  }

  notifyError = (e: unknown): void => {
    const err = errorOf(e);
    if (err.code === CANCELLED) return;
    console.warn("tally:", err);
    useUiStore.getState().pushToast("error", err.message);
  };

  private requireDoc(): number {
    const info = this.info;
    if (!info) throw { code: "not_found", message: "No workbook is open." } satisfies ApiError;
    return info.doc;
  }

  /** Feeds an engine result to the cache and stores. */
  handleResult(res: ApplyResult): void {
    const before = this.sheet;
    useDocStore.getState().source?.applyResult(res);
    useDocStore.getState().applyResult(res);
    const after = this.sheet;
    if (after !== before) this.showSheet(after, false);
    if (res.selection && res.selection.sheet === this.sheet) {
      const sel = fromSelectionDto(res.selection);
      useSelectionStore.getState().setSelection(sel);
      this.grid?.scrollCellIntoView(sel.active);
    }
    const first = res.errors[0];
    if (first) useUiStore.getState().pushToast("error", first.message);
    void this.refreshActive();
  }

  /** Applies ops as one undoable transaction. */
  async apply(label: string, ops: Op[], after?: Selection): Promise<ApplyResult | null> {
    try {
      const doc = this.requireDoc();
      const sel = this.selection;
      const tx: Transaction = {
        ops,
        label,
        selectionBefore: toSelectionDto(sel, this.sheet),
        selectionAfter: toSelectionDto(after ?? sel, this.sheet),
      };
      const res = await this.client.apply(doc, tx);
      this.handleResult(res);
      return res;
    } catch (e) {
      this.notifyError(e);
      return null;
    }
  }

  /** Re-reads the active cell's input and hint for the formula bar. */
  async refreshActive(): Promise<void> {
    const info = this.info;
    if (!info) return;
    const seq = ++this.infoSeq;
    const sheet = this.sheet;
    const cell = { ...this.selection.active };
    try {
      const res = await this.client.cellEditInfo(info.doc, sheet, cell.r, cell.c);
      if (seq !== this.infoSeq) return;
      useEditStore.getState().setActive({ sheet, cell, input: res.input, hint: res.hint });
    } catch {
      // Quiet: background query (the stub engine does not support it).
      if (seq === this.infoSeq) useEditStore.getState().setActive({ sheet, cell, input: "", hint: null });
    }
  }

  // -- undo -------------------------------------------------------------------

  undo = async (): Promise<void> => {
    try {
      this.handleResult(await this.client.undo(this.requireDoc()));
    } catch (e) {
      this.notifyError(e);
    }
  };

  redo = async (): Promise<void> => {
    try {
      this.handleResult(await this.client.redo(this.requireDoc()));
    } catch (e) {
      this.notifyError(e);
    }
  };

  // -- formatting -------------------------------------------------------------

  /** Every range of the selection (multi-range aware). */
  get ranges(): RangeDto[] {
    return selectionRanges(this.selection);
  }

  /** The resolved style of the active cell (fetched). */
  async activeStyle(): Promise<StyleDto | undefined> {
    const doc = this.requireDoc();
    const a = this.selection.active;
    const cached = useDocStore.getState().source?.getCell(a.r, a.c);
    const [style] = await this.client.styles(doc, [cached?.style ?? 0]);
    return style;
  }

  /** Toggles a style flag on every selected range; the active cell's current value decides. */
  async toggleStyle(field: StyleToggle): Promise<void> {
    try {
      const style = await this.activeStyle();
      const patch: StylePatch = { ...EMPTY_PATCH, [field]: !(style?.[field] ?? false) };
      await this.applyPatch(TOGGLE_LABELS[field], patch);
    } catch (e) {
      this.notifyError(e);
    }
  }

  /** Applies a style patch to every selected range. */
  applyPatch(label: string, patch: StylePatch, ranges: RangeDto[] = this.ranges): Promise<ApplyResult | null> {
    return this.apply(label, [{ type: "applyStyle", sheet: this.sheet, ranges, patch }]);
  }

  setFontSize = (pt: number): Promise<ApplyResult | null> => this.applyPatch("Font size", { ...EMPTY_PATCH, fontSizePt: pt });
  setTextColor = (rgba: number): Promise<ApplyResult | null> => this.applyPatch("Font colour", { ...EMPTY_PATCH, color: rgba });
  setFillColor = (rgba: number): Promise<ApplyResult | null> => this.applyPatch("Fill colour", { ...EMPTY_PATCH, fill: rgba });
  setAlign = (h: HAlign): Promise<ApplyResult | null> => this.applyPatch("Align", { ...EMPTY_PATCH, hAlign: h });

  setNumberFormat(code: string, label: string): Promise<ApplyResult | null> {
    return this.setNumberFormatOn(this.ranges, code, label);
  }

  /** Applies a number format to explicit ranges of the active sheet. */
  setNumberFormatOn(ranges: RangeDto[], code: string, label: string): Promise<ApplyResult | null> {
    return this.apply(label, [{ type: "setNumberFormat", sheet: this.sheet, ranges: ranges.map((r) => ({ ...r })), code }]);
  }

  /**
   * "Colour by value": for the selected column of category text, gives each
   * distinct value its own chip colours (fill + font) from the chip
   * palette, one `applyStyle` per value. Emulates status chips until
   * conditional formatting exists.
   */
  async colorByValue(): Promise<void> {
    try {
      const doc = this.requireDoc();
      const sheet = this.sheet;
      const sel = this.selection.range;
      const layout = await this.client.sheetLayout(doc, sheet);
      const used = layout.used;
      if (!used) return;
      const c = sel.c0;
      const r0 = Math.max(sel.r0, used.r0);
      const r1 = Math.min(sel.r1, used.r1);
      if (r1 < r0) return;
      const [block] = await this.client.cells(doc, sheet, [{ r0, c0: c, r1, c1: c }]);
      const texts = (block?.cells ?? []).filter((v) => v.kind === "text" && v.text.trim() !== "");
      // A whole-column selection skips a header (the first text above repeating values).
      const counts = new Map<string, number>();
      for (const v of texts) counts.set(v.text.trim(), (counts.get(v.text.trim()) ?? 0) + 1);
      const header = sel.r0 === 0 && sel.r1 === LAST_ROW ? texts.find((v) => (counts.get(v.text.trim()) ?? 0) === 1) : undefined;
      const groups = new Map<string, RangeDto[]>();
      for (const v of texts) {
        if (v === header) continue;
        const key = v.text.trim();
        const list = groups.get(key) ?? [];
        const last = list[list.length - 1];
        if (last && last.r1 === v.r - 1) last.r1 = v.r;
        else list.push({ r0: v.r, c0: c, r1: v.r, c1: c });
        groups.set(key, list);
      }
      if (groups.size === 0) {
        useUiStore.getState().pushToast("info", "Select a column of text values to colour.");
        return;
      }
      const colors = chipColorsFor([...groups.keys()]);
      const ops: Op[] = [...groups.entries()].map(([value, ranges]) => {
        const chip = colors.get(value)!;
        return { type: "applyStyle", sheet, ranges, patch: { ...EMPTY_PATCH, fill: chip.bg, color: chip.fg } };
      });
      await this.apply("Colour by value", ops);
    } catch (e) {
      this.notifyError(e);
    }
  }

  /** Σ: `=SUM(<numeric block above, else to the left>)` in the active cell. */
  async addUp(): Promise<void> {
    try {
      const doc = this.requireDoc();
      const sheet = this.sheet;
      const active = { ...this.selection.active };
      const ranges = sumScanRanges(active);
      const cells = new Map<string, CellView>();
      if (ranges.length > 0) {
        for (const block of await this.client.cells(doc, sheet, ranges)) {
          for (const c of block.cells) cells.set(`${c.r},${c.c}`, c);
        }
      }
      const range = detectSumRange(active, (r, c) => cells.get(`${r},${c}`));
      if (!range) {
        // Nothing to add up: open the editor with a starter formula.
        this.startEdit(active, "=SUM(", "grid");
        this.focusGrid();
        return;
      }
      await this.apply("Add up", [{ type: "setCellInput", sheet, r: active.r, c: active.c, input: sumFormula(range) }]);
    } catch (e) {
      this.notifyError(e);
    }
  }

  insertFormula(formula: string): Promise<ApplyResult | null> {
    const a = this.selection.active;
    return this.apply("Insert formula", [{ type: "setCellInput", sheet: this.sheet, r: a.r, c: a.c, input: formula }]);
  }

  // -- cells --------------------------------------------------------------------

  onSelectionChange = (sel: Selection): void => {
    useSelectionStore.getState().setSelection(sel);
  };

  /** Starts the shared EditSession (grid or formula bar). */
  startEdit = (cell: CellPos, seed?: string, origin: "grid" | "bar" = "grid"): void => {
    const store = useEditStore.getState();
    const active = store.active;
    const known = active && active.sheet === this.sheet && active.cell.r === cell.r && active.cell.c === cell.c ? active.input : null;
    this.editOriginal = known;
    if (seed !== undefined) {
      store.start(cell, seed, origin);
      return;
    }
    store.start(cell, known ?? "", origin);
    if (known !== null) return;
    const info = this.info;
    if (!info) return;
    this.client.cellEditInfo(info.doc, this.sheet, cell.r, cell.c).then(
      (res) => {
        const cur = useEditStore.getState().edit;
        this.editOriginal = res.input;
        if (cur && cur.cell.r === cell.r && cur.cell.c === cell.c && cur.text === "") useEditStore.getState().change(res.input);
      },
      () => undefined,
    );
  };

  onGridStartEdit = (cell: CellPos, seed?: string): void => this.startEdit(cell, seed, "grid");

  onEditChange = (text: string, caret: number): void => {
    useEditStore.getState().change(text, caret);
  };

  /** Commits the edit. The grid moves its own selection; the formula bar asks us to. */
  commitEdit = async (move: CommitMove): Promise<void> => {
    const { edit, origin } = useEditStore.getState();
    if (!edit) return;
    useEditStore.getState().end();
    const sheet = this.sheet;
    const unchanged = this.editOriginal !== null && edit.text === this.editOriginal;
    // apply() builds the transaction synchronously, so selectionBefore is the pre-move selection.
    const pending = unchanged
      ? null
      : this.apply("Typing", [{ type: "setCellInput", sheet, r: edit.cell.r, c: edit.cell.c, input: edit.text }]);
    if (origin === "bar") {
      if (move !== "none") {
        const [dr, dc] = MOVES[move];
        const next = {
          r: Math.min(LAST_ROW, Math.max(0, edit.cell.r + dr)),
          c: Math.min(LAST_COL, Math.max(0, edit.cell.c + dc)),
        };
        useSelectionStore.getState().setSelection(cellSelection(next));
        this.grid?.scrollCellIntoView(next);
      }
      this.focusGrid();
    }
    await pending;
  };

  onGridCommitEdit = (move: CommitMove): void => {
    void this.commitEdit(move);
  };

  cancelEdit = (): void => {
    const origin = useEditStore.getState().origin;
    useEditStore.getState().end();
    if (origin === "bar") this.focusGrid();
  };

  deleteContents = (range: RangeDto): void => {
    // The grid reports the current range; clear every range of a multi-selection.
    const ranges = this.ranges;
    const all = ranges.some((r) => r.r0 === range.r0 && r.c0 === range.c0 && r.r1 === range.r1 && r.c1 === range.c1) ? ranges : [{ ...range }];
    void this.apply("Delete", [{ type: "clear", sheet: this.sheet, ranges: all, what: "contents" }]);
  };

  clearSelection = (): void => this.deleteContents(this.selection.range);

  clearFormats = (): Promise<ApplyResult | null> =>
    this.apply("Clear formats", [{ type: "clear", sheet: this.sheet, ranges: this.ranges, what: "formats" }]);

  // -- structure ------------------------------------------------------------------

  /** Inserts `count` rows above (`before`) or below the selected rows. */
  insertRows = (where: "before" | "after", count?: number): Promise<ApplyResult | null> => {
    const r = this.selection.range;
    const n = count ?? r.r1 - r.r0 + 1;
    const at = where === "before" ? r.r0 : r.r1 + 1;
    const after = { range: { ...r, r0: at, r1: at + n - 1 }, active: { r: at, c: this.selection.active.c }, anchor: { r: at, c: r.c0 } };
    return this.apply(n === 1 ? "Insert row" : "Insert rows", [{ type: "insertRows", sheet: this.sheet, at, count: n }], after);
  };

  insertCols = (where: "before" | "after", count?: number): Promise<ApplyResult | null> => {
    const r = this.selection.range;
    const n = count ?? r.c1 - r.c0 + 1;
    const at = where === "before" ? r.c0 : r.c1 + 1;
    const after = { range: { ...r, c0: at, c1: at + n - 1 }, active: { r: this.selection.active.r, c: at }, anchor: { r: r.r0, c: at } };
    return this.apply(n === 1 ? "Insert column" : "Insert columns", [{ type: "insertCols", sheet: this.sheet, at, count: n }], after);
  };

  deleteRows = (): Promise<ApplyResult | null> => {
    const r = this.selection.range;
    return this.apply("Delete rows", [{ type: "deleteRows", sheet: this.sheet, at: r.r0, count: r.r1 - r.r0 + 1 }], cellSelection({ r: r.r0, c: this.selection.active.c }));
  };

  deleteCols = (): Promise<ApplyResult | null> => {
    const r = this.selection.range;
    return this.apply("Delete columns", [{ type: "deleteCols", sheet: this.sheet, at: r.c0, count: r.c1 - r.c0 + 1 }], cellSelection({ r: this.selection.active.r, c: r.c0 }));
  };

  /** Fits columns `start..=end` to their widest text (visible cells + the first rows of each column). */
  autoFit = async (axis: "col", start: number, end: number): Promise<void> => {
    try {
      const doc = this.requireDoc();
      const grid = this.grid;
      if (!grid || axis !== "col") return;
      const vp = grid.getViewport();
      const top = Math.max(0, Math.min(vp.r0, LAST_ROW - AUTOFIT_ROWS + 1));
      const ops: Op[] = [];
      for (let c = start; c <= end; c++) {
        // The first rows of the column plus the rows from the viewport down (up to AUTOFIT_ROWS each).
        const ranges = [{ r0: 0, c0: c, r1: Math.min(LAST_ROW, AUTOFIT_ROWS - 1), c1: c }];
        if (top >= AUTOFIT_ROWS) ranges.push({ r0: top, c0: c, r1: Math.min(LAST_ROW, top + AUTOFIT_ROWS - 1), c1: c });
        const blocks = await this.client.cells(doc, this.sheet, ranges);
        const w = grid.measureColumnWidth(c, blocks.flatMap((b) => b.cells));
        if (w === null) continue;
        ops.push({ type: "setColWidth", sheet: this.sheet, start: c, end: c, px: Math.max(AUTOFIT_MIN, Math.min(AUTOFIT_MAX, w)) });
      }
      if (ops.length > 0) await this.apply("Fit column width", ops);
    } catch (e) {
      this.notifyError(e);
    }
  };

  // -- fill handle ------------------------------------------------------------------

  /** Fill-handle drag: see fill.ts (`fillRange` is the engine-swap point). */
  onFill = async (source: RangeDto, target: RangeDto): Promise<void> => {
    try {
      const doc = this.requireDoc();
      const ops = await fillRange({ client: this.client, doc, sheet: this.sheet }, source, target);
      if (ops.length === 0) return;
      const sel: Selection = { range: { ...target }, active: { ...this.selection.active }, anchor: { r: target.r0, c: target.c0 } };
      await this.apply("Fill", ops, sel);
    } catch (e) {
      this.notifyError(e);
    }
  };

  /** Fill-handle double-click: fill down to the end of the adjacent column's data. */
  onFillHandleDoubleClick = async (source: RangeDto): Promise<void> => {
    try {
      const doc = this.requireDoc();
      const rows = { r0: source.r0, r1: Math.min(LAST_ROW, source.r1 + FILL_SCAN_ROWS) };
      const cols: RangeDto[] = [];
      if (source.c0 > 0) cols.push({ ...rows, c0: source.c0 - 1, c1: source.c0 - 1 });
      if (source.c1 < LAST_COL) cols.push({ ...rows, c0: source.c1 + 1, c1: source.c1 + 1 });
      cols.push({ ...rows, c0: source.c0, c1: source.c0 });
      const cells = new Map<string, CellView>();
      for (const b of await this.client.cells(doc, this.sheet, cols)) for (const v of b.cells) cells.set(`${v.r},${v.c}`, v);
      const end = fillDownEnd(source, (r, c) => cells.get(`${r},${c}`), FILL_SCAN_ROWS);
      if (end === null) return;
      const target = { ...source, r1: end };
      this.grid?.setSelection({ range: target, active: this.selection.active, anchor: { r: target.r0, c: target.c0 } });
      await this.onFill(source, target);
    } catch (e) {
      this.notifyError(e);
    }
  };

  // -- formula editing --------------------------------------------------------------

  /** Point mode: insert or replace the picked reference at the caret (both editors mirror it). */
  onPointRange = (range: RangeDto, _phase: PointPhase): void => {
    const st = useEditStore.getState();
    const edit = st.edit;
    if (!edit) return;
    const res = applyPoint(edit.text, edit.caret, st.pointSpan, range);
    if (!res) return;
    st.replace(res.text, res.caret, res.span);
  };

  /** Inserts `NAME(` for the identifier being typed. */
  insertCompletion = (name: string): boolean => {
    const st = useEditStore.getState();
    const edit = st.edit;
    if (!edit) return false;
    const list = assistFor(edit.text, edit.caret, useFunctionsStore.getState().functions, st.assistDismissed);
    if (!list) return false;
    const res = applyCompletion(edit.text, list.span, name);
    st.replace(res.text, res.caret);
    return true;
  };

  /**
   * Keys inside an open editor (in-cell or formula bar), before anything
   * else: editor-local undo/redo and autocomplete navigation. Returns true
   * when handled (the caller prevents the default and stops propagation).
   */
  handleEditorKey = (e: { key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): boolean => {
    const st = useEditStore.getState();
    const edit = st.edit;
    if (!edit) return false;
    const mod = e.metaKey || e.ctrlKey;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    if (mod && !e.altKey && key === "z") {
      if (e.shiftKey) st.redo();
      else st.undo();
      return true;
    }
    if (mod && !e.altKey && !e.shiftKey && key === "y") {
      st.redo();
      return true;
    }
    const list = assistFor(edit.text, edit.caret, useFunctionsStore.getState().functions, st.assistDismissed);
    if (!list || mod || e.altKey) return false;
    const n = list.items.length;
    switch (e.key) {
      case "ArrowDown":
        st.setAssist((st.assistIndex + 1) % n);
        return true;
      case "ArrowUp":
        st.setAssist((st.assistIndex - 1 + n) % n);
        return true;
      case "Tab":
      case "Enter": {
        const item = list.items[Math.min(st.assistIndex, n - 1)];
        return item ? this.insertCompletion(item.name) : false;
      }
      case "Escape":
        st.dismissAssist(assistKey(list));
        return true;
      default:
        return false;
    }
  };

  /** ⌘Z: the editor's own history while editing, else the document's. */
  undoAny = async (): Promise<void> => {
    if (useEditStore.getState().edit) {
      useEditStore.getState().undo();
      return;
    }
    await this.undo();
  };

  redoAny = async (): Promise<void> => {
    if (useEditStore.getState().edit) {
      useEditStore.getState().redo();
      return;
    }
    await this.redo();
  };

  resize = (axis: "col" | "row", start: number, end: number, px: number): void => {
    const op: Op =
      axis === "col"
        ? { type: "setColWidth", sheet: this.sheet, start, end, px }
        : { type: "setRowHeight", sheet: this.sheet, start, end, px };
    void this.apply(axis === "col" ? "Column width" : "Row height", [op]);
  };

  /** Current size (px at 100%) of row/column `i` from the sheet layout. */
  layoutSize(axis: "row" | "col", i: number): number {
    const layout = useDocStore.getState().source?.getLayout(this.sheet);
    if (!layout) return axis === "row" ? 24 : 100;
    const runs = axis === "row" ? layout.rowRuns : layout.colRuns;
    const run = runs.find((r) => i >= r.start && i <= r.end);
    return run ? run.size : axis === "row" ? layout.defaultRowHeightPx : layout.defaultColWidthPx;
  }

  navigateEdge = (from: CellPos, dir: Dir): Promise<CellPos> => {
    const info = this.info;
    if (!info) return Promise.resolve(from);
    return this.client.navigateEdge(info.doc, this.sheet, from, dir).catch((e: unknown) => {
      this.notifyError(e);
      return from;
    });
  };

  /** Name box: `B12`, `A1:C5`, `Sheet2.A1`. */
  async goTo(text: string): Promise<boolean> {
    try {
      const dto = await this.client.resolveNameBox(this.requireDoc(), text);
      if (dto.sheet !== this.sheet) this.showSheet(dto.sheet, false);
      const sel = fromSelectionDto(dto);
      useSelectionStore.getState().setSelection(sel);
      this.grid?.scrollCellIntoView(sel.active);
      this.focusGrid();
      return true;
    } catch (e) {
      this.notifyError(e);
      return false;
    }
  }

  // -- clipboard ----------------------------------------------------------------

  copy = async (): Promise<boolean> => {
    try {
      const doc = this.requireDoc();
      if ((this.selection.others ?? []).length > 0) {
        useUiStore.getState().pushToast("info", "Copying several ranges at once is not supported; copied the last one.");
      }
      const sel = this.selection;
      const out = await this.client.copy(doc, { sheet: this.sheet, ranges: [{ ...sel.range }], active: { ...sel.active } });
      this.lastCopy = { token: out.token, tsv: out.tsv };
      await writeClipboard(out.tsv, out.html);
      return true;
    } catch (e) {
      this.notifyError(e);
      return false;
    }
  };

  cut = async (): Promise<void> => {
    const range = { ...this.selection.range };
    if (await this.copy()) this.deleteContents(range);
  };

  onGridCopy = (): void => void this.copy();
  onGridCut = (): void => void this.cut();

  /** Pastes TSV text or an internal clipboard token at the selection. */
  pasteData(text: string, token: string | null, valuesOnly: boolean): Promise<ApplyResult | null> {
    if ((valuesOnly || !token) && text === "") return Promise.resolve(null);
    const target = { ...this.selection.range };
    const payload = token && !valuesOnly ? { type: "internal" as const, token } : { type: "tsv" as const, text };
    return this.apply(valuesOnly ? "Paste values" : "Paste", [{ type: "paste", sheet: this.sheet, target, payload }]);
  }

  onGridPaste = (data: DataTransfer | null, opts?: { plain: boolean }): void => {
    const text = data?.getData("text/plain") ?? "";
    const html = data?.getData("text/html") ?? "";
    const token = /data-tally-clip="([^"]+)"/.exec(html)?.[1] ?? null;
    void this.pasteData(text, token, opts?.plain ?? false);
  };

  /**
   * Paste from a menu (no DOM paste event): reads the system clipboard if
   * the browser allows, else uses the last copy made in this window.
   */
  pasteFromClipboard = async (valuesOnly: boolean): Promise<void> => {
    let text: string | null = null;
    let token: string | null = null;
    const clip = typeof navigator === "undefined" ? undefined : navigator.clipboard;
    try {
      if (clip && typeof clip.read === "function") {
        for (const item of await clip.read()) {
          if (item.types.includes("text/html")) token = /data-tally-clip="([^"]+)"/.exec(await (await item.getType("text/html")).text())?.[1] ?? token;
          if (item.types.includes("text/plain")) text = await (await item.getType("text/plain")).text();
        }
      } else if (clip) text = await clip.readText();
    } catch {
      text = null;
    }
    if (text === null && this.lastCopy) {
      text = this.lastCopy.tsv;
      token = this.lastCopy.token;
    }
    if (text === null) {
      useUiStore.getState().pushToast("info", "Use ⌘V to paste (the clipboard is not readable here).");
      return;
    }
    await this.pasteData(text, token, valuesOnly);
    this.focusGrid();
  };

  // -- sheets -------------------------------------------------------------------

  /** Switches the visible sheet (UI state). */
  showSheet(sheet: number, focus = true): void {
    const doc = useDocStore.getState();
    if (useEditStore.getState().edit) useEditStore.getState().end();
    if (doc.activeSheet !== sheet) doc.setActiveSheet(sheet);
    doc.source?.setSheet(sheet);
    useSelectionStore.getState().reset();
    void this.refreshActive();
    if (focus) this.focusGrid();
  }

  stepSheet(delta: 1 | -1): void {
    const info = this.info;
    if (!info) return;
    const i = info.sheets.findIndex((s) => s.id === this.sheet);
    const next = info.sheets[(i + delta + info.sheets.length) % info.sheets.length];
    if (next) this.showSheet(next.id);
  }

  addSheet = (): Promise<ApplyResult | null> => this.apply("Add sheet", [{ type: "addSheet", name: null, index: null }]);

  renameSheet = (sheet: number, name: string): Promise<ApplyResult | null> =>
    this.apply("Rename sheet", [{ type: "renameSheet", sheet, name }]);

  deleteSheet = (sheet: number): Promise<ApplyResult | null> => this.apply("Delete sheet", [{ type: "deleteSheet", sheet }]);

  moveSheet = (sheet: number, to: number): Promise<ApplyResult | null> =>
    this.apply("Move sheet", [{ type: "moveSheet", sheet, to }]);

  // -- files ----------------------------------------------------------------------

  newWorkbook = async (): Promise<void> => {
    try {
      this.load(await this.client.newWorkbook(null));
    } catch (e) {
      this.notifyError(e);
    }
  };

  newFromTemplate = async (id: string): Promise<void> => {
    try {
      this.load(await this.client.newWorkbook(id));
    } catch (e) {
      this.notifyError(e);
    }
  };

  openDialog = async (): Promise<void> => {
    try {
      const info = await this.shell.openDialog();
      if (info) this.load(info);
    } catch (e) {
      this.notifyError(e);
    }
  };

  openRecent = async (path: string): Promise<void> => {
    try {
      this.load(await this.client.open(path));
    } catch (e) {
      this.notifyError(e);
    }
  };

  save = async (saveAs: boolean, format: FileFormat | null = null): Promise<void> => {
    try {
      const info = this.info;
      if (!info) return;
      const res = await this.shell.saveWithDialog(info.doc, saveAs, format);
      useDocStore.getState().setInfo(res);
      if (format === "csv" || format === "ods") useUiStore.getState().pushToast("info", `Exported ${res.title}`);
    } catch (e) {
      this.notifyError(e);
    }
  };

  closeDocument = async (): Promise<void> => {
    await this.newWorkbook();
  };
}

/** Writes TSV + HTML with the async Clipboard API (text only as a fallback). */
export async function writeClipboard(tsv: string, html: string): Promise<void> {
  const nav = typeof navigator === "undefined" ? undefined : navigator;
  const clip = nav?.clipboard;
  if (!clip) return;
  if (typeof ClipboardItem !== "undefined" && typeof clip.write === "function") {
    try {
      await clip.write([
        new ClipboardItem({
          "text/plain": new Blob([tsv], { type: "text/plain" }),
          "text/html": new Blob([html], { type: "text/html" }),
        }),
      ]);
      return;
    } catch {
      // Fall through to text.
    }
  }
  await clip.writeText(tsv);
}
