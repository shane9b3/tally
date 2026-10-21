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
  type ColumnFilter,
  type EngineClientV3,
  type FileFormat,
  type FilterOp,
  type NameInfo,
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
  drawStyleId,
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
import {
  absoluteRef,
  arrayFormulaOp,
  bordersPatch,
  columnFilterOp,
  fillDownRight,
  findQuery,
  freezeAt,
  friendlyError,
  isValidName,
  pasteOptions,
  sortOp,
  stepHit,
  a1 as a1Text,
  type BorderPreset,
  type PasteSpecialChoice,
  type SortSpec,
} from "./dataOps";
import { cappedRange, currentRegion } from "./glance";
import { useDataStore } from "./stores/dataStore";
import { applyCompletion, applyPoint, assistFor, assistKey } from "./formulaEdit";
import { chipColorsFor } from "./palette";
import { useDocStore } from "./stores/docStore";
import { useEditStore } from "./stores/editStore";
import { useFunctionsStore } from "./stores/functionsStore";
import { fromSelectionDto, selectionRanges, toSelectionDto, useSelectionStore } from "./stores/selectionStore";
import { toastWarnings, useUiStore } from "./stores/uiStore";
import { detectSumRange, sumFormula, sumScanRanges } from "./sumRange";
import { V3Features } from "./features";
import { useV3Store } from "./stores/v3Store";
import { W5Features } from "./objects/w5Features";
import { W56Features } from "./data/w56";

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
/** How long a menu paste waits for the system clipboard before using the last internal copy. */
const CLIPBOARD_TIMEOUT_MS = 400;
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

  /** v0.3 features (conditional formatting, validation, notes, links, protection, page setup…). */
  readonly v3: V3Features;
  /** Wave 5 (W5-5): charts and pictures, print/PDF, split window, detective, view presets. */
  readonly w5: W5Features;
  /** Wave 5 (W5-6): DataPilot, data tools, styles, options, assists, navigator, i18n. */
  readonly w56: W56Features;

  constructor(
    readonly client: EngineClientV3,
    readonly shell: ShellClient,
    readonly opts: ControllerOptions,
  ) {
    this.v3 = new V3Features(this);
    this.w5 = new W5Features(this);
    this.w56 = new W56Features(this);
  }

  // -- lifecycle --------------------------------------------------------------

  /** Subscribes to stores and engine events; returns the cleanup. */
  attach(): () => void {
    this.offs.push(
      useSelectionStore.subscribe((s, prev) => {
        if (s.selection.active.r !== prev.selection.active.r || s.selection.active.c !== prev.selection.active.c) {
          void this.refreshActive();
          void this.v3.showInputHelp({ ...s.selection.active });
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
      this.w56.attach(),
    );
    return () => {
      this.offs.splice(0).forEach((f) => f());
      useDocStore.getState().source?.dispose();
    };
  }

  async start(template?: string | null): Promise<void> {
    this.client.listFunctions().then(
      (fns) => this.w56.setFunctions(fns),
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
    useDataStore.getState().reset();
    useV3Store.getState().reset();
    toastWarnings(info.warnings);
    void this.refreshActive();
    void this.refreshFilter();
    void this.refreshNames();
    void this.v3.refreshSheet();
    this.w5.refreshSheet();
    this.w56.onLoad();
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

  requireDoc(): number {
    const info = this.info;
    if (!info) throw { code: "not_found", message: "No workbook is open." } satisfies ApiError;
    return info.doc;
  }

  /**
   * Feeds an engine result to the cache and stores. With the `ops` that
   * produced it, validation and protection refusals and validation warnings
   * are reported as dialogs/toasts (`V3Features.reportProblems`); other
   * errors show the first one as a toast.
   */
  handleResult(res: ApplyResult, ops: readonly Op[] | null = null, restoreSelection = true): void {
    const before = this.sheet;
    useDocStore.getState().source?.applyResult(res);
    useDocStore.getState().applyResult(res);
    const after = this.sheet;
    if (after !== before) this.showSheet(after, false);
    // Undo/redo (and ops that name their own selection) put the cursor back;
    // a plain edit must not, or Enter/Tab would snap back to the edited cell
    // after the grid has already moved on.
    if (restoreSelection && res.selection && res.selection.sheet === this.sheet) {
      const sel = fromSelectionDto(res.selection);
      useSelectionStore.getState().setSelection(sel);
      this.grid?.scrollCellIntoView(sel.active);
    }
    const handled = ops ? this.v3.reportProblems(res, ops) : new Set<number>();
    const first = res.errors.find((_, i) => !handled.has(i));
    if (first) useUiStore.getState().pushToast("error", friendlyError(first.code, first.message));
    this.v3.afterResult(res, ops);
    this.w5.afterResult(res, ops);
    this.w56.afterResult(res, ops);
    void this.refreshActive();
    const sheet = this.sheet;
    const filtered = useDataStore.getState().filter !== null;
    if (res.layoutChanged.includes(sheet) || (filtered && res.changed.some((c) => c.sheet === sheet))) void this.refreshFilter();
    if (res.sheetsChanged) void this.refreshNames();
  }

  /** Applies ops as one undoable transaction. */
  async apply(label: string, ops: Op[], after?: Selection, keepClip = false): Promise<ApplyResult | null> {
    try {
      const doc = this.requireDoc();
      // Any other edit ends a pending copy/cut (as in Calc and Excel).
      if (!keepClip && useDataStore.getState().clip) useDataStore.getState().setClip(null);
      const sel = this.selection;
      const tx: Transaction = {
        ops,
        label,
        selectionBefore: toSelectionDto(sel, this.sheet),
        selectionAfter: toSelectionDto(after ?? sel, this.sheet),
      };
      const res = await this.client.apply(doc, tx);
      this.handleResult(res, ops, after !== undefined);
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
      useEditStore.getState().setActive({ sheet, cell, input: this.w56.displayText(res.input), hint: res.hint, arrayRange: res.array ? (res.arrayRange ?? null) : null });
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
   * "Colour by value": for the selected column of category text, one
   * conditional format over the column body with a chip rule per distinct
   * value (colours from the chip palette), so the chips follow edits.
   */
  colorByValue(): Promise<void> {
    return this.colorByValueOn(this.selection.range);
  }

  /** "Colour by value" over an explicit column range. */
  async colorByValueOn(sel: RangeDto): Promise<void> {
    try {
      const doc = this.requireDoc();
      const sheet = this.sheet;
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
      const values = [...groups.keys()];
      const colors = chipColorsFor(values);
      const rows = [...groups.values()].flat();
      const body = { r0: Math.min(...rows.map((g) => g.r0)), c0: c, r1: Math.max(...rows.map((g) => g.r1)), c1: c };
      await this.v3.colorByValue(body, values, colors);
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
        this.editOriginal = this.w56.displayText(res.input);
        if (cur && cur.cell.r === cell.r && cur.cell.c === cell.c && cur.text === "") useEditStore.getState().change(this.editOriginal);
      },
      () => undefined,
    );
  };

  onGridStartEdit = (cell: CellPos, seed?: string): void => {
    if (seed === undefined && this.w56.maybeDrillDown(cell)) return;
    this.startEdit(cell, seed, "grid");
  };

  onEditChange = (text: string, caret: number): void => {
    useEditStore.getState().change(text, caret);
    this.w56.onEditChange(text);
  };

  /** Commits the edit. The grid moves its own selection; the formula bar asks us to. */
  commitEdit = async (move: CommitMove): Promise<void> => {
    const { edit, origin } = useEditStore.getState();
    if (!edit) return;
    useEditStore.getState().end();
    const sheet = this.sheet;
    const unchanged = this.editOriginal !== null && edit.text === this.editOriginal;
    // Editing `{=…}` inside an array block re-enters the whole block.
    const active = useEditStore.getState().active;
    const block = active && active.cell.r === edit.cell.r && active.cell.c === edit.cell.c ? active.arrayRange : null;
    const arrayOp = block && edit.text.trim().startsWith("{=") ? arrayFormulaOp(sheet, block, edit.text) : null;
    // apply() builds the transaction synchronously, so selectionBefore is the pre-move selection.
    const pending = unchanged
      ? null
      : arrayOp
        ? this.apply("Array formula", [arrayOp])
        : this.apply("Typing", this.typingOps(sheet, edit.cell, edit.text));
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

  /**
   * The ops for typing `text` into `cell`: the input, plus `autoFitRows` for
   * the row when the cell wraps or the text has line breaks (one undo step).
   */
  typingOps(sheet: number, cell: CellPos, text: string): Op[] {
    const ops: Op[] = [{ type: "setCellInput", sheet, r: cell.r, c: cell.c, input: this.w56.commitText(text, this.editOriginal) }];
    const source = useDocStore.getState().source;
    const view = source?.getCell(cell.r, cell.c);
    const wraps = source?.getStyle(drawStyleId(view))?.wrap ?? false;
    if (wraps || text.includes("\n")) ops.push({ type: "autoFitRows", sheet, start: cell.r, end: cell.r });
    return ops;
  }

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
      this.requireDoc();
      const ops = fillRange({ sheet: this.sheet }, source, target);
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
    if (this.w56.editorKey(e)) return true;
    const mod = e.metaKey || e.ctrlKey;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    if (e.key === "Enter" && mod && e.shiftKey && !e.altKey) {
      void this.commitArray();
      return true;
    }
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
      const dto = await this.client.resolveNameBoxIn(this.requireDoc(), this.sheet, text);
      if (dto.sheet !== this.sheet) this.showSheet(dto.sheet, false);
      const sel = fromSelectionDto(dto);
      useSelectionStore.getState().setSelection(sel);
      this.grid?.scrollCellIntoView(sel.active);
      this.focusGrid();
      return true;
    } catch (e) {
      // Typing a new name into the name box names the selection (Calc, Excel).
      const name = text.trim();
      if (isApiError(e) && e.code === "not_found" && isValidName(name) && !useDataStore.getState().names.some((n) => n.name.toLowerCase() === name.toLowerCase())) {
        const ok = await this.defineName(name, null, absoluteRef(this.sheetName(), this.selection.range), null);
        if (ok) useUiStore.getState().pushToast("info", `Named ${a1Text(this.selection.range)} “${name}”`);
        this.focusGrid();
        return ok;
      }
      this.notifyError(e);
      return false;
    }
  }

  // -- clipboard ----------------------------------------------------------------

  copy = async (): Promise<boolean> => {
    if (await this.w5.clipboard("copy")) return true;
    try {
      const doc = this.requireDoc();
      if ((this.selection.others ?? []).length > 0) {
        useUiStore.getState().pushToast("info", "Copying several ranges at once is not supported; copied the last one.");
      }
      const sel = this.selection;
      const out = await this.client.copy(doc, { sheet: this.sheet, ranges: [{ ...sel.range }], active: { ...sel.active } });
      this.lastCopy = { token: out.token, tsv: out.tsv };
      useDataStore.getState().setClip({ sheet: this.sheet, range: { ...sel.range }, token: out.token, cut: false });
      await writeClipboard(out.tsv, out.html);
      return true;
    } catch (e) {
      this.notifyError(e);
      return false;
    }
  };

  /**
   * Cut: copies and marks the range; the next paste of that clipboard
   * moves it (`moveRange`, so references follow), as in Calc.
   */
  cut = async (): Promise<void> => {
    if (await this.w5.clipboard("cut")) return;
    if (!(await this.copy())) return;
    const clip = useDataStore.getState().clip;
    if (clip) useDataStore.getState().setClip({ ...clip, cut: true });
  };

  /** Esc: forgets the pending copy/cut outline. */
  cancelClip = (): void => {
    if (useDataStore.getState().clip) useDataStore.getState().setClip(null);
  };

  onGridCopy = (): void => void this.copy();
  onGridCut = (): void => void this.cut();

  /** Pastes TSV text or an internal clipboard token at the selection. */
  pasteData(text: string, token: string | null, valuesOnly: boolean): Promise<ApplyResult | null> {
    if (!valuesOnly && this.w5.wantsChartPaste(text)) return this.w5.charts.pasteChart().then(() => null);
    if ((valuesOnly || !token) && text === "") return Promise.resolve(null);
    const target = { ...this.selection.range };
    const clip = useDataStore.getState().clip;
    if (!valuesOnly && token && clip?.cut && clip.token === token) {
      useDataStore.getState().setClip(null);
      if (clip.sheet === this.sheet) {
        const to = { r: target.r0, c: target.c0 };
        const h = clip.range.r1 - clip.range.r0;
        const w = clip.range.c1 - clip.range.c0;
        const after = { range: { r0: to.r, c0: to.c, r1: to.r + h, c1: to.c + w }, active: { ...to }, anchor: { ...to } };
        return this.apply("Move", [{ type: "moveRange", sheet: this.sheet, from: { ...clip.range }, to }], after);
      }
      // Across sheets: paste, then clear the source.
      return this.apply("Move", [
        { type: "paste", sheet: this.sheet, target, payload: { type: "internal", token } },
        { type: "clear", sheet: clip.sheet, ranges: [{ ...clip.range }], what: "all" },
      ]);
    }
    if (valuesOnly && token) {
      return this.apply("Paste values", [{ type: "pasteSpecial", sheet: this.sheet, target, payload: { type: "internal", token }, options: pasteOptions({ what: "values", transpose: false, skipEmpty: false, operation: "none" }) }], undefined, true);
    }
    const payload = token && !valuesOnly ? { type: "internal" as const, token } : { type: "tsv" as const, text };
    return this.apply(valuesOnly ? "Paste values" : "Paste", [{ type: "paste", sheet: this.sheet, target, payload }], undefined, true);
  }

  /** Reads the clipboard (system if allowed, else the last copy in this window). */
  private async readClipboard(): Promise<{ text: string | null; token: string | null }> {
    let text: string | null = null;
    let token: string | null = null;
    const clip = typeof navigator === "undefined" ? undefined : navigator.clipboard;
    const read = async (): Promise<void> => {
      if (clip && typeof clip.read === "function") {
        for (const item of await clip.read()) {
          if (item.types.includes("text/html")) token = /data-tally-clip="([^"]+)"/.exec(await (await item.getType("text/html")).text())?.[1] ?? token;
          if (item.types.includes("text/plain")) text = await (await item.getType("text/plain")).text();
        }
      } else if (clip) text = await clip.readText();
    };
    try {
      // A permission prompt can leave the read pending: give up after a moment.
      await Promise.race([read(), new Promise((_, reject) => setTimeout(() => reject(new Error("clipboard timeout")), CLIPBOARD_TIMEOUT_MS))]);
    } catch {
      text = null;
      token = null;
    }
    if (text === null && this.lastCopy) {
      text = this.lastCopy.tsv;
      token = this.lastCopy.token;
    }
    return { text, token };
  }

  onGridPaste = (data: DataTransfer | null, opts?: { plain: boolean }): void => {
    const text = data?.getData("text/plain") ?? "";
    const html = data?.getData("text/html") ?? "";
    let token = /data-tally-clip="([^"]+)"/.exec(html)?.[1] ?? null;
    // Some webviews drop the HTML flavour: the same text as our last copy is that copy.
    if (token === null && this.lastCopy && text !== "" && text === this.lastCopy.tsv) token = this.lastCopy.token;
    void this.pasteData(text, token, opts?.plain ?? false);
  };

  /**
   * Paste from a menu (no DOM paste event): reads the system clipboard if
   * the browser allows, else uses the last copy made in this window.
   */
  pasteFromClipboard = async (valuesOnly: boolean): Promise<void> => {
    const { text, token } = await this.readClipboard();
    if (text === null) {
      useUiStore.getState().pushToast("info", "Use ⌘V to paste (the clipboard is not readable here).");
      return;
    }
    await this.pasteData(text, token, valuesOnly);
    this.focusGrid();
  };

  /** Paste Special… (values / formats / transpose / skip empty / arithmetic). */
  pasteSpecial = async (choice: PasteSpecialChoice): Promise<void> => {
    const { text, token } = await this.readClipboard();
    if (text === null && !token) {
      useUiStore.getState().pushToast("info", "Copy something first.");
      return;
    }
    const payload = token ? { type: "internal" as const, token } : { type: "tsv" as const, text: text ?? "" };
    const target = { ...this.selection.range };
    await this.apply("Paste special", [{ type: "pasteSpecial", sheet: this.sheet, target, payload, options: pasteOptions(choice) }], undefined, true);
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
    useDataStore.getState().setFilter(null);
    useV3Store.getState().reset();
    void this.refreshActive();
    void this.refreshFilter();
    void this.v3.refreshSheet();
    this.w5.refreshSheet();
    this.w56.onSheet();
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
      if (format !== null) useUiStore.getState().pushToast("info", `Exported ${res.title}`);
    } catch (e) {
      this.notifyError(e);
    }
  };

  closeDocument = async (): Promise<void> => {
    await this.newWorkbook();
  };
  // -- v0.2: data region, sort, AutoFilter ---------------------------------------

  /** The active sheet's tab name. */
  sheetName(sheet: number = this.sheet): string {
    return this.info?.sheets.find((s) => s.id === sheet)?.name ?? "Sheet1";
  }

  /** Merged areas of the active sheet (from the grid's layout). */
  merges(): RangeDto[] {
    return useDocStore.getState().source?.getLayout(this.sheet).merges ?? [];
  }

  /**
   * The range a sort or filter acts on: the AutoFilter area when the active
   * cell is in it, a multi-cell selection (clipped to the used area), else
   * the current region around the active cell (Ctrl+*).
   */
  async dataRegion(): Promise<RangeDto | null> {
    const doc = this.requireDoc();
    const sheet = this.sheet;
    const sel = this.selection;
    const af = useDataStore.getState().filter?.range;
    const a = sel.active;
    if (af && a.r >= af.r0 && a.r <= af.r1 && a.c >= af.c0 && a.c <= af.c1) return { ...af };
    const layout = await this.client.sheetLayout(doc, sheet);
    const used = layout.used;
    if (!used) return null;
    const r = sel.range;
    const merge = this.merges().find((m) => m.r0 === r.r0 && m.c0 === r.c0 && m.r1 === r.r1 && m.c1 === r.c1);
    if (!(r.r0 === r.r1 && r.c0 === r.c1) && !merge) {
      const out = { r0: Math.max(r.r0, used.r0), c0: Math.max(r.c0, used.c0), r1: Math.min(r.r1, used.r1), c1: Math.min(r.c1, used.c1) };
      return out.r0 <= out.r1 && out.c0 <= out.c1 ? out : null;
    }
    const { range } = cappedRange(used);
    const [block] = await this.client.cells(doc, sheet, [range]);
    const map = new Map<string, CellView>();
    for (const v of block?.cells ?? []) map.set(`${v.r},${v.c}`, v);
    return currentRegion((rr, cc) => map.get(`${rr},${cc}`), a, range);
  }

  /** Header texts of `range`'s first row (for the Sort dialog), "Column X" when blank. */
  private async headerLabels(range: RangeDto, hasHeader: boolean): Promise<string[]> {
    const letters = (c: number) => `Column ${a1Text({ r0: 0, c0: c, r1: 0, c1: c }).replace(/\d+$/, "")}`;
    const out: string[] = [];
    let row: CellView[] = [];
    if (hasHeader) {
      const [block] = await this.client.cells(this.requireDoc(), this.sheet, [{ ...range, r1: range.r0 }]);
      row = block?.cells ?? [];
    }
    for (let c = range.c0; c <= range.c1; c++) {
      const t = row.find((v) => v.c === c)?.text.trim();
      out.push(t ? t : letters(c));
    }
    return out;
  }

  private async hasHeader(range: RangeDto): Promise<boolean> {
    if (range.r1 <= range.r0) return false;
    const af = useDataStore.getState().filter?.range;
    if (af && af.r0 === range.r0 && af.c0 === range.c0 && af.r1 === range.r1 && af.c1 === range.c1) return true;
    try {
      return await this.client.detectHeader(this.requireDoc(), this.sheet, range);
    } catch {
      return false;
    }
  }

  /** Toolbar Sort / Data ▸ Sort Ascending|Descending: the data region by the active column. */
  sortByActive = async (ascending: boolean): Promise<void> => {
    try {
      const region = await this.dataRegion();
      if (!region || region.r0 === region.r1) {
        useUiStore.getState().pushToast("info", "Select a table (or a cell in one) to sort.");
        return;
      }
      const hasHeader = await this.hasHeader(region);
      const col = Math.min(region.c1, Math.max(region.c0, this.selection.active.c));
      await this.sortWith({ range: region, keys: [{ col, ascending }], hasHeader }, ascending ? "Sort ascending" : "Sort descending");
    } catch (e) {
      this.notifyError(e);
    }
  };

  /** Opens Sort… with the region, `detect_header` and the active column as the first key. */
  openSortDialog = async (): Promise<void> => {
    try {
      const region = await this.dataRegion();
      if (!region) {
        useUiStore.getState().pushToast("info", "Select a table (or a cell in one) to sort.");
        return;
      }
      const hasHeader = await this.hasHeader(region);
      const labels = await this.headerLabels(region, true);
      const col = Math.min(region.c1, Math.max(region.c0, this.selection.active.c));
      useUiStore.getState().openDialog({ kind: "sort", range: region, hasHeader, keys: [{ col, ascending: true }], labels });
    } catch (e) {
      this.notifyError(e);
    }
  };

  sortWith = async (spec: SortSpec, label = "Sort"): Promise<ApplyResult | null> => {
    const sel = { range: { ...spec.range }, active: { r: spec.range.r0, c: spec.range.c0 }, anchor: { r: spec.range.r0, c: spec.range.c0 } };
    return this.apply(label, [sortOp(this.sheet, spec)], sel);
  };

  async refreshFilter(): Promise<void> {
    const info = this.info;
    if (!info) return;
    const sheet = this.sheet;
    try {
      const f = await this.client.filterInfo(info.doc, sheet);
      if (sheet === this.sheet && this.info?.doc === info.doc) useDataStore.getState().setFilter(f.range ? f : null);
    } catch {
      if (sheet === this.sheet) useDataStore.getState().setFilter(null);
    }
  }

  async refreshNames(): Promise<void> {
    const info = this.info;
    if (!info) return;
    try {
      const names = await this.client.names(info.doc);
      if (this.info?.doc === info.doc) useDataStore.getState().setNames(names);
    } catch {
      useDataStore.getState().setNames([]);
    }
  }

  /** Toolbar Filter: turns the AutoFilter on (over the data region) or off. */
  toggleAutoFilter = async (): Promise<void> => {
    try {
      const sheet = this.sheet;
      if (useDataStore.getState().filter?.range) {
        await this.apply("Remove filter", [{ type: "setAutoFilter", sheet, range: null }]);
      } else {
        const region = await this.dataRegion();
        if (!region || region.r0 === region.r1) {
          useUiStore.getState().pushToast("info", "Select a table with a header row to filter.");
          return;
        }
        await this.apply("Filter", [{ type: "setAutoFilter", sheet, range: region }]);
      }
      await this.refreshFilter();
    } catch (e) {
      this.notifyError(e);
    }
  };

  setColumnFilter = async (col: number, filter: ColumnFilter | null): Promise<void> => {
    await this.apply(filter ? "Filter" : "Clear filter", [columnFilterOp(this.sheet, col, filter)]);
    await this.refreshFilter();
  };

  /** Data ▸ Clear Filters: every column's filter (the AutoFilter stays). */
  clearFilters = async (): Promise<void> => {
    const f = useDataStore.getState().filter;
    const ops = (f?.columns ?? []).filter((c) => c.active).map((c) => columnFilterOp(this.sheet, c.col, null));
    if (ops.length === 0) return;
    await this.apply("Clear filters", ops);
    await this.refreshFilter();
  };

  /** The ▾ of a filter header was clicked. */
  onFilterButton = (col: number, rect: { left: number; bottom: number }): void => {
    useUiStore.getState().openContextMenu(null);
    useUiStore.getState().openFilterPopover({ col, x: rect.left, y: rect.bottom + 4 });
  };

  /**
   * ⌘K "Show only rows where <column> <op> <value>" (`filter.byColumn`):
   * turns the AutoFilter on if needed and sets the condition.
   */
  filterByColumn = async (args: { column?: string; op?: string; value?: string } | undefined): Promise<void> => {
    try {
      const letters = (args?.column ?? "").toUpperCase();
      let col = -1;
      for (const ch of letters) col = (col + 1) * 26 + (ch.charCodeAt(0) - 64) - 1;
      if (col < 0) return;
      const ops: Record<string, FilterOp> = { "=": "eq", "==": "eq", "<>": "ne", "!=": "ne", ">": "gt", ">=": "ge", "<": "lt", "<=": "le" };
      const op = ops[args?.op ?? "="] ?? "eq";
      const value = args?.value ?? "";
      if (!useDataStore.getState().filter?.range) {
        const sel = this.selection;
        useSelectionStore.getState().setSelection({ ...sel, active: { r: sel.active.r, c: col } });
        await this.toggleAutoFilter();
      }
      const af = useDataStore.getState().filter?.range;
      if (!af || col < af.c0 || col > af.c1) {
        useUiStore.getState().pushToast("info", "That column is outside the table.");
        return;
      }
      await this.setColumnFilter(col, { values: null, condition: { op, value } });
    } catch (e) {
      this.notifyError(e);
    }
  };

  // -- v0.2: merge, freeze, hide, borders ------------------------------------------

  /** Whether the selection is exactly one merged area. */
  selectionIsMerged(): boolean {
    const r = this.selection.range;
    return this.merges().some((m) => m.r0 <= r.r0 && m.c0 <= r.c0 && m.r1 >= r.r1 && m.c1 >= r.c1);
  }

  /** Merge (warns first when cells other than the top-left have content). */
  mergeCells = async (): Promise<void> => {
    try {
      const doc = this.requireDoc();
      const sheet = this.sheet;
      const range = { ...this.selection.range };
      if (range.r0 === range.r1 && range.c0 === range.c1) {
        useUiStore.getState().pushToast("info", "Select two or more cells to merge.");
        return;
      }
      const [block] = await this.client.cells(doc, sheet, [range]);
      const filled = (block?.cells ?? []).filter((v) => v.kind !== "empty" && v.text !== "");
      const others = filled.filter((v) => v.r !== range.r0 || v.c !== range.c0);
      const tl = { r: range.r0, c: range.c0 };
      const go = (): void => {
        void this.apply("Merge cells", [{ type: "mergeCells", sheet, range }], { range, active: tl, anchor: tl });
      };
      if (others.length === 0) {
        go();
        return;
      }
      // §51: AOO asks whether to move the hidden cells' contents into the first cell.
      if (!this.w56.mergePrompt(range, filled, go)) go();
    } catch (e) {
      this.notifyError(e);
    }
  };

  unmergeCells = (): Promise<ApplyResult | null> => this.apply("Unmerge cells", [{ type: "unmergeCells", sheet: this.sheet, range: { ...this.selection.range } }]);

  toggleMerge = async (): Promise<void> => {
    if (this.selectionIsMerged()) await this.unmergeCells();
    else await this.mergeCells();
  };

  /** Freezes the rows above and/or the columns left of the active cell. */
  freeze = async (what: "both" | "rows" | "cols"): Promise<void> => {
    const { rows, cols } = freezeAt(this.selection.active, what);
    if (rows === 0 && cols === 0) {
      const where = what === "rows" ? "below the rows" : what === "cols" ? "right of the columns" : "below and right of what";
      useUiStore.getState().pushToast("info", `Select a cell ${where} you want to keep in view.`);
      return;
    }
    await this.apply("Freeze panes", [{ type: "setFrozen", sheet: this.sheet, rows, cols }]);
  };

  unfreeze = (): Promise<ApplyResult | null> => this.apply("Unfreeze panes", [{ type: "setFrozen", sheet: this.sheet, rows: 0, cols: 0 }]);

  isFrozen(): boolean {
    const f = useDocStore.getState().source?.getLayout(this.sheet).frozen;
    return !!f && (f.r > 0 || f.c > 0);
  }

  /** Hide/show the selected rows or columns. Showing a single row/column also reveals hidden neighbours. */
  setHidden = async (axis: "row" | "col", hidden: boolean): Promise<void> => {
    const r = this.selection.range;
    let start = axis === "row" ? r.r0 : r.c0;
    let end = axis === "row" ? r.r1 : r.c1;
    if (!hidden) {
      const layout = useDocStore.getState().source?.getLayout(this.sheet);
      const runs = (axis === "row" ? layout?.rowRuns : layout?.colRuns) ?? [];
      for (const run of runs) {
        if (!run.hidden) continue;
        if (run.end === start - 1) start = run.start;
        if (run.start === end + 1) end = run.end;
      }
    }
    const what = axis === "row" ? (hidden ? "Hide rows" : "Show rows") : hidden ? "Hide columns" : "Show columns";
    const op: Op = axis === "row" ? { type: "setRowsHidden", sheet: this.sheet, start, end, hidden } : { type: "setColsHidden", sheet: this.sheet, start, end, hidden };
    await this.apply(what, [op]);
  };

  /** Borders on every selected range. */
  applyBorders = (preset: BorderPreset, line: Parameters<typeof bordersPatch>[1]): Promise<ApplyResult | null> =>
    this.applyPatch(preset === "none" ? "Remove borders" : "Borders", { ...EMPTY_PATCH, borders: bordersPatch(preset, line) });

  // -- v0.2: names --------------------------------------------------------------------

  /** Define Name… prefilled with the selection (and a name from the header above, if any). */
  openDefineName = (): void => {
    useUiStore.getState().openDialog({
      kind: "defineName",
      name: "",
      refersTo: absoluteRef(this.sheetName(), this.selection.range),
      scope: null,
      replacing: null,
    });
  };

  /** Defines (or redefines) a name; `replacing` is the entry being edited (renamed or re-scoped). */
  defineName = async (name: string, scope: number | null, refersTo: string, replacing: NameInfo | null): Promise<boolean> => {
    const ops: Op[] = [];
    if (replacing && (replacing.name.toLowerCase() !== name.toLowerCase() || replacing.scope !== scope)) {
      ops.push({ type: "deleteName", name: replacing.name, scope: replacing.scope });
    }
    ops.push({ type: "defineName", name, scope, refersTo: refersTo.trim().replace(/^=/, "") });
    const res = await this.apply(replacing ? "Edit name" : "Define name", ops);
    await this.refreshNames();
    return res !== null && res.errors.length === 0;
  };

  deleteName = async (n: NameInfo): Promise<void> => {
    await this.apply("Delete name", [{ type: "deleteName", name: n.name, scope: n.scope }]);
    await this.refreshNames();
  };

  // -- v0.2: find & replace --------------------------------------------------------------

  /** Find next (`1`) / previous (`-1`): searches when the query changed, then steps through the hits. */
  findStep = async (dir: 1 | -1): Promise<void> => {
    try {
      const doc = this.requireDoc();
      const st = useDataStore.getState();
      const { text, options } = st.find;
      if (text === "") return;
      const key = JSON.stringify([text, options, this.sheet, options.allSheets ? null : this.sheet]);
      let { hits, index } = st.find;
      if (st.find.searched !== key) {
        const res = await this.client.find(doc, findQuery(text, options, this.sheet, { ...this.selection.active }));
        hits = res.hits;
        // Hits come rotated to start after the active cell.
        const a = this.selection.active;
        const last = hits.length - 1;
        const atActive = (i: number) => hits[i]?.sheet === this.sheet && hits[i]?.r === a.r && hits[i]?.c === a.c;
        index = dir > 0 ? 0 : atActive(last) && last > 0 ? last - 1 : last;
        useDataStore.getState().patchFind({ hits, truncated: res.truncated, searched: key, index });
      } else {
        index = stepHit(index, hits.length, dir);
        useDataStore.getState().patchFind({ index });
      }
      const hit = hits[index];
      if (!hit) return;
      if (hit.sheet !== this.sheet) this.showSheet(hit.sheet, false);
      const cell = { r: hit.r, c: hit.c };
      const sel = { range: { r0: cell.r, c0: cell.c, r1: cell.r, c1: cell.c }, active: cell, anchor: cell };
      useSelectionStore.getState().setSelection(sel);
      this.grid?.scrollCellIntoView(cell);
    } catch (e) {
      this.notifyError(e);
    }
  };

  /** Replace All; returns how many cells changed (hits found minus refusals). */
  replaceAll = async (text: string, replace: string): Promise<number> => {
    try {
      const doc = this.requireDoc();
      const { options } = useDataStore.getState().find;
      const query = findQuery(text, options, this.sheet, null);
      const before = await this.client.find(doc, { ...query, inFormulas: true });
      const res = await this.apply("Replace all", [{ type: "replaceAll", sheet: options.allSheets ? null : this.sheet, query, replace }]);
      if (!res) return 0;
      const n = res.replaced ?? Math.max(0, before.hits.length - res.errors.length);
      const skipped = res.errors.length;
      useDataStore.getState().patchFind({ searched: null, hits: [], index: -1 });
      useUiStore
        .getState()
        .pushToast("info", n === 0 && skipped === 0 ? `No cells match “${text}”.` : `Replaced ${n === 1 ? "1 cell" : `${n} cells`}${skipped > 0 ? ` (${skipped} skipped)` : ""}.`);
      return n;
    } catch (e) {
      this.notifyError(e);
      return 0;
    }
  };

  // -- v0.2: arrays, fill, recalc, sheets ---------------------------------------------------

  /** ⌃⇧↩ in an editor: the formula as an array over the selection (or the cell's existing block). */
  commitArray = async (): Promise<void> => {
    const st = useEditStore.getState();
    const edit = st.edit;
    if (!edit) return;
    const origin = st.origin;
    const active = st.active;
    const block = active && active.cell.r === edit.cell.r && active.cell.c === edit.cell.c ? active.arrayRange : null;
    const sel = this.selection.range;
    const inSel = edit.cell.r >= sel.r0 && edit.cell.r <= sel.r1 && edit.cell.c >= sel.c0 && edit.cell.c <= sel.c1;
    const range = block ?? (inSel ? { ...sel } : { r0: edit.cell.r, c0: edit.cell.c, r1: edit.cell.r, c1: edit.cell.c });
    const op = arrayFormulaOp(this.sheet, range, this.w56.commitText(edit.text));
    if (!op) {
      useUiStore.getState().pushToast("info", "An array formula starts with = (type it, then press ⌃⇧↩).");
      return;
    }
    st.end();
    if (origin === "bar") this.focusGrid();
    const tl = { r: range.r0, c: range.c0 };
    await this.apply("Array formula", [op], { range, active: tl, anchor: tl });
  };

  /** ⌘D / ⌘R. */
  fillDir = async (dir: "down" | "right"): Promise<void> => {
    const plan = fillDownRight(this.selection.range, dir);
    if (!plan) return;
    await this.onFill(plan.source, plan.target);
  };

  /** ⇧⌘F9: hard recalculation. */
  recalcAll = async (): Promise<void> => {
    try {
      this.handleResult(await this.client.recalcAll(this.requireDoc()));
      useUiStore.getState().pushToast("info", "Recalculated every formula.");
    } catch (e) {
      this.notifyError(e);
    }
  };

  /** Moves the active sheet one tab left/right. */
  moveSheetBy = async (delta: -1 | 1): Promise<void> => {
    const info = this.info;
    if (!info) return;
    const i = info.sheets.findIndex((s) => s.id === this.sheet);
    const to = i + delta;
    if (i < 0 || to < 0 || to >= info.sheets.length) return;
    await this.moveSheet(this.sheet, to);
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
