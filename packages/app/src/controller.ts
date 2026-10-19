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
  type ShellClient,
  type Op,
  type RangeDto,
  type StylePatch,
  type Transaction,
  type WorkbookInfo,
} from "@tally/bridge";
import { cellSelection, EngineClientSource, type CommitMove, type Dir, type Grid as GridCore, type Selection } from "@tally/grid";
import { CommandRegistry } from "./registry";
import { EMPTY_PATCH } from "./formats";
import { useDocStore } from "./stores/docStore";
import { useEditStore } from "./stores/editStore";
import { fromSelectionDto, toSelectionDto, useSelectionStore } from "./stores/selectionStore";
import { toastWarnings, useUiStore } from "./stores/uiStore";
import { detectSumRange, sumFormula, sumScanRanges } from "./sumRange";

export interface ControllerOptions {
  /** Inside Tauri: the native menu owns its accelerators. */
  nativeMenu: boolean;
}

const MOVES: Record<Dir, [number, number]> = { up: [-1, 0], down: [1, 0], left: [0, -1], right: [0, 1] };
const LAST_ROW = 1_048_575;
const LAST_COL = 16_383;

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

  /** Bold/italic toggle: the active cell's current value decides. */
  async toggleStyle(field: "bold" | "italic"): Promise<void> {
    try {
      const doc = this.requireDoc();
      const sel = this.selection;
      const cell = useDocStore.getState().source?.getCell(sel.active.r, sel.active.c);
      const [style] = await this.client.styles(doc, [cell?.style ?? 0]);
      const patch: StylePatch = { ...EMPTY_PATCH, [field]: !(style?.[field] ?? false) };
      await this.apply(field === "bold" ? "Bold" : "Italic", [
        { type: "applyStyle", sheet: this.sheet, ranges: [{ ...sel.range }], patch },
      ]);
    } catch (e) {
      this.notifyError(e);
    }
  }

  setNumberFormat(code: string, label: string): Promise<ApplyResult | null> {
    return this.setNumberFormatOn([{ ...this.selection.range }], code, label);
  }

  /** Applies a number format to explicit ranges of the active sheet. */
  setNumberFormatOn(ranges: RangeDto[], code: string, label: string): Promise<ApplyResult | null> {
    return this.apply(label, [{ type: "setNumberFormat", sheet: this.sheet, ranges: ranges.map((r) => ({ ...r })), code }]);
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
    void this.apply("Delete", [{ type: "clear", sheet: this.sheet, ranges: [{ ...range }], what: "contents" }]);
  };

  clearSelection = (): void => this.deleteContents(this.selection.range);

  resize = (axis: "col" | "row", start: number, end: number, px: number): void => {
    const op: Op =
      axis === "col"
        ? { type: "setColWidth", sheet: this.sheet, start, end, px }
        : { type: "setRowHeight", sheet: this.sheet, start, end, px };
    void this.apply(axis === "col" ? "Column width" : "Row height", [op]);
  };

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
      const out = await this.client.copy(doc, toSelectionDto(this.selection, this.sheet));
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

  onGridPaste = (data: DataTransfer | null): void => {
    const text = data?.getData("text/plain") ?? "";
    const html = data?.getData("text/html") ?? "";
    const token = /data-tally-clip="([^"]+)"/.exec(html)?.[1];
    if (!token && text === "") return;
    const target = { ...this.selection.range };
    void this.apply("Paste", [
      {
        type: "paste",
        sheet: this.sheet,
        target,
        payload: token ? { type: "internal", token } : { type: "tsv", text },
      },
    ]);
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
      if (format === "csv") useUiStore.getState().pushToast("info", `Exported ${res.title}`);
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
