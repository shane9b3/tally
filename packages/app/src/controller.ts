/**
 * AppController: the only place that talks to the {@link EngineClient}.
 * Components call its methods (or run CommandRegistry commands); results
 * flow back into the zustand stores and the grid's data source.
 */
import { fontFamilyPatch, fontSizePatch } from "./fonts/fontModel";
import {
  CANCELLED,
  CSV_OPTIONS,
  isApiError,
  type ApiError,
  type ApplyResult,
  type CellPos,
  type CellView,
  type ColumnFilter,
  type Direction,
  type DocId,
  type EngineClientV3,
  type FileFormat,
  type FillMode,
  type FilterOp,
  type NameInfo,
  type HAlign,
  type ShellClient,
  type Op,
  type PasteOptions,
  type PastePayload,
  type RangeDto,
  type StyleDto,
  type StylePatch,
  type Transaction,
  type WorkbookInfo,
} from "@tally/bridge";
import {
  cellAddress,
  cellSelection,
  drawStyleId,
  stepWithin,
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
  DEFAULT_PASTE_SPECIAL,
  pasteAsksOverwrite,
  pasteOptions,
  sortOp,
  stepHit,
  a1 as a1Text,
  type BorderPreset,
  type PasteSpecialChoice,
  type SortSpec,
} from "./dataOps";
import { useDataStore } from "./stores/dataStore";
import { useW56Store } from "./data/store";
import { applyCompletion, applyPoint, assistFor, assistKey } from "./formulaEdit";
import { isFormulaText } from "@tally/formula-ts";
import { chipColorsFor } from "./palette";
import { useDocStore } from "./stores/docStore";
import { GROUP_OPS } from "@tally/bridge";
import { extendStep, moveBlockSteps, nextActiveAfterDelete, selectedIds, stepVisible, txGroup } from "./sheetGroup";
import { isPendingEdit, useEditStore } from "./stores/editStore";
import { useFunctionsStore } from "./stores/functionsStore";
import { fromSelectionDto, selectionRanges, toSelectionDto, useSelectionStore } from "./stores/selectionStore";
import { openNotices, toastWarnings, useUiStore } from "./stores/uiStore";

/** The `menu-action` id prefix of a file the OS asked to open (`dialogs::OS_OPEN_ACTION`). */
export const OS_OPEN_ACTION = "os.open:";
import { detectSumRange, sumFormula, sumScanRanges } from "./sumRange";
import { V3Features } from "./features";
import { useV3Store } from "./stores/v3Store";
import { W5Features } from "./objects/w5Features";
import { guardDecision, isSequence, proceedAfter, useSavePromptStore, type CloseKind, type CloseShell } from "./unsaved";
import { TabsFeature, useTabsStore, type TabShell } from "./tabs";
import { canReuseWindow, type WindowShell } from "./windows";
import { W56Features } from "./data/w56";
import { t } from "./i18n";
import { useCsvPromptStore } from "./csvExport";
import { displayTitle, lockedMessage, parseLock, readOnlyGate, ReadOnlyFeature } from "./readOnly";

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

/** Commands that stay enabled while a formula edit shows another sheet (editor undo, zoom, sheet stepping). */
const PICKING_COMMANDS: ReadonlySet<string> = new Set(["edit.undo", "edit.redo", "view.zoomIn", "view.zoomOut", "view.zoomReset", "sheet.next", "sheet.prev"]);

function errorOf(e: unknown): ApiError {
  if (isApiError(e)) return e;
  return { code: "internal", message: e instanceof Error ? e.message : String(e) };
}

/** Ops that never go with a sheet group (protection is per sheet / per document). */
const NO_GROUP_OPS: ReadonlySet<string> = new Set(["protectSheet", "unprotectSheet", "protectDocument", "unprotectDocument"]);

export class AppController {
  readonly registry = new CommandRegistry();
  grid: GridCore | null = null;
  private infoSeq = 0;
  /** The committed input of the cell being edited (skip no-op commits). */
  private editOriginal: string | null = null;
  /** Bumped by every startEdit: a late `cellEditInfo` reply only fills in its own session. */
  private editSession = 0;
  private readonly offs: (() => void)[] = [];
  /** The last internal copy (context-menu paste when the system clipboard is unreadable). */
  private lastCopy: { token: string; tsv: string } | null = null;
  /** The unsaved-changes guard is running (prompt open or its Save in progress). */
  private guarding = false;
  /** The edited state, document and title last reported to the shell (`set_document_edited`). */
  private reportedEdited: boolean | null = null;
  private reportedDoc: DocId | null = null;
  private reportedTitle: string | null = null;
  /** The blank workbook this window started with, until its first change (window reuse, windows.ts). */
  private pristineDoc: DocId | null = null;
  /** A quit asked while this window's own prompt was up (it answers for both). */
  private quitWaiting = false;
  /** A theme change applied from another window (`prefs-changed`) is not broadcast back. */
  private remoteTheme = false;
  /** Typing commits in flight (still "edited" until the engine marks the doc dirty). */
  private committing = 0;
  /** The last typing commit's apply (the guard waits for it before reading `dirty`). */
  private lastCommit: Promise<unknown> = Promise.resolve();

  /** v0.3 features (conditional formatting, validation, notes, links, protection, page setup…). */
  readonly v3: V3Features;
  /** Wave 5 (W5-5): charts and pictures, print/PDF, split window, detective, view presets. */
  readonly w5: W5Features;
  /** Wave 5 (W5-6): DataPilot, data tools, styles, options, assists, navigator, i18n. */
  readonly w56: W56Features;
  /** Document tabs (tabs.ts): the strip's and the Window menu's actions. */
  readonly tabs: TabsFeature;
  /** Read-only mode and document locks (readOnly.ts). */
  readonly readOnly: ReadOnlyFeature;

  constructor(
    readonly client: EngineClientV3,
    readonly shell: ShellClient,
    readonly opts: ControllerOptions,
  ) {
    this.v3 = new V3Features(this);
    this.w5 = new W5Features(this);
    this.w56 = new W56Features(this);
    this.tabs = new TabsFeature(this);
    this.readOnly = new ReadOnlyFeature(this);
    // A read-only document runs only viewing, copying, finding, printing
    // and Save As (AOO disables the editing slots).
    this.registry.addGate(readOnlyGate(() => this.readOnly.active));
    // While a formula picks references on another sheet only editing and
    // viewing commands run: everything else would act on the shown sheet.
    this.registry.addGate({
      active: () => this.editingElsewhere,
      allow: (id) => PICKING_COMMANDS.has(id) || id.startsWith("window."),
      reason: () => t("msg.reason.finishEditing", "Finish editing the cell first"),
    });
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
        // A file the OS asked to open (Finder, Open With, a second launch):
        // opened like a recent file, in place of an untouched blank workbook.
        if (e.id.startsWith(OS_OPEN_ACTION)) {
          void this.openRecent(e.id.slice(OS_OPEN_ACTION.length));
          return;
        }
        if (this.registry.run(e.id)) return;
        // The native menu has no enabled state: say why a disabled command did nothing.
        const reason = this.registry.has(e.id) ? this.registry.reasonFor(e.id) : undefined;
        if (reason) useUiStore.getState().pushToast("info", reason);
        else console.warn(`menu-action: unknown or disabled command ${e.id}`);
      }),
      this.client.onDocChanged((e) => {
        if (e.doc === useDocStore.getState().info?.doc) {
          useDocStore.getState().applyResult(e.result);
          void this.refreshActive();
        }
      }),
      this.w56.attach(),
      useDocStore.subscribe(() => this.syncDocumentEdited()),
      useEditStore.subscribe(() => this.syncDocumentEdited()),
      this.closeShell.onCloseRequested?.((kind) => void this.closeRequested(kind)) ?? (() => undefined),
      this.windowShell.onPrefsChanged?.((e) => this.prefsChangedElsewhere(e.theme)) ?? (() => undefined),
      (this.shell as unknown as TabShell).onTabsChanged?.((state) => useTabsStore.getState().setState(state)) ?? (() => undefined),
      // This webview became the visible tab: give the grid the keyboard.
      useTabsStore.subscribe((s, prev) => {
        const self = s.self;
        if (self !== null && s.state?.active === self && prev.state?.active !== self) this.focusGrid();
      }),
      useUiStore.subscribe((s, prev) => {
        if (s.themePref !== prev.themePref && !this.remoteTheme) this.broadcastPrefs(s.themePref);
      }),
    );
    this.syncDocumentEdited();
    return () => {
      this.offs.splice(0).forEach((f) => f());
      useDocStore.getState().source?.dispose();
    };
  }

  /**
   * Shows this window's first document: the one the desktop shell opened the
   * window for (`window_init`), else a new blank workbook (or `template`).
   * Only the session's first window offers the crash recovery.
   */
  async start(template?: string | null): Promise<void> {
    this.client.listFunctions().then(
      (fns) => this.w56.setFunctions(fns),
      () => undefined,
    );
    let offerRecovery = true;
    let ask: CloseKind | null = null;
    let openPaths: readonly string[] = [];
    try {
      const init = this.windowShell.windowInit ? await this.windowShell.windowInit() : null;
      if (init) {
        offerRecovery = init.offerRecovery;
        ask = init.ask ?? null;
        openPaths = init.openPaths ?? [];
        if (init.label !== undefined) useTabsStore.getState().setSelf(init.label);
        if (init.tabs !== undefined) useTabsStore.getState().setState(init.tabs);
      }
      if (init?.doc) {
        this.load(init.doc);
      } else {
        const blank = template === undefined || template === null;
        this.load(await this.client.newWorkbook(template ?? null), blank);
      }
    } catch (e) {
      this.notifyError(e);
    }
    // Tally was launched by opening files: the first replaces the blank
    // workbook just shown (it is untouched), the others open beside it.
    for (const path of openPaths) await this.openRecent(path);
    if (offerRecovery) void this.w56.offerRecovery();
    // A close sequence was asking this document when its webview reloaded:
    // ask again, so the sequence does not wait for an answer forever.
    if (ask) void this.closeRequested(ask);
  }

  /**
   * Shows a workbook (replacing the current one). `pristine`: a blank
   * workbook this window starts with, which a later Open may replace.
   */
  load(info: WorkbookInfo, pristine = false): void {
    this.pristineDoc = pristine ? info.doc : null;
    const doc = useDocStore.getState();
    const prev = doc.info;
    doc.source?.dispose();
    if (prev && prev.doc !== info.doc) void this.client.close(prev.doc).catch(() => undefined);
    const source = new EngineClientSource(this.client, info.doc, info.activeSheet, {
      onError: (e) => console.warn("tally: fetch failed", e),
    });
    useEditStore.getState().reset();
    useSelectionStore.getState().reset();
    // A new view: detective Fill Mode ends (a sheet switch keeps it, as in AOO).
    this.w5.detective.endFillMode();
    doc.setInfo(info);
    useDocStore.getState().setActiveSheet(info.activeSheet);
    doc.setSource(source);
    useDataStore.getState().reset();
    useV3Store.getState().reset();
    toastWarnings(openNotices(info));
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
    // Someone else's lock (a Save As target): AOO's wording, not the JSON.
    const lock = parseLock(err);
    if (lock) {
      useUiStore.getState().pushToast("error", lockedMessage(lock));
      return;
    }
    useUiStore.getState().pushToast("error", friendlyError(err.code, err.message));
  };

  requireDoc(): number {
    const info = this.info;
    if (!info) throw { code: "not_found", message: t("msg.error.noWorkbook", "No workbook is open.") } satisfies ApiError;
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
    const elsewhere = this.editingElsewhere;
    const groupBefore = [...useDocStore.getState().groupSheets];
    useDocStore.getState().source?.applyResult(res);
    useDocStore.getState().applyResult(res);
    if (!this.settleEditAfterResult(before, elsewhere, groupBefore)) {
      const after = this.sheet;
      if (after !== before) this.showSheet(after, false);
    }
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

  /**
   * After a result changed the sheets (from another window, an undo…) while
   * a formula edit is open: an edit whose own sheet is gone is cancelled; a
   * pick on another sheet stays there when that sheet still exists, else
   * goes back to the edited cell's sheet with the edit open. Returns true
   * when it settled the shown sheet itself.
   */
  private settleEditAfterResult(shownBefore: number, elsewhere: boolean, groupBefore: number[]): boolean {
    const st = useEditStore.getState();
    if (!st.edit || st.sheet === null) return false;
    if (!this.sheetExists(st.sheet)) {
      this.pickHome = null;
      useEditStore.getState().end();
      return false;
    }
    if (!elsewhere) return false;
    const shown = this.info?.sheets.find((s) => s.id === shownBefore);
    if (shown && !shown.hidden) {
      // Keep picking there; the result may have moved the active sheet or pruned the group.
      if (this.sheet !== shownBefore) {
        useDocStore.setState({ activeSheet: shownBefore, groupSheets: groupBefore });
        useDocStore.getState().source?.setSheet(shownBefore);
      }
      return true;
    }
    // The shown sheet is gone: back to the edited cell's sheet, still editing.
    if (!this.returnToEditSheet()) {
      this.viewSheet(st.sheet, cellSelection(st.edit.cell), null);
      this.syncActiveSheet(st.sheet);
    }
    return true;
  }

  /**
   * Applies ops as one undoable transaction. With several sheets selected
   * a group edit carries the group (the active sheet first, the others in
   * tab order): the engine repeats the active sheet's cell, format and
   * structure ops on every selected sheet, all or nothing. See
   * {@link groupFor} for which transactions are group edits; `opts.group:
   * false` sends one without the group (an edit meant for the active sheet
   * only, or explicit per-sheet ops).
   */
  async apply(label: string, ops: Op[], after?: Selection, keepClip = false, opts: { group?: boolean } = {}): Promise<ApplyResult | null> {
    try {
      const doc = this.requireDoc();
      // Backstop: the shown sheet is not the edited cell's and the selection is that cell's copy.
      if (this.editingElsewhere) {
        useUiStore.getState().pushToast("info", t("msg.reason.finishEditing", "Finish editing the cell first"));
        return null;
      }
      // Any other edit ends a pending copy/cut (as in Calc and Excel).
      if (!keepClip && useDataStore.getState().clip) useDataStore.getState().setClip(null);
      const sel = this.selection;
      const tx: Transaction = {
        ops,
        label,
        selectionBefore: toSelectionDto(sel, this.sheet),
        selectionAfter: toSelectionDto(after ?? sel, this.sheet),
      };
      const group = opts.group === false ? null : this.groupFor(ops);
      if (group) tx.group = group;
      const res = await this.client.apply(doc, tx);
      this.handleResult(res, ops, after !== undefined);
      return res;
    } catch (e) {
      this.notifyError(e);
      return null;
    }
  }

  /**
   * The transaction group for `ops`, or `null`: only with several sheets
   * selected, only when an op is a group op (`GROUP_OPS`, the engine's
   * whitelist) on the active sheet, and never with sheet or document
   * protection ops in the transaction.
   */
  groupFor(ops: readonly Op[]): number[] | null {
    const group = txGroup(useDocStore.getState().groupSheets, this.sheet);
    if (!group) return null;
    if (ops.some((op) => NO_GROUP_OPS.has(op.type))) return null;
    const active = this.sheet;
    return ops.some((op) => GROUP_OPS.includes(op.type) && (op as { sheet?: unknown }).sheet === active) ? group : null;
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

  setFontSize = (pt: number): Promise<ApplyResult | null> => this.applyPatch(t("toolbar.fontSize", "Font size"), fontSizePatch(pt));
  /** The font box: one undoable ApplyStyle; the document font clears the cells' own family. */
  setFontFamily = (name: string, documentFont: string): Promise<ApplyResult | null> => this.applyPatch(t("toolbar.font", "Font"), fontFamilyPatch(name, documentFont));
  setTextColor = (rgba: number): Promise<ApplyResult | null> => this.applyPatch(t("toolbar.fontColor", "Font colour"), { ...EMPTY_PATCH, color: rgba });
  setFillColor = (rgba: number): Promise<ApplyResult | null> => this.applyPatch(t("toolbar.fillColor", "Fill colour"), { ...EMPTY_PATCH, fill: rgba });
  setAlign = (h: HAlign): Promise<ApplyResult | null> => this.applyPatch(t("msg.undo.align", "Align"), { ...EMPTY_PATCH, hAlign: h });

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
        useUiStore.getState().pushToast("info", t("msg.toast.colorByValueNeedsText", "Select a column of text values to colour."));
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
      await this.apply(t("chrome.item.addUp", "Add up"), [{ type: "setCellInput", sheet, r: active.r, c: active.c, input: sumFormula(range) }]);
    } catch (e) {
      this.notifyError(e);
    }
  }

  insertFormula(formula: string): Promise<ApplyResult | null> {
    const a = this.selection.active;
    return this.apply(t("msg.undo.insertFormula", "Insert formula"), [{ type: "setCellInput", sheet: this.sheet, r: a.r, c: a.c, input: formula }]);
  }

  // -- cells --------------------------------------------------------------------

  onSelectionChange = (sel: Selection): void => {
    useSelectionStore.getState().setSelection(sel);
  };

  /** Starts the shared EditSession (grid or formula bar). */
  startEdit = (cell: CellPos, seed?: string, origin: "grid" | "bar" = "grid"): void => {
    // A formula picking references on another sheet is not replaced (the shown sheet's cell is not editable).
    if (this.editingElsewhere) return;
    // A read-only document: AOO's "Document opened in read-only mode." (typing does nothing).
    if (this.readOnly.refuseEdit(origin === "bar" && seed === undefined)) return;
    // Editing ends detective Fill Mode (AOO switches to the edit shell).
    this.w5.detective.endFillMode();
    const store = useEditStore.getState();
    const active = store.active;
    const known = active && active.sheet === this.sheet && active.cell.r === cell.r && active.cell.c === cell.c ? active.input : null;
    this.editOriginal = known;
    const session = ++this.editSession;
    this.pickHome = null;
    if (seed !== undefined) {
      store.start(cell, seed, origin, this.sheet);
      return;
    }
    store.start(cell, known ?? "", origin, this.sheet);
    if (known !== null) return;
    const info = this.info;
    if (!info) return;
    this.client.cellEditInfo(info.doc, this.sheet, cell.r, cell.c).then(
      (res) => {
        const cur = useEditStore.getState().edit;
        if (session !== this.editSession || !cur || cur.cell.r !== cell.r || cur.cell.c !== cell.c) return;
        this.editOriginal = this.w56.displayText(res.input);
        if (cur.text === "") useEditStore.getState().change(this.editOriginal);
        // The original decides whether the session counts as edited.
        this.syncDocumentEdited();
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

  /**
   * Commits the edit. The grid moves its own selection; the formula bar asks
   * us to. Resolves `false` when the engine refused the input (a validation
   * Stop, a locked cell) or the apply failed.
   */
  commitEdit = async (move: CommitMove): Promise<boolean> => {
    const { edit, origin } = useEditStore.getState();
    if (!edit) return true;
    // Counted before end() so the shell's "edited" flag does not blink off.
    this.committing++;
    try {
      // References picked on another sheet: back to the edited cell's sheet
      // first (AOO ScInputHandler::EnterHandler), so the input, its undo
      // selection and a sheet group all belong to that sheet.
      const homeSel = this.pickHome;
      const editSheet = useEditStore.getState().sheet;
      if (editSheet !== null && !this.sheetExists(editSheet)) {
        // The edited cell's sheet is gone (deleted meanwhile): nothing to commit into.
        this.pickHome = null;
        useEditStore.getState().end();
        return false;
      }
      const away = this.returnToEditSheet();
      useEditStore.getState().end();
      const sheet = this.sheet;
      const unchanged = this.editOriginal !== null && edit.text === this.editOriginal;
      // Editing `{=…}` inside an array block re-enters the whole block.
      const active = useEditStore.getState().active;
      const block = !away && active && active.sheet === sheet && active.cell.r === edit.cell.r && active.cell.c === edit.cell.c ? active.arrayRange : null;
      const arrayOp = block && edit.text.trim().startsWith("{=") ? arrayFormulaOp(sheet, block, edit.text) : null;
      // apply() builds the transaction synchronously, so selectionBefore is the pre-move selection.
      const pending = unchanged
        ? null
        : arrayOp
          ? this.apply(t("msg.undo.arrayFormula", "Array formula"), [arrayOp])
          : this.apply(t("msg.undo.typing", "Typing"), this.typingOps(sheet, edit.cell, edit.text));
      if (pending) this.lastCommit = pending;
      // The formula bar (and a commit from another sheet, where the grid
      // leaves its selection alone) moves the cursor here.
      if (away) {
        if (move !== "none") this.moveAfterCommitHome(homeSel?.selection ?? cellSelection(edit.cell), move, homeSel?.protectedNav ?? false);
        else this.revealSoon(edit.cell);
        this.focusGrid();
      } else if (origin === "bar") {
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
      const res = await pending;
      // Only a refusal of the input itself (op 0) fails the commit: the
      // trailing autoFitRows may be refused (protected sheet) while the input lands.
      return unchanged || (res !== null && !res.errors.some((e) => e.opIndex === 0));
    } finally {
      this.committing--;
      this.syncDocumentEdited();
    }
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
    const { origin, edit } = useEditStore.getState();
    // Escape on another sheet returns to the edited cell (AOO ScInputHandler::CancelHandler).
    const away = this.returnToEditSheet();
    useEditStore.getState().end();
    if (away && edit) this.revealSoon(edit.cell);
    if (origin === "bar" || away) this.focusGrid();
  };

  // -- references across sheets (AOO reference input with SetTabNo) ---------------

  /** The edited cell's selection and the sheet group, saved when the view left its sheet. */
  private pickHome: { selection: Selection; group: number[]; protectedNav: boolean } | null = null;

  private sheetExists(id: number): boolean {
    return this.info?.sheets.some((s) => s.id === id) ?? false;
  }

  /**
   * Enter / Tab committed from another sheet: the edited cell's selection
   * steps as the grid would (within a multi-cell selection; on a protected
   * sheet to the next unlocked cell, asked of the engine).
   */
  private moveAfterCommitHome(sel: Selection, move: Exclude<CommitMove, "none">, protectedNav: boolean): void {
    const lim = { rows: LAST_ROW + 1, cols: LAST_COL + 1, pageRows: 1, pageCols: 1, used: null };
    const set = (next: Selection) => {
      useSelectionStore.getState().setSelection(next);
      this.revealSoon(next.active);
    };
    const r = sel.range;
    const single = r.r0 === r.r1 && r.c0 === r.c1;
    if (protectedNav) {
      void this.nextUnlocked(sel.active, move, single ? null : { ...r }).then(
        (cell) => {
          if (cell) set(single ? cellSelection(cell) : { range: { ...r }, active: cell, anchor: { ...cell } });
          else this.revealSoon(sel.active);
        },
        () => this.revealSoon(sel.active),
      );
      return;
    }
    set(stepWithin(sel, move, lim));
  }

  /** The sheet of the open edit (the displayed sheet when idle). */
  get editSheet(): number {
    return useEditStore.getState().sheet ?? this.sheet;
  }

  /** The open edit's text, or `null` when no edit is open. */
  get editText(): string | null {
    return useEditStore.getState().edit?.text ?? null;
  }

  /** Whether a formula edit shows another sheet than its own (references picked there). */
  get editingElsewhere(): boolean {
    const st = useEditStore.getState();
    return st.edit !== null && st.sheet !== null && st.sheet !== this.sheet;
  }

  /**
   * Whether a sheet switch keeps the open edit (AOO `ScModule::IsFormulaMode`:
   * text starting with `=`, `+` or `-`), showing the other sheet for picking
   * references; other edits are committed first.
   */
  keepsEditAcrossSheets(): boolean {
    const edit = useEditStore.getState().edit;
    return edit !== null && isFormulaText(edit.text);
  }

  private sheetNameOf(id: number): string | null {
    return this.info?.sheets.find((s) => s.id === id)?.name ?? null;
  }

  /**
   * Shows `sheet` while a formula edit stays open (AOO ScTabControl::Select
   * in reference mode: no InputEnterHandler, the tab selection untouched).
   * `extend` (Shift+click on a tab, ⇧⌃PgDn) turns the last picked reference
   * into a 3D one ending on `sheet`. Hidden sheets are ignored.
   */
  pickOnSheet(sheet: number, extend = false): void {
    const st = useEditStore.getState();
    const info = this.info;
    const target = info?.sheets.find((s) => s.id === sheet);
    if (!st.edit || !info || !target || target.hidden) return;
    const home = st.sheet ?? this.sheet;
    if (extend) this.extendPick3d(sheet);
    if (sheet === this.sheet) return;
    if (this.sheet === home && !this.pickHome) {
      this.pickHome = {
        selection: this.selection,
        group: [...useDocStore.getState().groupSheets],
        protectedNav: useV3Store.getState().protection?.sheetProtected === true,
      };
    }
    const saved = this.pickHome;
    if (sheet === home) {
      // Back on the edited cell's sheet: its selection and group again, the editor over the cell.
      this.pickHome = null;
      this.viewSheet(sheet, saved?.selection ?? cellSelection(st.edit.cell), saved?.group ?? null);
    } else {
      // No cell cursor of its own: the selection only mirrors the edited cell's (hidden by the grid).
      this.viewSheet(sheet, saved?.selection ?? cellSelection(st.edit.cell), null);
    }
    const homeName = this.sheetNameOf(home) ?? "";
    const shown = target.name;
    const cell = cellAddress(st.edit.cell.r, st.edit.cell.c);
    this.grid?.ariaMirror.announce(
      sheet === home
        ? t("formulaUi.pickHome", "Editing {cell} on {sheet}", { cell, sheet: homeName })
        : t("formulaUi.pickOther", "Editing {cell} on {sheet}, showing {other}", { cell, sheet: homeName, other: shown }),
    );
    if (useEditStore.getState().origin !== "bar") this.focusGrid();
  }

  /**
   * Shift+click on a tab: the reference just picked (caret still at its end)
   * becomes a 3D reference from its sheet to `sheet` (`Sheet1.A1:Sheet3.A1`,
   * the first sheet in tab order first).
   */
  private extendPick3d(sheet: number): void {
    const st = useEditStore.getState();
    const edit = st.edit;
    const ref = st.pointRef;
    if (!edit || !ref || !st.pointSpan || edit.caret !== st.pointSpan.end) return;
    this.pick3d(ref.sheet, sheet, ref.range);
  }

  /**
   * Replaces the last pick with a reference over `range` on the sheets from
   * `from` to `to` (tab order; one sheet gives a plain reference, named
   * unless it is the edited cell's).
   */
  private pick3d(from: number, to: number, range: RangeDto): void {
    const st = useEditStore.getState();
    const edit = st.edit;
    const info = this.info;
    if (!edit || !info) return;
    const order = info.sheets.map((s) => s.id);
    const [a, b] = order.indexOf(from) <= order.indexOf(to) ? [from, to] : [to, from];
    const home = st.sheet ?? this.sheet;
    const nameA = a === b && a === home ? null : this.sheetNameOf(a);
    const nameB = a === b ? null : this.sheetNameOf(b);
    const res = applyPoint(edit.text, edit.caret, st.pointSpan, range, nameA, nameB);
    if (!res) return;
    st.replace(res.text, res.caret, res.span, a === b ? { sheet: a, range: { ...range } } : { sheet: a, sheet2: b, range: { ...range } });
  }

  /** A Shift+click on another sheet extending an earlier pick to 3D: that pick's sheet and range, until the drag ends. */
  private pickAnchor3d: { sheet: number; range: RangeDto } | null = null;

  /**
   * Shows `sheet` without touching the open edit or the engine's active
   * sheet (the view only): the grid source, the selection and the
   * sheet-level state. `group` restores a sheet group (`null` keeps the
   * current one).
   */
  private viewSheet(sheet: number, selection: Selection, group: number[] | null): void {
    const doc = useDocStore.getState();
    // Raw: setActiveSheet would drop a group without the shown sheet; the group belongs to the edited cell.
    useDocStore.setState({ activeSheet: sheet });
    // A restored group is normalised (sheets deleted or hidden meanwhile drop out).
    if (group) useDocStore.getState().setGroup(group);
    doc.source?.setSheet(sheet);
    useSelectionStore.getState().setSelection(selection);
    this.refreshSheetState();
  }

  /**
   * Ends the view of another sheet: shows the edited cell's sheet again with
   * its selection and group and tells the engine. Returns whether it did
   * (the edit was elsewhere).
   */
  private returnToEditSheet(): boolean {
    const st = useEditStore.getState();
    const saved = this.pickHome;
    this.pickHome = null;
    if (!st.edit || st.sheet === null || (st.sheet === this.sheet && !saved)) return false;
    const home = st.sheet;
    if (!this.sheetExists(home)) return false;
    this.viewSheet(home, saved?.selection ?? cellSelection(st.edit.cell), saved?.group ?? null);
    this.syncActiveSheet(home);
    return true;
  }

  /** Scrolls a cell into view once the grid shows the sheet just switched to. */
  private revealSoon(cell: CellPos): void {
    const raf = typeof requestAnimationFrame === "function" ? requestAnimationFrame : (f: () => void) => setTimeout(f, 0);
    raf(() => raf(() => this.grid?.scrollCellIntoView(cell)));
  }

  deleteContents = (range: RangeDto): void => {
    // The grid reports the current range; clear every range of a multi-selection.
    const ranges = this.ranges;
    const all = ranges.some((r) => r.r0 === range.r0 && r.c0 === range.c0 && r.r1 === range.r1 && r.c1 === range.c1) ? ranges : [{ ...range }];
    void this.apply(t("dialog.delete", "Delete"), [{ type: "clear", sheet: this.sheet, ranges: all, what: "contents" }]);
  };

  clearSelection = (): void => this.deleteContents(this.selection.range);

  clearFormats = (): Promise<ApplyResult | null> =>
    this.apply(t("cmd.edit.clearFormats", "Clear formats"), [{ type: "clear", sheet: this.sheet, ranges: this.ranges, what: "formats" }]);

  // -- structure ------------------------------------------------------------------

  /** Inserts `count` rows above (`before`) or below the selected rows. */
  insertRows = (where: "before" | "after", count?: number): Promise<ApplyResult | null> => {
    const r = this.selection.range;
    const n = count ?? r.r1 - r.r0 + 1;
    const at = where === "before" ? r.r0 : r.r1 + 1;
    const after = { range: { ...r, r0: at, r1: at + n - 1 }, active: { r: at, c: this.selection.active.c }, anchor: { r: at, c: r.c0 } };
    return this.apply(n === 1 ? t("msg.undo.insertRow", "Insert row") : t("dlg.insertRows", "Insert rows"), [{ type: "insertRows", sheet: this.sheet, at, count: n }], after);
  };

  insertCols = (where: "before" | "after", count?: number): Promise<ApplyResult | null> => {
    const r = this.selection.range;
    const n = count ?? r.c1 - r.c0 + 1;
    const at = where === "before" ? r.c0 : r.c1 + 1;
    const after = { range: { ...r, c0: at, c1: at + n - 1 }, active: { r: this.selection.active.r, c: at }, anchor: { r: r.r0, c: at } };
    return this.apply(n === 1 ? t("msg.undo.insertColumn", "Insert column") : t("dlg.insertColumns", "Insert columns"), [{ type: "insertCols", sheet: this.sheet, at, count: n }], after);
  };

  deleteRows = (): Promise<ApplyResult | null> => {
    const r = this.selection.range;
    return this.apply(t("cmd.sheet.deleteRows", "Delete rows"), [{ type: "deleteRows", sheet: this.sheet, at: r.r0, count: r.r1 - r.r0 + 1 }], cellSelection({ r: r.r0, c: this.selection.active.c }));
  };

  deleteCols = (): Promise<ApplyResult | null> => {
    const r = this.selection.range;
    return this.apply(t("cmd.sheet.deleteCols", "Delete columns"), [{ type: "deleteCols", sheet: this.sheet, at: r.c0, count: r.c1 - r.c0 + 1 }], cellSelection({ r: this.selection.active.r, c: r.c0 }));
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
      // Each selected sheet is measured on its own contents (explicit per-sheet ops, no group).
      for (const sheet of this.selectedSheets) {
        for (let c = start; c <= end; c++) {
          // The first rows of the column plus the rows from the viewport down (up to AUTOFIT_ROWS each).
          const ranges = [{ r0: 0, c0: c, r1: Math.min(LAST_ROW, AUTOFIT_ROWS - 1), c1: c }];
          if (top >= AUTOFIT_ROWS) ranges.push({ r0: top, c0: c, r1: Math.min(LAST_ROW, top + AUTOFIT_ROWS - 1), c1: c });
          const blocks = await this.client.cells(doc, sheet, ranges);
          const w = grid.measureColumnWidth(c, blocks.flatMap((b) => b.cells));
          if (w === null) continue;
          ops.push({ type: "setColWidth", sheet, start: c, end: c, px: Math.max(AUTOFIT_MIN, Math.min(AUTOFIT_MAX, w)) });
        }
      }
      if (ops.length > 0) await this.apply(t("msg.undo.fitColumnWidth", "Fit column width"), ops, undefined, false, { group: false });
    } catch (e) {
      this.notifyError(e);
    }
  };

  // -- fill handle ------------------------------------------------------------------

  /** Fill-handle drag: see fill.ts (`fillRange` is the engine-swap point). */
  onFill = async (source: RangeDto, target: RangeDto, mode: FillMode = "auto"): Promise<void> => {
    try {
      this.requireDoc();
      const ops = fillRange({ sheet: this.sheet }, source, target, mode);
      if (ops.length === 0) return;
      const sel: Selection = { range: { ...target }, active: { ...this.selection.active }, anchor: { r: target.r0, c: target.c0 } };
      await this.apply(t("menu.edit.fill", "Fill"), ops, sel);
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
  onPointRange = (range: RangeDto, phase: PointPhase, extend = false): void => {
    const st = useEditStore.getState();
    const edit = st.edit;
    if (!edit) return;
    // Shift+click on another sheet after a pick elsewhere (AOO ScTabView::UpdateRef
    // with a new tab): a 3D reference from the earlier pick to this cell.
    if (phase === "start") {
      const ref = st.pointRef;
      const atPick = st.pointSpan !== null && edit.caret === st.pointSpan.end;
      this.pickAnchor3d = extend && ref && atPick && ref.sheet !== this.sheet ? { sheet: ref.sheet, range: { ...ref.range } } : null;
    }
    const anchor = this.pickAnchor3d;
    if (anchor) {
      if (phase === "end") this.pickAnchor3d = null;
      const a = anchor.range;
      this.pick3d(anchor.sheet, this.sheet, {
        r0: Math.min(a.r0, range.r0),
        c0: Math.min(a.c0, range.c0),
        r1: Math.max(a.r1, range.r1),
        c1: Math.max(a.c1, range.c1),
      });
      return;
    }
    // On another sheet the reference names it (`Sheet2.A1`), on the edited cell's own sheet not.
    const shown = this.sheet;
    const other = st.sheet !== null && st.sheet !== shown ? this.sheetNameOf(shown) : null;
    const res = applyPoint(edit.text, edit.caret, st.pointSpan, range, other);
    if (!res) return;
    st.replace(res.text, res.caret, res.span, { sheet: shown, range: { ...range } });
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
    if (mod && !e.altKey && (e.key === "PageDown" || e.key === "PageUp") && this.keepsEditAcrossSheets()) {
      // ⌃PgDn / ⌃PgUp show the next sheet for picking; with Shift the pick extends to it (3D).
      const info = this.info;
      const next = info ? stepVisible(info.sheets, this.sheet, e.key === "PageDown" ? 1 : -1, !e.shiftKey) : null;
      if (next !== null) this.pickOnSheet(next, e.shiftKey);
      return true;
    }
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
    void this.apply(axis === "col" ? t("dlg.columnWidth", "Column width") : t("dlg.rowHeight", "Row height"), [op]);
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
        if (ok) useUiStore.getState().pushToast("info", t("msg.toast.named", "Named {range} “{name}”", { range: a1Text(this.selection.range), name }));
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
        useUiStore.getState().pushToast("info", t("msg.toast.copyMultiRange", "Copying several ranges at once is not supported; copied the last one."));
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
    // A cut-paste onto the same sheet moves the range (AOO asks there too); the move writes the block at the top-left.
    const moving = !valuesOnly && token !== null && clip?.cut === true && clip.token === token && clip.sheet === this.sheet;
    const payload: PastePayload = token ? { type: "internal", token } : { type: "tsv", text };
    const check = moving ? this.confirmPasteOverData({ r0: target.r0, c0: target.c0, r1: target.r0, c1: target.c0 }, payload, pasteOptions(DEFAULT_PASTE_SPECIAL)) : this.confirmPasteOverData(target, payload, pasteOptions(DEFAULT_PASTE_SPECIAL));
    return check.then((ok) => (ok ? this.pasteNow(text, token, valuesOnly, target) : null));
  }

  /**
   * AOO's "You are pasting data into cells that already contain data."
   * confirmation (Options ▸ Input, on by default). Asks the engine whether the
   * paste would write over data (one query, the engine's own target area);
   * resolves `true` to go on.
   */
  async confirmPasteOverData(target: RangeDto, payload: PastePayload, options: PasteOptions): Promise<boolean> {
    if (!useW56Store.getState().prefs.pasteOverwriteWarning) return true;
    const info = this.info;
    if (!info) return true;
    // Every sheet the paste writes to (AOO checks each selected sheet, viewfun3.cxx:923).
    const sheets = this.groupFor([{ type: "pasteSpecial", sheet: this.sheet, target, payload, options }]) ?? [this.sheet];
    try {
      let any = false;
      for (const sheet of sheets) {
        if (await this.client.pasteOverwrites(info.doc, sheet, target, payload, options)) {
          any = true;
          break;
        }
      }
      if (!any) return true;
    } catch {
      return true;
    }
    return this.askPasteOverData();
  }

  /** Tab / Enter on a protected sheet: the engine's next unlocked cell (grid `onNextUnlocked`). */
  nextUnlocked = async (from: CellPos, dir: Direction, within: RangeDto | null): Promise<CellPos | null> => {
    const info = this.info;
    if (!info) return null;
    return this.client.nextUnlocked(info.doc, this.sheet, from, dir, within);
  };

  private askPasteOverData(): Promise<boolean> {
    return new Promise<boolean>((resolve) => {
      useUiStore.getState().openDialog({
        kind: "confirm",
        title: t("cmd.edit.paste", "Paste"),
        message: t("dialog.pasteOverData", "You are pasting data into cells that already contain data."),
        ok: t("dialog.yes", "Yes"),
        cancel: t("dialog.no", "No"),
        focusOk: true,
        onConfirm: () => resolve(true),
        onCancel: () => resolve(false),
      });
    });
  }

  private pasteNow(text: string, token: string | null, valuesOnly: boolean, target: RangeDto): Promise<ApplyResult | null> {
    const clip = useDataStore.getState().clip;
    if (!valuesOnly && token && clip?.cut && clip.token === token) {
      useDataStore.getState().setClip(null);
      if (clip.sheet === this.sheet) {
        const to = { r: target.r0, c: target.c0 };
        const h = clip.range.r1 - clip.range.r0;
        const w = clip.range.c1 - clip.range.c0;
        const after = { range: { r0: to.r, c0: to.c, r1: to.r + h, c1: to.c + w }, active: { ...to }, anchor: { ...to } };
        return this.apply(t("shell.group.move", "Move"), [{ type: "moveRange", sheet: this.sheet, from: { ...clip.range }, to }], after);
      }
      // Across sheets: paste, then clear the source. A move is never a group
      // edit (AOO moves on the active sheet only): with the source sheet in
      // the group the clear would wipe what the paste put there.
      return this.apply(
        t("shell.group.move", "Move"),
        [
          { type: "paste", sheet: this.sheet, target, payload: { type: "internal", token } },
          { type: "clear", sheet: clip.sheet, ranges: [{ ...clip.range }], what: "all" },
        ],
        undefined,
        false,
        { group: false },
      );
    }
    if (valuesOnly && token) {
      return this.apply(t("msg.undo.pasteValues", "Paste values"), [{ type: "pasteSpecial", sheet: this.sheet, target, payload: { type: "internal", token }, options: pasteOptions({ what: "values", transpose: false, skipEmpty: false, operation: "none" }) }], undefined, true);
    }
    const payload = token && !valuesOnly ? { type: "internal" as const, token } : { type: "tsv" as const, text };
    return this.apply(valuesOnly ? t("msg.undo.pasteValues", "Paste values") : t("cmd.edit.paste", "Paste"), [{ type: "paste", sheet: this.sheet, target, payload }], undefined, true);
  }

  /** Reads the clipboard (system if allowed, else the last copy in this window). */
  async readClipboard(): Promise<{ text: string | null; token: string | null }> {
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
      useUiStore.getState().pushToast("info", t("msg.toast.pasteWithKey", "Use ⌘V to paste (the clipboard is not readable here)."));
      return;
    }
    await this.pasteData(text, token, valuesOnly);
    this.focusGrid();
  };

  /** Paste Special… (content kinds, operations, skip empty, transpose, link, shift cells). */
  pasteSpecial = async (choice: PasteSpecialChoice, source?: { text: string | null; token: string | null }): Promise<void> => {
    const { text, token } = source ?? (await this.readClipboard());
    if (text === null && !token) {
      useUiStore.getState().pushToast("info", t("msg.toast.copyFirst", "Copy something first."));
      return;
    }
    if (choice.link && !token) {
      useUiStore.getState().pushToast("info", t("paste.linkNeedsCopy", "Link needs cells copied in this document."));
      return;
    }
    const payload = token ? { type: "internal" as const, token } : { type: "tsv" as const, text: text ?? "" };
    const target = { ...this.selection.range };
    // AOO asks when cell contents are pasted without an operation (not for formats or notes only, nor when cells shift).
    if (pasteAsksOverwrite(choice) && !(await this.confirmPasteOverData(target, payload, pasteOptions(choice)))) {
      this.focusGrid();
      return;
    }
    await this.apply(t("dlg.pasteSpecial", "Paste special"), [{ type: "pasteSpecial", sheet: this.sheet, target, payload, options: pasteOptions(choice) }], undefined, true);
    this.focusGrid();
  };

  // -- sheets -------------------------------------------------------------------

  /**
   * Switches the visible sheet and tells the engine (`set_active_sheet`,
   * view state: no undo step, no modified flag), so a save, CSV export and
   * Find across sheets use the sheet the user is on.
   *
   * A switch to a sheet outside the multi-sheet selection collapses it to
   * that sheet; one inside it (or with `keepGroup`) keeps it.
   */
  showSheet(sheet: number, focus = true, keepGroup = false): void {
    if (useEditStore.getState().edit) {
      // An edit showing another sheet goes back to its own first (its group and selection).
      this.returnToEditSheet();
      useEditStore.getState().end();
    }
    this.pickHome = null;
    const doc = useDocStore.getState();
    if (!keepGroup && !doc.groupSheets.includes(sheet)) doc.selectOnly(sheet);
    if (doc.activeSheet !== sheet) {
      const group = useDocStore.getState().groupSheets;
      doc.setActiveSheet(sheet);
      // setActiveSheet drops a group that doesn't hold the sheet; `keepGroup` keeps it.
      if (keepGroup && group.length > 1) useDocStore.getState().setGroup([...group, sheet]);
    }
    this.syncActiveSheet(sheet);
    doc.source?.setSheet(sheet);
    useSelectionStore.getState().reset();
    this.refreshSheetState();
    if (focus) this.focusGrid();
  }

  /** Re-reads the shown sheet's state: active cell info, filter, page setup, objects, outline. */
  private refreshSheetState(): void {
    useDataStore.getState().setFilter(null);
    useV3Store.getState().reset();
    void this.refreshActive();
    void this.refreshFilter();
    void this.v3.refreshSheet();
    this.w5.refreshSheet();
    this.w56.onSheet();
  }

  private pendingActive: { doc: DocId; sheet: number } | null = null;
  private activeSync: Promise<void> = Promise.resolve();

  /**
   * Sends the user's sheet to the engine: one call at a time, in order, and
   * only the latest of a burst (Ctrl+PgDn held down). Failures are ignored
   * (an older engine, a closed document): the print options name the sheet
   * themselves.
   */
  private syncActiveSheet(sheet: number): void {
    const doc = this.info?.doc;
    if (doc === undefined) return;
    const queued = this.pendingActive !== null;
    this.pendingActive = { doc, sheet };
    if (queued) return;
    this.activeSync = this.activeSync.then(async () => {
      const p = this.pendingActive;
      this.pendingActive = null;
      if (p) await this.client.setActiveSheet(p.doc, p.sheet).catch(() => undefined);
    });
  }

  /** Resolves once the engine knows the user's sheet (awaited before a save and a Find). */
  activeSheetSynced(): Promise<void> {
    return this.activeSync;
  }

  /** ⌃PgDn / ⌃PgUp: the next visible sheet, alone (a multi-sheet selection collapses to it). */
  stepSheet(delta: 1 | -1): void {
    const info = this.info;
    if (!info) return;
    const next = stepVisible(info.sheets, this.sheet, delta, true);
    if (this.keepsEditAcrossSheets()) {
      if (next !== null) this.pickOnSheet(next);
      return;
    }
    if (useEditStore.getState().edit) {
      void this.commitEdit("none").then((ok) => {
        if (ok) this.stepSheet(delta);
      });
      return;
    }
    useDocStore.getState().selectOnly(next ?? this.sheet);
    if (next !== null) this.showSheet(next);
  }

  /** ⇧⌃PgDn / ⇧⌃PgUp: the next visible sheet joins the selection and becomes active. */
  selectStep(delta: 1 | -1, focus = true): void {
    const info = this.info;
    if (!info) return;
    const step = extendStep(info.sheets, useDocStore.getState().groupSheets, this.sheet, delta);
    if (!step) return;
    this.showSheet(step.active, focus, true);
    useDocStore.getState().setGroup(step.group);
  }

  /** The selected sheets in tab order (the active one alone without a group). */
  get selectedSheets(): number[] {
    return selectedIds(useDocStore.getState().groupSheets, this.sheet);
  }

  /** Whether several sheets are selected. */
  get grouped(): boolean {
    return useDocStore.getState().groupSheets.length > 1;
  }

  /**
   * A click on a sheet tab (AOO ScTabControl): plain selects only that
   * sheet (inside the group it becomes active and the group stays), ⌘/Ctrl
   * toggles it in or out of the group (not the active one), Shift selects
   * the sheets from the active one to it.
   */
  clickTab(sheet: number, mods: { toggle?: boolean; range?: boolean } = {}): void {
    // While a formula is edited the tab only shows the sheet for picking
    // references (Shift extends the last pick to a 3D reference; ⌘ is
    // ignored, the tab selection stays); any other edit is committed first
    // (AOO ScTabControl::Select: InputEnterHandler unless in reference mode).
    if (this.keepsEditAcrossSheets()) {
      this.pickOnSheet(sheet, mods.range === true);
      return;
    }
    if (useEditStore.getState().edit) {
      // Switch only once the engine took the input (a validation Stop keeps the user here).
      void this.commitEdit("none").then((ok) => {
        if (ok) this.clickTab(sheet, mods);
      });
      return;
    }
    const doc = useDocStore.getState();
    if (mods.range) doc.extendTo(sheet);
    else if (mods.toggle) doc.toggleSheet(sheet);
    else this.showSheet(sheet, true, doc.groupSheets.includes(sheet));
  }

  /** Sheet ▸ Select All Sheets. */
  selectAllSheets = (): void => useDocStore.getState().selectAllSheets();

  /** Deselect All Sheets: back to the active sheet. */
  deselectAllSheets = (): void => useDocStore.getState().deselectSheets();

  /** Deletes the selected sheets (one undo step), then shows the next one left. */
  deleteSelectedSheets = async (): Promise<ApplyResult | null> => {
    const info = this.info;
    const ids = this.selectedSheets;
    // At least one visible sheet stays.
    if (!info || ids.length >= info.sheets.filter((s) => !s.hidden).length) return null;
    if (ids.length === 1) return this.deleteSheet(ids[0]!);
    const next = nextActiveAfterDelete(info.sheets, ids, this.sheet);
    const label = t("sheetTabs.deleteN", "Delete {n} sheets", { n: ids.length });
    const res = await this.apply(label, ids.map((sheet): Op => ({ type: "deleteSheet", sheet })));
    if (res && next !== null && useDocStore.getState().info?.sheets.some((s) => s.id === next)) this.showSheet(next);
    return res;
  };

  /**
   * Drag-and-drop of a tab: the sheet (with a group, every selected sheet,
   * as one block in tab order) moves before the visible tab `dropAt`.
   */
  dropSheets = async (sheet: number, dropAt: number): Promise<void> => {
    const info = this.info;
    if (!info) return;
    const visible = info.sheets.filter((s) => !s.hidden);
    const at = dropAt >= visible.length ? info.sheets.length : info.sheets.findIndex((s) => s.id === visible[dropAt]?.id);
    const ids = this.grouped && useDocStore.getState().groupSheets.includes(sheet) ? this.selectedSheets : [sheet];
    const steps = moveBlockSteps(info.sheets.map((s) => s.id), ids, at < 0 ? info.sheets.length : at);
    if (steps.length === 0) return;
    const label = ids.length > 1 ? t("msg.undo.moveSheets", "Move sheets") : t("msg.undo.moveSheet", "Move sheet");
    await this.apply(label, steps.map((m): Op => ({ type: "moveSheet", sheet: m.sheet, to: m.to })));
  };

  addSheet = (): Promise<ApplyResult | null> => this.apply(t("sheetTabs.add", "Add sheet"), [{ type: "addSheet", name: null, index: null }]);

  renameSheet = (sheet: number, name: string): Promise<ApplyResult | null> =>
    this.apply(t("cmd.sheet.rename", "Rename sheet"), [{ type: "renameSheet", sheet, name }]);

  deleteSheet = (sheet: number): Promise<ApplyResult | null> => this.apply(t("cmd.sheet.delete", "Delete sheet"), [{ type: "deleteSheet", sheet }]);

  moveSheet = (sheet: number, to: number): Promise<ApplyResult | null> =>
    this.apply(t("msg.undo.moveSheet", "Move sheet"), [{ type: "moveSheet", sheet, to }]);

  // -- files ----------------------------------------------------------------------

  // -- unsaved-changes guard (unsaved.ts) -------------------------------------------

  private get closeShell(): CloseShell {
    return this.shell as unknown as CloseShell;
  }

  private get windowShell(): WindowShell {
    return this.shell as unknown as WindowShell;
  }

  /** Desktop: one document per window (`place_document`); the browser keeps one document. */
  get multiWindow(): boolean {
    return typeof this.windowShell.placeDocument === "function";
  }

  /** Whether the open edit session holds typed text not yet in the document. */
  get pendingEdit(): boolean {
    return isPendingEdit(useEditStore.getState().edit, this.editOriginal);
  }

  /**
   * Mirrors the edited state to the shell (macOS "edited" dot, close/quit
   * guard) when it or the document changes: dirty, or uncommitted typing in
   * the cell editor / formula bar (or its commit still in flight).
   */
  private syncDocumentEdited(): void {
    const info = useDocStore.getState().info;
    const edited = this.edited;
    // The first change makes the start-up blank workbook a real document (no window reuse).
    if (edited) this.pristineDoc = null;
    const doc = info?.doc ?? null;
    // AOO appends " (read-only)" to the title of a read-only document.
    const title = info ? displayTitle(info) : null;
    if ((edited === this.reportedEdited && doc === this.reportedDoc && title === this.reportedTitle) || !this.closeShell.setDocumentEdited) return;
    this.reportedEdited = edited;
    this.reportedDoc = doc;
    this.reportedTitle = title;
    this.closeShell.setDocumentEdited(edited, doc, title).catch((e: unknown) => console.warn("tally: set_document_edited", e));
  }

  /** Unsaved changes: dirty, or uncommitted typing (or its commit still in flight). */
  private get edited(): boolean {
    return (useDocStore.getState().info?.dirty ?? false) || this.pendingEdit || this.committing > 0;
  }

  /** Whether a document opened from here may replace this window's (windows.ts). */
  canReuseWindow(): boolean {
    return canReuseWindow({ info: this.info, pristineDoc: this.pristineDoc, edited: this.edited });
  }

  /**
   * Before creating a document that replaces this one: the browser's single
   * document asks about unsaved changes; with windows nothing is replaced
   * unless the window is untouched, so there is nothing to ask.
   */
  replaceGuard = async (): Promise<boolean> => this.multiWindow || (await this.confirmUnsaved());

  /**
   * Shows a document just created or opened: in the desktop app in a new
   * window, or in this one when it only holds its untouched blank workbook
   * (`place_document`); in the browser it replaces the current document.
   */
  present = async (info: WorkbookInfo): Promise<void> => {
    const place = this.windowShell.placeDocument;
    if (!place) {
      this.load(info);
      return;
    }
    try {
      if ((await place.call(this.windowShell, info.doc, this.canReuseWindow())) === "here") this.load(info);
      // The new tab or window shows the engine's summary, which no longer has the
      // open warnings (they are delivered once): show them here.
      else toastWarnings(openNotices(info).map((w) => `${info.title}: ${w}`));
    } catch (e) {
      this.notifyError(e);
    }
  };

  /** Tells the other windows about a prefs or theme change made here. */
  broadcastPrefs(theme: "system" | "light" | "dark" | null): void {
    this.windowShell.broadcastPrefs?.(theme).catch((e: unknown) => console.warn("tally: broadcast_prefs", e));
  }

  /** `prefs-changed`: another window changed the prefs or the theme. */
  private prefsChangedElsewhere(theme: "system" | "light" | "dark" | null): void {
    if (theme) {
      this.remoteTheme = true;
      try {
        useUiStore.getState().setThemePref(theme);
      } finally {
        this.remoteTheme = false;
      }
    }
    void this.w56.reloadPrefs();
  }

  /**
   * The guard's first step (as in AOO, closing while editing commits the
   * input): typed text is committed; an unchanged edit session just ends.
   * Resolves `false` when the commit was refused (the Stop alert is up) or
   * an editor is still open, so the replace/close aborts.
   */
  /**
   * Commits typed text (or ends an unchanged edit) and waits for the
   * commit to land: `false` when the commit was refused (Edit Mode).
   */
  settleEdits = async (): Promise<boolean> => {
    if (!(await this.commitPendingEdit())) return false;
    await this.lastCommit;
    return true;
  };

  private async commitPendingEdit(): Promise<boolean> {
    if (!useEditStore.getState().edit) return true;
    if (!this.pendingEdit) {
      this.cancelEdit();
      return true;
    }
    if (!(await this.commitEdit("none"))) return false;
    return !useEditStore.getState().edit;
  }

  /**
   * Run before anything replaces or closes the current document. A clean
   * document resolves `true` at once; a dirty one opens the save prompt:
   * Save (Save As for an untitled document) resolves `true` once saved and
   * `false` if that dialog is cancelled or the save fails; Don't Save
   * resolves `true`; Cancel (and Escape) `false`. A request while the
   * guard is already running resolves `false`. Uncommitted typing is
   * committed first (`commitPendingEdit`); a refused commit resolves `false`.
   */
  confirmUnsaved = async (): Promise<boolean> => {
    if (this.guarding) return false;
    this.guarding = true;
    try {
      if (!(await this.commitPendingEdit())) return false;
      // An Enter just before ⌘W/⌘Q: its apply has to land before `dirty` is read.
      await this.lastCommit;
      const info = this.info;
      if (guardDecision(info) === "proceed" || !info) return true;
      const choice = await useSavePromptStore.getState().ask(info.title);
      return proceedAfter(choice, choice === "save" && (await this.saveForGuard()));
    } finally {
      this.guarding = false;
    }
  };

  /** Save for the guard: the file's own Save, or the Save As dialog when untitled. */
  private async saveForGuard(): Promise<boolean> {
    const info = this.info;
    if (!info) return true;
    try {
      const res = await this.runSave(false, null);
      if (!res) return false;
      useDocStore.getState().setInfo(res);
      return !res.dirty;
    } catch (e) {
      this.notifyError(e);
      return false;
    }
  }

  /**
   * Saves through the shell with AOO's CSV prompts: Save of a CSV file asks
   * "Keep current format"; a CSV file picked in the Save dialog (code
   * `csv_options`) asks it too (not for File ▸ Export as CSV), then the
   * Text Export options, and the shell writes it (`saveCsv`). `null` when
   * one of the prompts was cancelled.
   */
  private async runSave(saveAs: boolean, format: FileFormat | null): Promise<WorkbookInfo | null> {
    const info = this.info;
    if (!info) return null;
    // The file keeps the sheet the user is on; CSV writes that sheet.
    await this.activeSheetSynced();
    const doc = info.doc;
    const multi = info.sheets.length > 1;
    const sheet = this.sheetName();
    const prompts = useCsvPromptStore.getState();
    // The text family (.csv, .tsv, .txt, .tab) saves as CSV: AOO asks first.
    if (!saveAs && format === null && info.path !== null && /\.(csv|tsv|txt|tab)$/i.test(info.path)) {
      const keep = await prompts.askKeep(info.title, multi, sheet);
      if (keep === "cancel") return null;
      if (keep === "odf") return this.runSave(true, "ods");
    }
    try {
      return await this.shell.saveWithDialog(doc, saveAs, format);
    } catch (e) {
      if (errorOf(e).code !== CSV_OPTIONS) throw e;
      const file = errorOf(e).message;
      if (format !== "csv") {
        const keep = await prompts.askKeep(file, multi, sheet);
        if (keep !== "keep") {
          await this.shell.saveCsv(doc, null);
          return keep === "odf" ? this.runSave(true, "ods") : null;
        }
      }
      const opts = await prompts.askOptions(file, multi, sheet);
      if (!opts) {
        await this.shell.saveCsv(doc, null);
        return null;
      }
      return await this.shell.saveCsv(doc, opts);
    }
  }

  /**
   * The desktop shell held back closing this document, a quit or a
   * multi-tab close (`close-requested`): run the guard, then let the shell
   * close this tab or window (which closes the document in the engine and
   * removes its autosave snapshot) or go on with the sequence
   * (`confirm_close`). Cancel during a sequence tells the shell
   * (`cancel_quit`), which aborts the whole quit or multi-tab close.
   */
  closeRequested = async (kind: CloseKind): Promise<boolean> => {
    if (this.guarding) {
      // A prompt is already up in this window (⌘W, or the red button): it
      // answers for the quit too. Cancel there must also end the quit.
      if (isSequence(kind)) this.quitWaiting = true;
      return false;
    }
    const ok = await this.confirmUnsaved();
    const quitAsked = isSequence(kind) || this.quitWaiting;
    this.quitWaiting = false;
    if (!ok) {
      // Cancel (or a cancelled Save As) during a quit aborts the whole quit.
      if (quitAsked) await this.closeShell.cancelQuit?.().catch((e: unknown) => console.warn("tally: cancel_quit", e));
      return false;
    }
    try {
      // A window close answered during a quit closes the window; the shell
      // then goes on with the quit.
      await this.closeShell.confirmClose?.(isSequence(kind));
    } catch (e) {
      this.notifyError(e);
    }
    // Answered for a quit that may still be cancelled elsewhere: the next
    // change here is reported again, which re-arms the shell's guard.
    if (isSequence(kind)) this.reportedEdited = null;
    return true;
  };

  /** File ▸ New: a new tab or window in the desktop app; the browser replaces the document (guarded). */
  newWorkbook = async (): Promise<void> => {
    const place = this.windowShell.placeDocument;
    if (place) {
      await place.call(this.windowShell, null, false).catch(this.notifyError);
      return;
    }
    if (!(await this.confirmUnsaved())) return;
    try {
      this.load(await this.client.newWorkbook(null), true);
    } catch (e) {
      this.notifyError(e);
    }
  };

  newFromTemplate = async (id: string): Promise<void> => {
    if (!(await this.replaceGuard())) return;
    try {
      await this.present(await this.client.newWorkbook(id));
    } catch (e) {
      this.notifyError(e);
    }
  };

  /**
   * File ▸ Open… (File ▸ Open Read-Only… with `readOnly`): a file already
   * open in a window brings that window to the front (the shell answers
   * `null`); a file someone else has locked asks AOO's "Document in Use"
   * question first.
   */
  openDialog = async (readOnly = false): Promise<void> => {
    if (!(await this.replaceGuard())) return;
    try {
      const info = await this.shell.openDialog(readOnly).catch((e: unknown) => this.readOnly.resolveLockedOpen(e));
      if (info) await this.present(info);
    } catch (e) {
      this.notifyError(e);
    }
  };

  openRecent = async (path: string): Promise<void> => {
    if (!(await this.replaceGuard())) return;
    try {
      const info = await this.client.open(path).catch((e: unknown) => this.readOnly.resolveLockedOpen(e));
      if (info) await this.present(info);
    } catch (e) {
      this.notifyError(e);
    }
  };

  save = async (saveAs: boolean, format: FileFormat | null = null): Promise<void> => {
    try {
      const info = this.info;
      if (!info) return;
      const res = await this.runSave(saveAs, format);
      if (!res) return;
      useDocStore.getState().setInfo(res);
      // Save warnings (CSV without a text delimiter, unmappable characters).
      toastWarnings(res.warnings);
      if (format !== null) useUiStore.getState().pushToast("info", t("msg.toast.exported", "Exported {title}", { title: res.title }));
    } catch (e) {
      this.notifyError(e);
    }
  };

  /**
   * File ▸ Close (⌘W): closes this document's tab (or its window, when it is
   * the only tab) in the desktop app, guarded like the red button; the
   * browser replaces the document with a blank one (guarded in `newWorkbook`).
   */
  closeDocument = async (): Promise<void> => {
    if (this.multiWindow) await this.closeRequested("window");
    else await this.newWorkbook();
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
   * the current region around the active cell (Ctrl+*, the engine's
   * `data_region`, so a large table is never clipped).
   */
  async dataRegion(): Promise<RangeDto | null> {
    return (await this.dataRegionOf())?.range ?? null;
  }

  /** `dataRegion`, telling whether the range is the selection as the user marked it. */
  private async dataRegionOf(): Promise<{ range: RangeDto; selected: boolean } | null> {
    const doc = this.requireDoc();
    const sheet = this.sheet;
    const sel = this.selection;
    const af = useDataStore.getState().filter?.range;
    const a = sel.active;
    if (af && a.r >= af.r0 && a.r <= af.r1 && a.c >= af.c0 && a.c <= af.c1) return { range: { ...af }, selected: false };
    const r = sel.range;
    const merge = this.merges().find((m) => m.r0 === r.r0 && m.c0 === r.c0 && m.r1 === r.r1 && m.c1 === r.c1);
    if (!(r.r0 === r.r1 && r.c0 === r.c1) && !merge) {
      const used = (await this.client.sheetLayout(doc, sheet)).used;
      if (!used) return null;
      const out = { r0: Math.max(r.r0, used.r0), c0: Math.max(r.c0, used.c0), r1: Math.min(r.r1, used.r1), c1: Math.min(r.c1, used.c1) };
      return out.r0 <= out.r1 && out.c0 <= out.c1 ? { range: out, selected: true } : null;
    }
    const one = { r0: a.r, c0: a.c, r1: a.r, c1: a.c };
    const range = await this.client.dataRegion(doc, sheet, one);
    if (range.r0 === range.r1 && range.c0 === range.c1) {
      // The cell and its neighbours are empty: nothing to act on unless the cell itself has content.
      const [block] = await this.client.cells(doc, sheet, [one]);
      const v = block?.cells.find((x) => x.r === a.r && x.c === a.c);
      if (!v || v.kind === "empty" || v.text === "") return null;
    }
    return { range, selected: false };
  }

  /**
   * The range a sort acts on: `dataRegion`, with AOO's "Sort Range" question
   * (`lcl_GetSortParam`) when a marked selection has data next to it: extend
   * the sort to the whole block, sort the selection as it is, or cancel. One
   * cell and the AutoFilter area never ask. `null`: nothing to sort (a toast
   * was shown) or cancelled.
   */
  async sortRegion(): Promise<RangeDto | null> {
    const found = await this.dataRegionOf();
    if (!found) {
      useUiStore.getState().pushToast("info", t("msg.toast.selectTableToSort", "Select a table (or a cell in one) to sort."));
      return null;
    }
    const { range, selected } = found;
    if (!selected) return range;
    const grown = await this.client.dataRegion(this.requireDoc(), this.sheet, range);
    if (grown.r0 === range.r0 && grown.c0 === range.c0 && grown.r1 === range.r1 && grown.c1 === range.c1) return range;
    const choice = await this.askSortRange(grown, range);
    return choice === "extend" ? grown : choice === "current" ? range : null;
  }

  private askSortRange(extend: RangeDto, current: RangeDto): Promise<"extend" | "current" | null> {
    return new Promise((resolve) => {
      useUiStore.getState().openDialog({
        kind: "confirm",
        title: t("dialog.sortRange.title", "Sort Range"),
        message: t(
          "dialog.sortRange.message",
          "The cells next to the current selection also contain data. Do you want to extend the sort range to {extend}, or sort the currently selected range, {current}?",
          { extend: a1Text(extend), current: a1Text(current) },
        ),
        ok: t("dialog.sortRange.extend", "Extend selection"),
        alt: t("dialog.sortRange.current", "Current selection"),
        focusOk: true,
        onConfirm: () => resolve("extend"),
        onAlt: () => resolve("current"),
        onCancel: () => resolve(null),
      });
    });
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
      const region = await this.sortRegion();
      if (!region) return;
      if (region.r0 === region.r1) {
        useUiStore.getState().pushToast("info", t("msg.toast.selectTableToSort", "Select a table (or a cell in one) to sort."));
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
      const region = await this.sortRegion();
      if (!region) return;
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
        await this.apply(t("msg.undo.removeFilter", "Remove filter"), [{ type: "setAutoFilter", sheet, range: null }]);
      } else {
        const region = await this.dataRegion();
        if (!region || region.r0 === region.r1) {
          useUiStore.getState().pushToast("info", t("msg.toast.selectTableToFilter", "Select a table with a header row to filter."));
          return;
        }
        await this.apply(t("menu.data.filter", "Filter"), [{ type: "setAutoFilter", sheet, range: region }]);
      }
      await this.refreshFilter();
    } catch (e) {
      this.notifyError(e);
    }
  };

  setColumnFilter = async (col: number, filter: ColumnFilter | null): Promise<void> => {
    await this.apply(filter ? t("menu.data.filter", "Filter") : t("filter.clear", "Clear filter"), [columnFilterOp(this.sheet, col, filter)]);
    await this.refreshFilter();
  };

  /** Data ▸ Clear Filters: every column's filter (the AutoFilter stays). */
  clearFilters = async (): Promise<void> => {
    const f = useDataStore.getState().filter;
    const ops = (f?.columns ?? []).filter((c) => c.active).map((c) => columnFilterOp(this.sheet, c.col, null));
    if (ops.length === 0) return;
    await this.apply(t("cmd.data.clearFilters", "Clear filters"), ops);
    await this.refreshFilter();
  };

  /** The ▾ of a filter header was clicked. */
  onFilterButton = (col: number, rect: { left: number; bottom: number }): void => {
    useUiStore.getState().openContextMenu(null);
    // The AutoFilter belongs to one sheet: off with several selected (AOO).
    if (this.grouped) {
      useUiStore.getState().pushToast("info", t("msg.reason.sheetsGrouped", "Not available with several sheets selected"));
      return;
    }
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
        useUiStore.getState().pushToast("info", t("msg.toast.columnOutsideTable", "That column is outside the table."));
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
        useUiStore.getState().pushToast("info", t("msg.toast.selectCellsToMerge", "Select two or more cells to merge."));
        return;
      }
      // Every selected sheet is checked for hidden content (AOO ScDocFunc::MergeCells).
      const filled = new Map<number, CellView[]>();
      for (const s of this.selectedSheets) {
        const [block] = await this.client.cells(doc, s, [range]);
        filled.set(s, (block?.cells ?? []).filter((v) => v.kind !== "empty" && v.text !== ""));
      }
      const others = [...filled.values()].some((cells) => cells.some((v) => v.r !== range.r0 || v.c !== range.c0));
      const tl = { r: range.r0, c: range.c0 };
      const go = (): void => {
        void this.apply(t("cmd.format.merge", "Merge cells"), [{ type: "mergeCells", sheet, range }], { range, active: tl, anchor: tl });
      };
      if (!others) {
        go();
        return;
      }
      // §51: AOO asks whether to move the hidden cells' contents into the first cell.
      if (!this.w56.mergePrompt(range, filled, go)) go();
    } catch (e) {
      this.notifyError(e);
    }
  };

  unmergeCells = (): Promise<ApplyResult | null> => this.apply(t("cmd.format.unmerge", "Unmerge cells"), [{ type: "unmergeCells", sheet: this.sheet, range: { ...this.selection.range } }]);

  toggleMerge = async (): Promise<void> => {
    if (this.selectionIsMerged()) await this.unmergeCells();
    else await this.mergeCells();
  };

  /** Freezes the rows above and/or the columns left of the active cell. */
  freeze = async (what: "both" | "rows" | "cols"): Promise<void> => {
    const { rows, cols } = freezeAt(this.selection.active, what);
    if (rows === 0 && cols === 0) {
      useUiStore
        .getState()
        .pushToast(
          "info",
          what === "rows"
            ? t("msg.toast.freezeRows", "Select a cell below the rows you want to keep in view.")
            : what === "cols"
              ? t("msg.toast.freezeCols", "Select a cell right of the columns you want to keep in view.")
              : t("msg.toast.freezeBoth", "Select a cell below and right of what you want to keep in view."),
        );
      return;
    }
    await this.apply(t("cmd.view.freeze", "Freeze panes"), [{ type: "setFrozen", sheet: this.sheet, rows, cols }]);
  };

  unfreeze = (): Promise<ApplyResult | null> => this.apply(t("cmd.view.unfreeze", "Unfreeze panes"), [{ type: "setFrozen", sheet: this.sheet, rows: 0, cols: 0 }]);

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
    this.applyPatch(preset === "none" ? t("msg.undo.removeBorders", "Remove borders") : t("toolbar.borders", "Borders"), { ...EMPTY_PATCH, borders: bordersPatch(preset, line) });

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
    const res = await this.apply(replacing ? t("dlg.editName", "Edit name") : t("dlg.defineName", "Define name"), ops);
    await this.refreshNames();
    return res !== null && res.errors.length === 0;
  };

  deleteName = async (n: NameInfo): Promise<void> => {
    await this.apply(t("msg.undo.deleteName", "Delete name"), [{ type: "deleteName", name: n.name, scope: n.scope }]);
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
      // With several sheets selected (and no cell scope) Find searches them (AOO).
      const group = !options.allSheets && !options.scope && this.grouped ? this.selectedSheets : null;
      const key = JSON.stringify([text, options, this.sheet, options.allSheets ? null : this.sheet, group]);
      let { hits, index } = st.find;
      if (st.find.searched !== key) {
        // All sheets: the hits rotate from the engine's active sheet.
        await this.activeSheetSynced();
        const query = findQuery(text, options, this.sheet, { ...this.selection.active });
        const res = await this.client.find(doc, group ? { ...query, sheet: null } : query);
        hits = group ? res.hits.filter((h) => group.includes(h.sheet)) : res.hits;
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
      if (hit.sheet !== this.sheet) this.showSheet(hit.sheet, false, group !== null);
      const cell = { r: hit.r, c: hit.c };
      const sel = { range: { r0: cell.r, c0: cell.c, r1: cell.r, c1: cell.c }, active: cell, anchor: cell };
      useSelectionStore.getState().setSelection(sel);
      this.grid?.scrollCellIntoView(cell);
    } catch (e) {
      this.notifyError(e);
    }
  };

  /**
   * Replace (AOO's single Replace): when the active cell (or its note, in
   * Search in Notes) is a match, replaces in it, then finds the next match.
   */
  replaceOne = async (text: string, replace: string): Promise<void> => {
    try {
      const doc = this.requireDoc();
      const st = useDataStore.getState();
      const a = this.selection.active;
      const query = { ...findQuery(text, st.find.options, this.sheet, null), sheet: this.sheet, selection: [{ r0: a.r, c0: a.c, r1: a.r, c1: a.c }] };
      const scope = st.find.options.scope;
      const inScope = !scope || (scope.sheet === this.sheet && scope.ranges.some((r) => a.r >= r.r0 && a.r <= r.r1 && a.c >= r.c0 && a.c <= r.c1));
      const hit = inScope ? await this.client.find(doc, query) : { hits: [] };
      if (hit.hits.length > 0) {
        // The active cell only: never a group edit.
        await this.apply(t("dlg.replace", "Replace"), [{ type: "replaceAll", sheet: this.sheet, query, replace }], undefined, false, { group: false });
      }
      useDataStore.getState().patchFind({ text, searched: null, hits: [], index: -1 });
      await this.findStep(1);
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
      const res = await this.apply(t("dlg.replaceAll", "Replace all"), [{ type: "replaceAll", sheet: query.sheet, query, replace }]);
      if (!res) return 0;
      const n = res.replaced ?? Math.max(0, before.hits.length - res.errors.length);
      const skipped = res.errors.length;
      useDataStore.getState().patchFind({ searched: null, hits: [], index: -1 });
      useUiStore
        .getState()
        .pushToast(
          "info",
          n === 0 && skipped === 0
            ? t("msg.toast.noMatch", "No cells match “{text}”.", { text })
            : n === 1
              ? skipped > 0
                ? t("msg.toast.replacedOneSkipped", "Replaced 1 cell ({skipped} skipped).", { skipped })
                : t("msg.toast.replacedOne", "Replaced 1 cell.")
              : skipped > 0
                ? t("msg.toast.replacedManySkipped", "Replaced {n} cells ({skipped} skipped).", { n, skipped })
                : t("msg.toast.replacedMany", "Replaced {n} cells.", { n }),
        );
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
    const active = st.active;
    // From another sheet: the array goes over the edited cell's selection on its own sheet
    // (the shown sheet's array block, in `active`, is not the edited cell's).
    const away = this.returnToEditSheet();
    if (away) this.revealSoon(edit.cell);
    const origin = st.origin;
    const block = !away && active && active.sheet === this.sheet && active.cell.r === edit.cell.r && active.cell.c === edit.cell.c ? active.arrayRange : null;
    const sel = this.selection.range;
    const inSel = edit.cell.r >= sel.r0 && edit.cell.r <= sel.r1 && edit.cell.c >= sel.c0 && edit.cell.c <= sel.c1;
    const range = block ?? (inSel ? { ...sel } : { r0: edit.cell.r, c0: edit.cell.c, r1: edit.cell.r, c1: edit.cell.c });
    const op = arrayFormulaOp(this.sheet, range, this.w56.commitText(edit.text));
    if (!op) {
      useUiStore.getState().pushToast("info", t("msg.toast.arrayFormulaHint", "An array formula starts with = (type it, then press ⌃⇧↩)."));
      return;
    }
    st.end();
    if (origin === "bar") this.focusGrid();
    const tl = { r: range.r0, c: range.c0 };
    await this.apply(t("msg.undo.arrayFormula", "Array formula"), [op], { range, active: tl, anchor: tl });
  };

  /** ⌘D / ⌘R: a copy, not AutoFill (AOO Edit ▸ Fill ▸ Down / Right is `FillSimple`). */
  fillDir = async (dir: "down" | "right"): Promise<void> => {
    const plan = fillDownRight(this.selection.range, dir);
    if (!plan) return;
    await this.onFill(plan.source, plan.target, "copy");
  };

  /** ⇧⌘F9: hard recalculation. */
  recalcAll = async (): Promise<void> => {
    try {
      this.handleResult(await this.client.recalcAll(this.requireDoc()));
      useUiStore.getState().pushToast("info", t("msg.toast.recalculated", "Recalculated every formula."));
    } catch (e) {
      this.notifyError(e);
    }
  };

  /** Moves the active sheet (with a group, the selected sheets as a block) one tab left/right. */
  moveSheetBy = async (delta: -1 | 1): Promise<void> => {
    const info = this.info;
    if (!info) return;
    const ids = this.selectedSheets;
    const order = info.sheets.map((s) => s.id);
    const idx = ids.map((id) => order.indexOf(id)).filter((i) => i >= 0);
    if (idx.length === 0) return;
    // The block jumps over the nearest unselected tab: right of the first selected one, or left of the last one.
    let at = -1;
    if (delta > 0) {
      for (let i = Math.min(...idx) + 1; i < order.length && at < 0; i++) if (!ids.includes(order[i]!)) at = i + 1;
    } else {
      for (let i = Math.max(...idx) - 1; i >= 0 && at < 0; i--) if (!ids.includes(order[i]!)) at = i;
    }
    if (at < 0) return;
    const steps = moveBlockSteps(order, ids, at);
    if (steps.length === 0) return;
    const label = ids.length > 1 ? t("msg.undo.moveSheets", "Move sheets") : t("msg.undo.moveSheet", "Move sheet");
    await this.apply(label, steps.map((m): Op => ({ type: "moveSheet", sheet: m.sheet, to: m.to })));
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
