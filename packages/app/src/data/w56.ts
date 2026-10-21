/**
 * `W56Features`: the wave-5 (W5-6) half of the controller — DataPilot,
 * the data-tool dialogs and ops, outline, scenarios, cell styles and
 * AutoFormat, Format Cells, Tools ▸ Options and the app prefs, AutoInput
 * and AutoCorrect, the Navigator, document statistics, autosave and
 * recovery, templates, the UI language and the extra shortcuts. Like
 * `V3Features` it talks to the engine only through the `AppController`
 * (`ctl.apply`, `ctl.client`), so undo, selection and errors stay in one
 * place. The React side (`W56Chrome`, the dialogs and panels) reads
 * `useW56Store` and calls these methods.
 */
import {
  isApiError,
  type ApplyResult,
  type CalcSettingsDto,
  type CellPos,
  type CellStyleDto,
  type CellView,
  type DatabaseRangeDto,
  type DeleteContentsFlags,
  type EngineClientV4,
  type FunctionInfo,
  type NavigatorDto,
  type Op,
  type PivotDto,
  type PivotFieldItemsDto,
  type PivotSource,
  type RangeDto,
  type SheetLayout,
  type ScenarioDto,
  type SheetCell,
  type SolverRequest,
  type SolverResult,
  type StyleDto,
  type StylePatch,
  type ViewOptionsDto,
  type WorkbookInfo,
} from "@tally/bridge";
import { cellSelection, drawStyleId, MAX_COLS, MAX_ROWS, type OutlineGroup, type Selection } from "@tally/grid";
import type { AppController } from "../controller";
import { a1 } from "../dataOps";
import { EMPTY_PATCH } from "../formats";
import { useDataStore } from "../stores/dataStore";
import { useDocStore } from "../stores/docStore";
import { useEditStore } from "../stores/editStore";
import { useFunctionsStore } from "../stores/functionsStore";
import { useSelectionStore, selectionRanges } from "../stores/selectionStore";
import { useUiStore } from "../stores/uiStore";
import { useV3Store } from "../stores/v3Store";
import { applyAutoCorrectToEdit } from "../autocorrect/autocorrect";
import { ghostSuffix, wantsCompletion } from "../autocorrect/autoinput";
import { formulaLangOf, localizeFunctions, toCanonicalFormula, toDisplayFormula, type FormulaLang } from "../i18n/formulaNames";
import { setUiLanguage as installUiLanguage } from "../i18n/locales";
import { DEFAULT_PREFS, parsePrefs, readLocalPrefs, serializePrefs, writeLocalPrefs, type AppPrefs, type UiLanguage } from "../options/prefs";
import { emptyLayout, fromPivotDto, layoutError, pivotAtCell, toPivotDto, type PivotLayoutState } from "../pivot/pivotModel";
import { stateFromStyle } from "../styles/formatCells";
import { copySheetName, deleteContentsOp, groupAxisFor, hiddenContentCount, mergedContent, maxOutlineLevel } from "./builders";
import { DEFAULT_VIEW, useW56Store, type OptionsTab, type RecoveryFile, type W56Dialog } from "./store";
import type { PopoverItem } from "../components/Popover";
import type { Command } from "../registry";
import { t } from "../i18n";

/** Optional desktop-shell commands used here (`TauriClient`; the browser mock has stand-ins). */
export interface W56Shell {
  getPrefs?(): Promise<string>;
  setPrefs?(json: string): Promise<void>;
  autosave?(doc: number): Promise<void>;
  listRecovery?(): Promise<RecoveryFile[]>;
  recoverSnapshot?(path: string): Promise<WorkbookInfo>;
  discardRecovery?(path: string): Promise<void>;
  saveAsTemplate?(doc: number, name: string): Promise<void>;
  listUserTemplates?(): Promise<{ id: string; title: string; description: string }[]>;
  newFromUserTemplate?(id: string): Promise<WorkbookInfo>;
  setUiLanguage?(lang: string): Promise<void>;
}

/** Autocomplete debounce while typing (ms). */
export const AUTOINPUT_DEBOUNCE_MS = 60;
const LAST_ROW = MAX_ROWS - 1;
const LAST_COL = MAX_COLS - 1;

export class W56Features {
  /** The source whose layout feeds the outline gutter, and its last layout seen. */
  private offSource: (() => void) | null = null;
  private lastLayout: SheetLayout | null = null;
  private viewSeq = 0;
  private canonicalFunctions: FunctionInfo[] = [];
  private lang: FormulaLang | null = null;
  private autoInputSeq = 0;
  private autoInputTimer: ReturnType<typeof setTimeout> | null = null;
  private autosaveTimer: ReturnType<typeof setInterval> | null = null;
  private rebuild: (() => Command[]) | null = null;
  private prefsLoaded = false;

  constructor(private readonly ctl: AppController) {}

  private get client(): EngineClientV4 {
    return this.ctl.client as EngineClientV4;
  }

  private get shell(): W56Shell {
    return this.ctl.shell as unknown as W56Shell;
  }

  private get doc(): number {
    return this.ctl.requireDoc();
  }

  private get sheet(): number {
    return this.ctl.sheet;
  }

  private get store() {
    return useW56Store.getState();
  }

  private toast(kind: "info" | "error" | "warning", message: string): void {
    useUiStore.getState().pushToast(kind, message);
  }

  // -- lifecycle ------------------------------------------------------------------

  /** Called from `AppController.attach()`; returns the cleanup. */
  attach(): () => void {
    const offs: (() => void)[] = [];
    // ⌘⇧Y: remember the last command that ran (wraps the registry's run).
    const registry = this.ctl.registry;
    const run = registry.run.bind(registry);
    registry.run = (id, args) => {
      const ok = run(id, args);
      if (ok && id !== "edit.repeat" && id !== "palette.toggle") useW56Store.getState().setLastCommand(id);
      return ok;
    };
    offs.push(() => {
      registry.run = run;
    });
    // Paint-can mode and the active pivot follow the selection.
    offs.push(
      useSelectionStore.subscribe((s, prev) => {
        if (s.selection === prev.selection) return;
        const paint = this.store.paintStyle;
        if (paint && !useEditStore.getState().edit) void this.applyCellStyle(paint, selectionRanges(s.selection));
        this.updateActivePivot(s.selection.active);
      }),
    );
    // The document locale decides the displayed function names.
    offs.push(
      useV3Store.subscribe((s, prev) => {
        if (s.locale !== prev.locale) this.setFormulaLang(formulaLangOf(s.locale));
      }),
    );
    offs.push(
      useEditStore.subscribe((s, prev) => {
        if (s.edit === null && prev.edit !== null) this.store.setGhost(null);
      }),
    );
    void this.loadPrefs();
    void this.offerRecovery();
    return () => {
      offs.splice(0).forEach((f) => f());
      this.offSource?.();
      this.offSource = null;
      if (this.autosaveTimer) clearInterval(this.autosaveTimer);
      this.autosaveTimer = null;
    };
  }

  /** The command list builder (App.tsx), used to re-register titles after a language switch. */
  setCommandRebuilder(f: () => Command[]): void {
    this.rebuild = f;
  }

  /** A workbook was loaded. */
  onLoad(): void {
    this.lists = null;
    this.store.reset();
    this.setFormulaLang(formulaLangOf(useV3Store.getState().locale));
    this.watchSource();
    void this.refreshView(true);
    void this.refreshPivots();
    void this.refreshCellStyles();
    this.restartAutosave();
  }

  /** The visible sheet changed. */
  onSheet(): void {
    this.syncOutline();
    void this.refreshView(true);
    this.updateActivePivot(this.ctl.selection.active);
  }

  /** The engine's function list (canonical names) arrived. */
  setFunctions(fns: FunctionInfo[]): void {
    this.canonicalFunctions = [...fns];
    useFunctionsStore.getState().set(localizeFunctions(this.lang, this.canonicalFunctions));
  }

  private setFormulaLang(lang: FormulaLang | null): void {
    if (lang === this.lang) return;
    this.lang = lang;
    if (this.canonicalFunctions.length > 0) useFunctionsStore.getState().set(localizeFunctions(lang, this.canonicalFunctions));
    void this.ctl.refreshActive();
  }

  /** After an engine result: pivots, outline (undo/redo) and view models. */
  afterResult(res: ApplyResult, ops: readonly Op[] | null): void {
    if (res.pivotsChanged || ops?.some((o) => o.type === "createPivot" || o.type === "updatePivot" || o.type === "deletePivot" || o.type === "refreshPivot" || o.type === "pivotDrillDown")) void this.refreshPivots();
    if (ops?.some((o) => o.type === "defineCellStyle" || o.type === "deleteCellStyle")) void this.refreshCellStyles();
    if (ops?.some((o) => o.type === "createScenario" || o.type === "deleteScenario" || o.type === "updateScenario")) void this.refreshNavigatorPanel();
    // Undo/redo (ops === null), view-option edits and layout changes: the engine's view options are re-read.
    if (ops === null || ops.some((o) => o.type === "setViewOptions") || res.layoutChanged.includes(this.sheet)) void this.refreshView(false);
    if (ops === null || ops.some((o) => o.type === "setCustomSortLists")) this.lists = null;
  }

  // -- outline (Data ▸ Group and Outline) -----------------------------------------
  // The groups come from the engine's `SheetLayout.outline`, which the grid's
  // source refetches whenever an op reports `layoutChanged` (outline ops,
  // undo/redo, structure edits).

  /** Follows the current document's grid source (re-attached on every load). */
  private watchSource(): void {
    this.offSource?.();
    this.offSource = null;
    this.lastLayout = null;
    const src = useDocStore.getState().source;
    if (src) this.offSource = src.subscribe(() => this.syncOutline());
    this.syncOutline();
  }

  /** Publishes the active sheet's outline groups from the source's layout (when it changed). */
  private syncOutline(): void {
    const src = useDocStore.getState().source;
    if (!src) return;
    const layout = src.getLayout(src.sheetId);
    if (layout === this.lastLayout) return;
    this.lastLayout = layout;
    const o = layout.outline;
    const next = { rows: (o?.rows ?? []).map((g) => ({ ...g })), cols: (o?.cols ?? []).map((g) => ({ ...g })) };
    if (JSON.stringify(next) !== JSON.stringify(this.store.outline)) this.store.setOutline(next);
  }

  private async outlineOp(op: Op, label: string): Promise<void> {
    await this.ctl.apply(label, [op]);
  }

  /** F12: groups the selected rows (or columns); a single-cell selection groups by the longer side. */
  async group(): Promise<void> {
    const r = this.ctl.selection.range;
    const axis = groupAxisFor(r, r.c0 === 0 && r.c1 === LAST_COL, r.r0 === 0 && r.r1 === LAST_ROW);
    if (axis === null) {
      this.toast("info", "Select rows or columns to group.");
      return;
    }
    const [start, end] = axis === "rows" ? [r.r0, r.r1] : [r.c0, r.c1];
    await this.outlineOp({ type: "group", sheet: this.sheet, axis, start, end }, "Group");
  }

  /** ⌘F12. */
  async ungroup(): Promise<void> {
    const r = this.ctl.selection.range;
    const axis = groupAxisFor(r, r.c0 === 0 && r.c1 === LAST_COL, r.r0 === 0 && r.r1 === LAST_ROW);
    if (axis === null) {
      this.toast("info", "Select grouped rows or columns to ungroup.");
      return;
    }
    const [start, end] = axis === "rows" ? [r.r0, r.r1] : [r.c0, r.c1];
    await this.outlineOp({ type: "ungroup", sheet: this.sheet, axis, start, end }, "Ungroup");
  }

  async autoOutline(): Promise<void> {
    const r = this.ctl.selection.range;
    const single = r.r0 === r.r1 && r.c0 === r.c1;
    await this.outlineOp({ type: "autoOutline", sheet: this.sheet, range: single ? null : { ...r } }, "AutoOutline");
  }

  async clearOutline(): Promise<void> {
    await this.outlineOp({ type: "clearOutline", sheet: this.sheet, axis: null }, "Remove outline");
  }

  /** The grid's +/− box. */
  onOutlineToggle = (axis: "rows" | "cols", g: OutlineGroup): void => {
    void this.outlineOp({ type: "setOutlineCollapsed", sheet: this.sheet, axis, start: g.start, end: g.end, collapsed: !g.collapsed }, g.collapsed ? "Show details" : "Hide details");
  };

  /** The grid's level button. */
  onOutlineLevel = (axis: "rows" | "cols", level: number): void => {
    void this.outlineOp({ type: "showOutlineLevel", sheet: this.sheet, axis, level }, t("outline.level", "Level {n}", { n: level }));
  };

  openShowOutlineLevel(): void {
    const o = this.store.outline;
    const axis = o.rows.length >= o.cols.length ? "rows" : "cols";
    const max = Math.max(maxOutlineLevel(o.rows), maxOutlineLevel(o.cols));
    if (max === 0) {
      this.toast("info", "There is no outline on this sheet (Data ▸ Group).");
      return;
    }
    this.store.openDialog({ kind: "showOutlineLevel", axis, max });
  }

  // -- DataPilot -------------------------------------------------------------------

  async refreshPivots(): Promise<void> {
    const info = this.ctl.info;
    if (!info) return;
    try {
      const pivots = await this.client.pivots(info.doc);
      if (this.ctl.info?.doc === info.doc) {
        this.store.setPivots(pivots);
        this.updateActivePivot(this.ctl.selection.active);
      }
    } catch {
      this.store.setPivots([]);
    }
  }

  private updateActivePivot(cell: CellPos): void {
    const p = pivotAtCell(this.store.pivots, this.sheet, cell);
    const id = p?.id ?? null;
    if (id !== this.store.activePivot) this.store.setActivePivot(id);
  }

  /** The pivot the active cell is in, if any. */
  activePivot(): PivotDto | null {
    const id = this.store.activePivot;
    return id === null ? null : (this.store.pivots.find((p) => p.id === id) ?? null);
  }

  /** Data ▸ DataPilot ▸ Start…: the layout dialog over the data region (or the pivot under the cursor). */
  async openDataPilot(existing: PivotDto | null = this.activePivot()): Promise<void> {
    try {
      const doc = this.doc;
      let source: PivotSource;
      let range: RangeDto | null = null;
      if (existing) source = existing.source;
      else {
        range = await this.ctl.dataRegion();
        if (!range || range.r0 === range.r1) {
          this.toast("info", "Select a table with a header row (or a cell in one) to summarise.");
          return;
        }
        source = { type: "range", sheet: this.sheet, range };
      }
      const fields = await this.client.pivotFieldItems(doc, source);
      const layout = existing ? fromPivotDto(existing, fields) : emptyLayout(source, range, this.sheet);
      this.store.openDialog({ kind: "dataPilot", layout, fields, existing });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  /** Create / Update from the dialog; asks before overwriting a non-empty target. */
  async savePivot(layout: PivotLayoutState, existing: PivotDto | null, overwrite = false): Promise<boolean> {
    const err = layoutError(layout);
    if (err) {
      this.toast("info", err);
      return false;
    }
    let targetSheet = layout.targetSheet;
    const ops: Op[] = [];
    if (targetSheet === null) {
      // A new sheet: add it first (one undo step with the pivot).
      const res = await this.ctl.apply("Insert sheet", [{ type: "addSheet", name: null, index: null }]);
      const added = res?.info?.sheets.find((s) => !useDocStore.getState().info?.sheets.some((o) => o.id === s.id)) ?? res?.info?.sheets[res.info.sheets.length - 1];
      if (!res || !added) return false;
      targetSheet = added.id;
      layout = { ...layout, targetSheet, target: { r: 0, c: 0 } };
    }
    const dto = toPivotDto(layout, targetSheet, existing?.id ?? 0, existing?.name ?? "");
    ops.push(existing ? { type: "updatePivot", id: existing.id, pivot: dto, overwrite } : { type: "createPivot", pivot: dto, overwrite });
    const res = await this.ctl.apply(existing ? "Update pivot table" : "Create pivot table", ops);
    if (!res) return false;
    const notEmpty = res.errors.find((e) => e.code === "target_not_empty");
    if (notEmpty) {
      useUiStore.getState().openDialog({
        kind: "confirm",
        title: t("pivot.title", "DataPilot"),
        message: t("dialog.overwrite", "The target range is not empty. Overwrite it?"),
        ok: t("dialog.yes", "Yes"),
        danger: true,
        onConfirm: () => void this.savePivot(layout, existing, true),
      });
      return false;
    }
    if (res.errors.length > 0) return false;
    if (targetSheet !== this.sheet) this.ctl.showSheet(targetSheet);
    await this.refreshPivots();
    return true;
  }

  async refreshPivot(p: PivotDto | null = this.activePivot(), overwrite = false): Promise<void> {
    if (!p) {
      this.toast("info", "Put the cursor inside a pivot table to refresh it.");
      return;
    }
    const res = await this.ctl.apply("Refresh pivot table", [{ type: "refreshPivot", id: p.id, overwrite }]);
    if (res?.errors.some((e) => e.code === "target_not_empty")) {
      useUiStore.getState().openDialog({
        kind: "confirm",
        title: t("pivot.title", "DataPilot"),
        message: t("pivot.refreshOverwrite", "The pivot table output will overwrite existing cells. Continue?"),
        ok: t("dialog.yes", "Yes"),
        danger: true,
        onConfirm: () => void this.refreshPivot(p, true),
      });
    }
  }

  async deletePivot(p: PivotDto | null = this.activePivot(), clearOutput = true): Promise<void> {
    if (!p) return;
    await this.ctl.apply("Delete pivot table", [{ type: "deletePivot", id: p.id, clearOutput }]);
  }

  /**
   * Double-click on a pivot data cell: drill down (a new sheet with the
   * source rows). Returns true when the click was consumed (no edit).
   */
  maybeDrillDown(cell: CellPos): boolean {
    const p = pivotAtCell(this.store.pivots, this.sheet, cell);
    if (!p || !p.drillDown) return false;
    const info = this.ctl.info;
    if (!info) return false;
    void this.client.pivotAt(info.doc, this.sheet, cell.r, cell.c).then(
      (hit) => {
        if (hit && (hit.role === "data" || hit.role === "total")) void this.ctl.apply("Drill down", [{ type: "pivotDrillDown", id: hit.pivot, r: cell.r, c: cell.c }]);
        else this.ctl.startEdit(cell, undefined, "grid");
      },
      () => this.ctl.startEdit(cell, undefined, "grid"),
    );
    return true;
  }

  // -- filters, subtotals, data tools ---------------------------------------------

  private async regionWithLabels(): Promise<{ range: RangeDto; labels: string[]; hasHeader: boolean } | null> {
    const range = await this.ctl.dataRegion();
    if (!range) {
      this.toast("info", "Select a table (or a cell in one).");
      return null;
    }
    const hasHeader = range.r1 > range.r0 ? await this.client.detectHeader(this.doc, this.sheet, range).catch(() => false) : false;
    const [block] = await this.client.cells(this.doc, this.sheet, [{ ...range, r1: range.r0 }]);
    const labels: string[] = [];
    for (let c = range.c0; c <= range.c1; c++) {
      const text = block?.cells.find((v) => v.c === c)?.text.trim();
      labels.push(text && hasHeader ? text : `Column ${a1({ r0: 0, c0: c, r1: 0, c1: c }).replace(/\d+$/, "")}`);
    }
    return { range, labels, hasHeader: hasHeader || useDataStore.getState().filter !== null };
  }

  async openStandardFilter(advanced = false): Promise<void> {
    try {
      const r = await this.regionWithLabels();
      if (r) this.store.openDialog({ kind: "standardFilter", ...r, advanced });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async resetFilter(): Promise<void> {
    await this.ctl.apply("Reset filter", [{ type: "resetFilter", sheet: this.sheet }]);
    await this.ctl.refreshFilter();
  }

  async openSubtotals(): Promise<void> {
    try {
      const r = await this.regionWithLabels();
      if (r) this.store.openDialog({ kind: "subtotals", range: r.range, labels: r.labels });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  openConsolidate(): void {
    this.store.openDialog({ kind: "consolidate" });
  }

  async openTextToColumns(): Promise<void> {
    try {
      const sel = this.ctl.selection.range;
      const col = sel.c0;
      const range = { r0: sel.r0, c0: col, r1: Math.min(sel.r1, sel.r0 + 100_000), c1: col };
      const [block] = await this.client.cells(this.doc, this.sheet, [{ ...range, r1: Math.min(range.r1, range.r0 + 200) }]);
      const texts: string[] = [];
      for (let r = range.r0; r <= Math.min(range.r1, range.r0 + 200); r++) texts.push(block?.cells.find((v) => v.r === r)?.text ?? "");
      while (texts.length > 1 && texts[texts.length - 1] === "") texts.pop();
      this.store.openDialog({ kind: "textToColumns", range, texts });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async openRemoveDuplicates(): Promise<void> {
    try {
      const r = await this.regionWithLabels();
      if (r) this.store.openDialog({ kind: "removeDuplicates", ...r });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async openScenarios(): Promise<void> {
    try {
      const list = await this.client.scenarios(this.doc, this.sheet).catch(() => [] as ScenarioDto[]);
      this.store.openDialog({ kind: "scenarios", list });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async createScenario(s: ScenarioDto): Promise<boolean> {
    const res = await this.ctl.apply("Create scenario", [{ type: "createScenario", sheet: this.sheet, scenario: s }]);
    return res !== null && res.errors.length === 0;
  }

  async showScenario(id: number): Promise<void> {
    await this.ctl.apply("Show scenario", [{ type: "showScenario", sheet: this.sheet, id }]);
  }

  async deleteScenario(id: number): Promise<void> {
    await this.ctl.apply("Delete scenario", [{ type: "deleteScenario", sheet: this.sheet, id }]);
  }

  async openDefineDatabaseRange(): Promise<void> {
    try {
      const range = (await this.ctl.dataRegion()) ?? { ...this.ctl.selection.range };
      const existing = await this.client.databaseRanges(this.doc).catch(() => [] as DatabaseRangeDto[]);
      this.store.openDialog({ kind: "defineDatabaseRange", range, existing });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async deleteDatabaseRange(name: string): Promise<void> {
    await this.ctl.apply("Delete database range", [{ type: "deleteDatabaseRange", name }]);
  }

  openCreateNames(): void {
    const r = this.ctl.selection.range;
    if (r.r0 === r.r1 && r.c0 === r.c1) {
      this.toast("info", "Select the labels together with the cells they name.");
      return;
    }
    this.store.openDialog({ kind: "createNamesFromLabels", range: { ...r } });
  }

  openFillSeries(): void {
    const r = this.ctl.selection.range;
    if (r.r0 === r.r1 && r.c0 === r.c1) {
      this.toast("info", "Select the cells to fill (the first one holds the start value).");
      return;
    }
    this.store.openDialog({ kind: "fillSeries", range: { ...r } });
  }

  async openSortOptions(): Promise<void> {
    try {
      const r = await this.regionWithLabels();
      if (!r) return;
      this.store.openDialog({ kind: "sortOptions", ...r, lists: await this.customLists() });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  /** The document's custom sort lists from `custom_sort_lists` (cached until they change, an undo or a load). */
  private lists: string[][] | null = null;

  async customLists(): Promise<string[][]> {
    if (this.lists) return this.lists.map((l) => [...l]);
    const doc = this.doc;
    let lists: string[][] = [];
    try {
      lists = await this.client.customSortLists(doc);
    } catch (e) {
      if (!isApiError(e) || e.code !== "unsupported") this.ctl.notifyError(e);
    }
    if (this.ctl.info?.doc === doc) this.lists = lists.map((l) => [...l]);
    return lists;
  }

  async setCustomLists(lists: string[][]): Promise<void> {
    const res = await this.ctl.apply("Sort lists", [{ type: "setCustomSortLists", lists: lists.map((l) => [...l]) }]);
    if (res && res.errors.length === 0) this.lists = null;
  }

  /** The Delete key (AOO's Delete Contents dialog). */
  openDeleteContents(): void {
    this.store.openDialog({ kind: "deleteContents", ranges: this.ctl.ranges });
  }

  async deleteContents(ranges: readonly RangeDto[], what: DeleteContentsFlags): Promise<void> {
    await this.ctl.apply("Delete contents", [deleteContentsOp(this.sheet, ranges, what)]);
  }

  openMultipleOperations(): void {
    const r = this.ctl.selection.range;
    if (r.r0 === r.r1 && r.c0 === r.c1) {
      this.toast("info", "Select the table area: the variable values and the cells to fill.");
      return;
    }
    this.store.openDialog({ kind: "multipleOperations", range: { ...r } });
  }

  openInsertCells(): void {
    this.store.openDialog({ kind: "insertCells", range: { ...this.ctl.selection.range } });
  }

  openDeleteCells(): void {
    this.store.openDialog({ kind: "deleteCells", range: { ...this.ctl.selection.range } });
  }

  async insertCells(range: RangeDto, shift: "down" | "right"): Promise<void> {
    await this.ctl.apply("Insert cells", [{ type: "insertCells", sheet: this.sheet, range: { ...range }, shift }]);
  }

  async deleteCells(range: RangeDto, shift: "up" | "left"): Promise<void> {
    await this.ctl.apply("Delete cells", [{ type: "deleteCells", sheet: this.sheet, range: { ...range }, shift }], cellSelection({ r: range.r0, c: range.c0 }));
  }

  /** Applies any op list from a dialog (one undo step). */
  applyOps(label: string, ops: Op[], after?: Selection): Promise<ApplyResult | null> {
    return this.ctl.apply(label, ops, after);
  }

  /**
   * §51: merging cells whose hidden (non-top-left) cells hold content asks
   * "Move the contents of the hidden cells into the first cell?". Called by
   * `AppController.mergeCells` with the filled cells; returns true when it
   * showed the prompt (the caller then does nothing).
   */
  mergePrompt(range: RangeDto, filled: readonly CellView[], go: () => void): boolean {
    const hidden = hiddenContentCount(range, filled.map((v) => ({ r: v.r, c: v.c })));
    if (hidden === 0) return false;
    const sheet = this.sheet;
    this.store.openDialog({
      kind: "mergePrompt",
      range,
      hidden,
      onMove: () => {
        void (async () => {
          const texts: { pos: CellPos; input: string }[] = [];
          for (const v of filled) {
            const info = await this.client.cellEditInfo(this.doc, sheet, v.r, v.c).catch(() => null);
            texts.push({ pos: { r: v.r, c: v.c }, input: info?.input ?? v.text });
          }
          const input = mergedContent(texts, range);
          const tl = { r: range.r0, c: range.c0 };
          await this.ctl.apply("Merge cells", [
            { type: "setCellInput", sheet, r: tl.r, c: tl.c, input },
            { type: "mergeCells", sheet, range: { ...range } },
          ], { range: { ...range }, active: tl, anchor: tl });
        })();
      },
      onKeep: go,
    });
    return true;
  }

  // -- solver ----------------------------------------------------------------------

  openSolver(): void {
    const a = this.ctl.selection.active;
    this.store.openDialog({ kind: "solver", objective: a1({ r0: a.r, c0: a.c, r1: a.r, c1: a.c }) });
  }

  solve(req: SolverRequest): Promise<SolverResult> {
    return this.client.solver(this.doc, req);
  }

  async applySolverResult(res: SolverResult): Promise<void> {
    const ops: Op[] = res.values.map((v) => ({ type: "setCellInput", sheet: v.cell.sheet, r: v.cell.r, c: v.cell.c, input: String(Number(v.value.toPrecision(15))) }));
    if (ops.length > 0) await this.ctl.apply("Solver", ops);
  }

  // -- sheets ----------------------------------------------------------------------

  openCopySheet(sheet = this.sheet): void {
    const info = this.ctl.info;
    if (!info) return;
    const name = copySheetName(this.ctl.sheetName(sheet), info.sheets.map((s) => s.name));
    this.store.openDialog({ kind: "copySheet", sheet, name });
  }

  async copySheet(sheet: number, name: string, index: number | null): Promise<void> {
    await this.ctl.apply("Copy sheet", [{ type: "copySheet", sheet, name: name.trim() === "" ? null : name.trim(), index }]);
  }

  async hideSheet(sheet = this.sheet): Promise<void> {
    const info = this.ctl.info;
    if (!info) return;
    if (info.sheets.filter((s) => !s.hidden).length <= 1) {
      this.toast("info", "The last visible sheet cannot be hidden.");
      return;
    }
    await this.ctl.apply("Hide sheet", [{ type: "setSheetHidden", sheet, hidden: true }]);
    if (sheet === this.sheet) {
      const next = useDocStore.getState().info?.sheets.find((s) => !s.hidden && s.id !== sheet);
      if (next) this.ctl.showSheet(next.id);
    }
  }

  openShowSheet(): void {
    const hidden = (this.ctl.info?.sheets ?? []).filter((s) => s.hidden).map((s) => ({ id: s.id, name: s.name }));
    if (hidden.length === 0) {
      this.toast("info", "There are no hidden sheets.");
      return;
    }
    this.store.openDialog({ kind: "showSheet", hidden });
  }

  async showSheets(ids: number[]): Promise<void> {
    if (ids.length === 0) return;
    await this.ctl.apply("Show sheet", ids.map((sheet): Op => ({ type: "setSheetHidden", sheet, hidden: false })));
    const first = ids[0];
    if (first !== undefined) this.ctl.showSheet(first);
  }

  openTabColor(sheet = this.sheet): void {
    const current = this.ctl.info?.sheets.find((s) => s.id === sheet)?.tabColor ?? null;
    const rgba = current ? Number.parseInt(current.replace("#", "").padEnd(8, "f"), 16) : null;
    this.store.openDialog({ kind: "tabColor", sheet, color: rgba === null || Number.isNaN(rgba) ? null : rgba });
  }

  async setTabColor(sheet: number, color: number | null): Promise<void> {
    await this.ctl.apply("Tab colour", [{ type: "setTabColor", sheet, color }]);
  }

  /** Extra items of the sheet-tab context menu. */
  sheetTabItems(sheet: number): PopoverItem[] {
    const info = this.ctl.info;
    const visible = (info?.sheets ?? []).filter((s) => !s.hidden).length;
    const hidden = (info?.sheets ?? []).some((s) => s.hidden);
    return [
      { id: "copy", label: t("cmd.sheet.duplicate", "Copy sheet…"), separator: true, onSelect: () => this.openCopySheet(sheet) },
      { id: "tabColor", label: t("cmd.sheet.tabColor", "Tab colour…"), onSelect: () => this.openTabColor(sheet) },
      { id: "hide", label: t("cmd.sheet.hide", "Hide sheet"), disabled: visible <= 1, onSelect: () => void this.hideSheet(sheet) },
      { id: "show", label: t("cmd.sheet.show", "Show sheet…"), disabled: !hidden, onSelect: () => this.openShowSheet() },
    ];
  }

  /** Extra items of the cell context menu (insert/delete cells, delete contents, pivot). */
  cellMenuItems(cell: CellPos): PopoverItem[] {
    const out: PopoverItem[] = [
      { id: "deleteContents", label: t("cmd.edit.deleteContents", "Delete contents…"), hint: "⌦", onSelect: () => this.openDeleteContents() },
      { id: "insertCells", label: t("cmd.sheet.insertCells", "Insert cells…"), separator: true, onSelect: () => this.openInsertCells() },
      { id: "deleteCells", label: t("cmd.sheet.deleteCells", "Delete cells…"), onSelect: () => this.openDeleteCells() },
      { id: "formatCells", label: t("cmd.format.cells", "Format cells…"), hint: "⌘1", onSelect: () => void this.openFormatCells() },
    ];
    const p = pivotAtCell(this.store.pivots, this.sheet, cell);
    if (p) {
      out.push(
        { id: "pivotRefresh", label: t("pivot.refresh", "Refresh"), separator: true, onSelect: () => void this.refreshPivot(p) },
        { id: "pivotEdit", label: t("pivot.layout", "Layout…"), onSelect: () => void this.openDataPilot(p) },
        { id: "pivotDelete", label: t("pivot.deleteTable", "Delete pivot table"), danger: true, onSelect: () => void this.deletePivot(p) },
      );
    }
    return out;
  }

  // -- cell styles, AutoFormat, Format Cells ----------------------------------------

  async refreshCellStyles(): Promise<void> {
    const info = this.ctl.info;
    if (!info) return;
    try {
      const styles = await this.client.cellStyles(info.doc);
      if (this.ctl.info?.doc === info.doc) this.store.setCellStyles(styles);
    } catch {
      this.store.setCellStyles([]);
    }
  }

  toggleStylesPanel(): void {
    this.store.togglePanel("styles");
    if (this.store.panel === "styles") void this.refreshCellStyles();
  }

  async applyCellStyle(name: string, ranges: RangeDto[] = this.ctl.ranges): Promise<void> {
    await this.ctl.apply(`Style: ${name}`, [{ type: "applyCellStyle", sheet: this.sheet, ranges: ranges.map((r) => ({ ...r })), name }]);
  }

  /** "New style from selection": the active cell's resolved style under a new name. */
  async newStyleFromSelection(): Promise<void> {
    try {
      const style = await this.ctl.activeStyle();
      const base = this.store.cellStyles.find((s) => s.name === "Default")?.style ?? style;
      if (!base) return;
      const n = this.store.cellStyles.length + 1;
      this.store.openDialog({ kind: "styleEditor", style: { name: `Style ${n}`, parent: "Default", builtin: false, style: style ?? base }, isNew: true, fromSelection: style ?? null });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  editStyle(s: CellStyleDto): void {
    this.store.openDialog({ kind: "styleEditor", style: structuredClone(s), isNew: false, fromSelection: null });
  }

  async defineCellStyle(s: CellStyleDto): Promise<boolean> {
    const res = await this.ctl.apply("Cell style", [{ type: "defineCellStyle", style: s }]);
    await this.refreshCellStyles();
    return res !== null && res.errors.length === 0;
  }

  async deleteCellStyle(name: string): Promise<void> {
    await this.ctl.apply("Delete cell style", [{ type: "deleteCellStyle", name }]);
    await this.refreshCellStyles();
  }

  togglePaintCan(name: string | null): void {
    this.store.setPaintStyle(this.store.paintStyle === name ? null : name);
  }

  async openAutoFormat(): Promise<void> {
    const range = (await this.ctl.dataRegion().catch(() => null)) ?? { ...this.ctl.selection.range };
    if (range.r0 === range.r1 || range.c0 === range.c1) {
      this.toast("info", "Select at least 2 rows and 2 columns to AutoFormat.");
      return;
    }
    this.store.openDialog({ kind: "autoFormat", range });
  }

  async applyAutoFormat(range: RangeDto, preset: string, options: Parameters<typeof this.autoFormatOp>[2]): Promise<void> {
    await this.ctl.apply(`AutoFormat ${preset}`, [this.autoFormatOp(range, preset, options)]);
  }

  private autoFormatOp(range: RangeDto, preset: string, options: Extract<Op, { type: "applyAutoFormat" }>["options"]): Op {
    return { type: "applyAutoFormat", sheet: this.sheet, range: { ...range }, preset, options: { ...options } };
  }

  /** ⌘1. */
  async openFormatCells(): Promise<void> {
    try {
      const style = await this.ctl.activeStyle();
      const base: StyleDto = style ?? { ...stateFromStyle(undefined), id: 0, borders: { top: null, bottom: null, left: null, right: null }, display: null, fontFamily: null, indent: 0 } as unknown as StyleDto;
      this.store.openDialog({ kind: "formatCells", style: base, ranges: this.ctl.ranges });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async applyFormatCells(ranges: RangeDto[], patch: StylePatch, protection: { locked: boolean | null; hiddenFormula: boolean | null } | null): Promise<void> {
    const ops: Op[] = [];
    const changes = Object.entries(patch).some(([, v]) => v !== null && v !== undefined);
    if (changes) ops.push({ type: "applyStyle", sheet: this.sheet, ranges: ranges.map((r) => ({ ...r })), patch });
    if (protection && (protection.locked !== null || protection.hiddenFormula !== null)) ops.push({ type: "setCellProtection", sheet: this.sheet, ranges: ranges.map((r) => ({ ...r })), locked: protection.locked, hiddenFormula: protection.hiddenFormula });
    if (ops.length > 0) await this.ctl.apply("Format cells", ops);
  }

  // -- view options and zoom ---------------------------------------------------------

  /**
   * Re-reads the active sheet's view options from the engine (`view_options`)
   * and applies them: the store (View menu checks, grid gridlines/headers),
   * the grid source's `cellsWith` options and, when `restoreSplit`, the
   * persisted Window ▸ Split.
   */
  async refreshView(restoreSplit: boolean): Promise<void> {
    const info = this.ctl.info;
    if (!info) return;
    const sheet = this.sheet;
    const seq = ++this.viewSeq;
    let v: ViewOptionsDto = DEFAULT_VIEW;
    try {
      v = await this.client.viewOptions(info.doc, sheet);
    } catch (e) {
      if (!isApiError(e) || (e.code !== "unsupported" && e.code !== "not_found")) this.ctl.notifyError(e);
    }
    if (seq !== this.viewSeq || this.ctl.info?.doc !== info.doc || this.sheet !== sheet) return;
    this.applyView(v);
    if (restoreSplit) this.ctl.w5.view.restoreSplit(v.split ?? null);
  }

  private applyView(v: ViewOptionsDto): void {
    const view: ViewOptionsDto = { ...DEFAULT_VIEW, ...v, split: v.split ?? null };
    if (JSON.stringify(view) !== JSON.stringify(this.store.view)) this.store.setView(view);
    useDocStore.getState().source?.setCellsOptions({ showFormulas: view.showFormulas, valueHighlighting: view.valueHighlighting, hideZeroValues: !view.showZeroValues });
  }

  /** The active sheet's toggles without the split (what `setViewOptions` takes). */
  private currentView(): ViewOptionsDto {
    const { split: _split, ...toggles } = this.store.view;
    return { ...DEFAULT_VIEW, ...toggles };
  }

  async setViewOption<K extends keyof ViewOptionsDto>(key: K, value: boolean): Promise<void> {
    await this.setViewOptions({ ...this.currentView(), [key]: value });
  }

  toggleView(key: Exclude<keyof ViewOptionsDto, "split">): void {
    void this.setViewOption(key, !this.currentView()[key]);
  }

  async setViewOptions(v: ViewOptionsDto): Promise<void> {
    const { split: _split, ...options } = v;
    const res = await this.ctl.apply("View options", [{ type: "setViewOptions", sheet: this.sheet, options: { ...DEFAULT_VIEW, ...options } }]);
    // The engine's answer is authoritative (afterResult re-reads it too; this covers results without a version bump).
    if (res && res.errors.length === 0) await this.refreshView(false);
  }

  /** File ▸ Open Recent ▸ Clear Menu (the shell keeps the list; the sidebar re-reads it). */
  async clearRecent(): Promise<void> {
    const shell = this.ctl.shell as unknown as { clearRecent?(): Promise<void> };
    if (!shell.clearRecent) {
      this.toast("info", "Clearing recent files needs the desktop app.");
      return;
    }
    await shell.clearRecent().catch((e: unknown) => this.ctl.notifyError(e));
    useDocStore.setState((st) => ({ rev: st.rev + 1 }));
  }

  // -- Tools ▸ Options and prefs -------------------------------------------------------

  async loadPrefs(): Promise<void> {
    if (this.prefsLoaded) return;
    this.prefsLoaded = true;
    let prefs: AppPrefs = readLocalPrefs();
    if (this.shell.getPrefs) {
      try {
        prefs = parsePrefs(JSON.parse(await this.shell.getPrefs()));
      } catch {
        prefs = DEFAULT_PREFS;
      }
    }
    this.store.setPrefs(prefs);
    if (prefs.uiLanguage !== "en") await this.applyUiLanguage(prefs.uiLanguage, false);
    this.restartAutosave();
  }

  async savePrefs(prefs: AppPrefs): Promise<void> {
    const prev = this.store.prefs;
    this.store.setPrefs(prefs);
    writeLocalPrefs(prefs);
    if (this.shell.setPrefs) {
      try {
        await this.shell.setPrefs(serializePrefs(prefs));
      } catch (e) {
        this.ctl.notifyError(e);
      }
    }
    if (prefs.uiLanguage !== prev.uiLanguage) await this.applyUiLanguage(prefs.uiLanguage, true);
    if (prefs.autosaveMinutes !== prev.autosaveMinutes) this.restartAutosave();
  }

  /** Installs a UI language: the `t()` table, the command titles and the native menu. */
  private async applyUiLanguage(lang: UiLanguage, notifyShell: boolean): Promise<void> {
    await installUiLanguage(lang);
    if (this.rebuild) this.ctl.registry.registerAll(this.rebuild());
    if (notifyShell && this.shell.setUiLanguage) await this.shell.setUiLanguage(lang).catch(() => undefined);
    this.store.bumpI18n();
  }

  /** Tools ▸ Options (⌘,). */
  async openOptions(tab: OptionsTab = "calculate"): Promise<void> {
    try {
      const calc = await this.client.calcSettings(this.doc).catch((): CalcSettingsDto => ({ iterations: null, dateEpoch: "d1899", caseSensitive: false, precisionAsShown: false, wholeCellMatch: true, criteriaMode: "wildcards", textToNumber: "convertUnambiguous" }));
      this.store.openDialog({ kind: "options", tab, calc, view: this.currentView(), lists: await this.customLists() });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async setCalcSettings(settings: CalcSettingsDto): Promise<boolean> {
    const res = await this.ctl.apply("Calculation settings", [{ type: "setCalcSettings", settings }]);
    return res !== null && res.errors.length === 0;
  }

  /** The AutoInput / AutoCorrect toggles of the Tools menu. */
  toggleAutoInput(): void {
    const p = this.store.prefs;
    void this.savePrefs({ ...p, autoInput: !p.autoInput });
    this.toast("info", p.autoInput ? "AutoInput is off." : "AutoInput is on.");
  }

  // -- editing assists: AutoCorrect, AutoInput, localized names ----------------------------

  /** The committed input: localized function names → canonical, then AutoCorrect. */
  commitText(text: string, original: string | null = null): string {
    const canonical = toCanonicalFormula(this.lang, text);
    return applyAutoCorrectToEdit(original === null ? null : toCanonicalFormula(this.lang, original), canonical, this.store.prefs.autoCorrect);
  }

  /** Canonical input → what the editor and formula bar show. */
  displayText(input: string): string {
    return toDisplayFormula(this.lang, input);
  }

  /** The editor text changed: ask the engine for a completion (debounced). */
  onEditChange(text: string): void {
    const edit = useEditStore.getState().edit;
    if (!this.store.prefs.autoInput || !edit || !wantsCompletion(text)) {
      if (this.store.ghost !== null) this.store.setGhost(null);
      return;
    }
    const seq = ++this.autoInputSeq;
    if (this.autoInputTimer) clearTimeout(this.autoInputTimer);
    const info = this.ctl.info;
    if (!info) return;
    const sheet = this.sheet;
    const cell = { ...edit.cell };
    this.autoInputTimer = setTimeout(() => {
      this.autoInputTimer = null;
      this.client.autocomplete(info.doc, sheet, cell.r, cell.c, text).then(
        (res) => {
          if (seq !== this.autoInputSeq) return;
          const cur = useEditStore.getState().edit;
          if (!cur || cur.text !== text) return;
          const ghost = res.completion && ghostSuffix(text, res.completion) !== "" ? res.completion : null;
          this.store.setGhost(ghost);
        },
        () => undefined,
      );
    }, AUTOINPUT_DEBOUNCE_MS);
  }

  /** Editor keys before the controller's own: Enter/Tab accept the AutoInput ghost (the commit then uses the full text). */
  editorKey(e: { key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): boolean {
    const st = useEditStore.getState();
    const ghost = this.store.ghost;
    if (!st.edit || !ghost) return false;
    if (e.ctrlKey && !e.metaKey && !e.altKey && e.key.toLowerCase() === "d") {
      void this.openSelectionList();
      return true;
    }
    if ((e.key === "Enter" || e.key === "Tab") && !e.metaKey && !e.ctrlKey && !e.altKey) {
      // Accept, then let the grid commit with the completed text.
      st.replace(ghost, ghost.length);
      this.store.setGhost(null);
      return false;
    }
    if (e.key === "ArrowRight" || e.key === "End") {
      st.replace(ghost, ghost.length);
      this.store.setGhost(null);
      return true;
    }
    if (e.key === "Escape") {
      this.store.setGhost(null);
      return true;
    }
    return false;
  }

  /** ⌃D: the selection list of the column's entries (while editing or on the active cell). */
  async openSelectionList(): Promise<void> {
    try {
      const edit = useEditStore.getState().edit;
      const cell = edit ? { ...edit.cell } : { ...this.ctl.selection.active };
      const res = await this.client.autocomplete(this.doc, this.sheet, cell.r, cell.c, "");
      if (res.items.length === 0) {
        this.toast("info", "No entries in this column yet.");
        return;
      }
      const rect = this.ctl.grid?.cellClientRect(cell) ?? { left: 200, top: 200, right: 300, bottom: 224 };
      this.store.openDialog({ kind: "selectionList", cell, items: res.items, rect: { left: rect.left, top: rect.top, bottom: rect.bottom } });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async pickFromList(cell: CellPos, item: string): Promise<void> {
    const st = useEditStore.getState();
    if (st.edit) {
      st.replace(item, item.length);
      this.store.setGhost(null);
      return;
    }
    await this.ctl.apply("Choose from list", [{ type: "setCellInput", sheet: this.sheet, r: cell.r, c: cell.c, input: item }]);
    this.ctl.focusGrid();
  }

  // -- Navigator, statistics -----------------------------------------------------------

  toggleNavigator(): void {
    this.store.togglePanel("navigator");
  }

  async navigator(): Promise<NavigatorDto> {
    return this.client.navigator(this.doc);
  }

  private refreshNavigatorPanel(): Promise<void> {
    useDocStore.setState((s) => ({ rev: s.rev + 1 }));
    return Promise.resolve();
  }

  /** Double-click in the Navigator: select the item. */
  goToItem(sheet: number | null, range: RangeDto | null): void {
    if (sheet !== null && sheet !== this.sheet) this.ctl.showSheet(sheet, false);
    if (range) {
      const sel: Selection = { range: { ...range }, active: { r: range.r0, c: range.c0 }, anchor: { r: range.r0, c: range.c0 } };
      useSelectionStore.getState().setSelection(sel);
      this.ctl.grid?.scrollCellIntoView(sel.active);
    }
    this.ctl.focusGrid();
  }

  async openDocumentStats(): Promise<void> {
    try {
      const stats = await this.client.documentStatistics(this.doc);
      this.store.openDialog({ kind: "documentStats", stats });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  // -- autosave, recovery, templates -----------------------------------------------------

  private restartAutosave(): void {
    if (this.autosaveTimer) clearInterval(this.autosaveTimer);
    this.autosaveTimer = null;
    const minutes = this.store.prefs.autosaveMinutes;
    if (minutes <= 0 || !this.shell.autosave) return;
    this.autosaveTimer = setInterval(() => void this.autosaveNow(), minutes * 60_000);
  }

  /** Writes a recovery snapshot when the document is dirty (also callable from tests). */
  async autosaveNow(): Promise<boolean> {
    const info = this.ctl.info;
    if (!info || !info.dirty || !this.shell.autosave) return false;
    try {
      await this.shell.autosave(info.doc);
      return true;
    } catch (e) {
      console.warn("tally: autosave failed", e);
      return false;
    }
  }

  private async offerRecovery(): Promise<void> {
    if (!this.shell.listRecovery) return;
    try {
      const files = await this.shell.listRecovery();
      if (files.length > 0) this.store.openDialog({ kind: "recovery", files });
    } catch {
      // No recovery directory: nothing to offer.
    }
  }

  async recover(file: RecoveryFile): Promise<void> {
    if (!this.shell.recoverSnapshot) return;
    try {
      this.ctl.load(await this.shell.recoverSnapshot(file.path));
      this.toast("info", `Recovered “${file.title}”. Save it to keep it.`);
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async discardRecovery(file: RecoveryFile): Promise<void> {
    try {
      await this.shell.discardRecovery?.(file.path);
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  openSaveAsTemplate(): void {
    if (!this.shell.saveAsTemplate) {
      this.toast("info", "Saving templates needs the desktop app.");
      return;
    }
    const title = (this.ctl.info?.title ?? "Untitled").replace(/\.(ods|xlsx|csv)$/i, "");
    this.store.openDialog({ kind: "saveAsTemplate", name: title });
  }

  async saveAsTemplate(name: string): Promise<boolean> {
    try {
      await this.shell.saveAsTemplate?.(this.doc, name);
      this.toast("info", `Saved the template “${name}”. It is listed in the sidebar.`);
      useDocStore.setState((s) => ({ rev: s.rev + 1 }));
      return true;
    } catch (e) {
      this.ctl.notifyError(e);
      return false;
    }
  }

  async listUserTemplates(): Promise<{ id: string; title: string; description: string }[]> {
    try {
      return (await this.shell.listUserTemplates?.()) ?? [];
    } catch {
      return [];
    }
  }

  async newFromUserTemplate(id: string): Promise<void> {
    try {
      const info = await this.shell.newFromUserTemplate?.(id);
      if (info) this.ctl.load(info);
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  // -- selection helpers for the extra shortcuts -------------------------------------------

  /** ⌘*: the data area around the cursor. */
  async selectDataArea(): Promise<void> {
    try {
      const region = await this.ctl.dataRegion();
      if (!region) return;
      this.ctl.grid?.setSelection({ range: region, active: { ...this.ctl.selection.active }, anchor: { r: region.r0, c: region.c0 } }, true);
      useSelectionStore.getState().setSelection({ range: region, active: { ...this.ctl.selection.active }, anchor: { r: region.r0, c: region.c0 } });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  /** ⌘/ (numpad): the array formula block around the cursor. */
  selectArrayFormula(): void {
    const a = useEditStore.getState().active;
    const block = a?.arrayRange;
    if (!block) {
      this.toast("info", "The active cell is not part of an array formula.");
      return;
    }
    const sel: Selection = { range: { ...block }, active: { ...this.ctl.selection.active }, anchor: { r: block.r0, c: block.c0 } };
    useSelectionStore.getState().setSelection(sel);
  }

  /** ⌥↩ outside the editor: the active cell's input into the whole selection. */
  async fillSelection(): Promise<void> {
    const sel = this.ctl.selection;
    const r = sel.range;
    if (r.r0 === r.r1 && r.c0 === r.c1) return;
    const a = sel.active;
    const input = (await this.client.cellEditInfo(this.doc, this.sheet, a.r, a.c).catch(() => null))?.input ?? "";
    const rows = Array.from({ length: Math.min(r.r1 - r.r0 + 1, 10_000) }, () => Array.from({ length: Math.min(r.c1 - r.c0 + 1, 1_000) }, () => input));
    await this.ctl.apply("Fill selection", [{ type: "setRangeInputs", sheet: this.sheet, r: r.r0, c: r.c0, rows }]);
  }

  /** ⌥ + arrows: the active row/column grows or shrinks; ⇧ fits it. */
  resizeByKey(axis: "row" | "col", delta: number, optimal: boolean): void {
    const a = this.ctl.selection.active;
    const i = axis === "row" ? a.r : a.c;
    if (optimal) {
      if (axis === "col") void this.ctl.autoFit("col", i, i);
      else void this.ctl.v3.fitRows();
      return;
    }
    const px = Math.max(4, Math.min(2000, this.ctl.layoutSize(axis, i) + delta));
    this.ctl.resize(axis, i, i, px);
  }

  /** ⌘⇧Y. */
  repeatLast(): void {
    const id = this.store.lastCommand;
    if (!id) {
      this.toast("info", "Nothing to repeat yet.");
      return;
    }
    this.ctl.registry.run(id);
  }

  focusNameBox(): void {
    const el = typeof document === "undefined" ? null : document.querySelector<HTMLInputElement>('[data-testid="name-box"]');
    el?.focus();
    el?.select();
  }

  /** The active cell's style id (for dialogs that need the resolved style synchronously). */
  activeStyleId(): number {
    const a = this.ctl.selection.active;
    const view = useDocStore.getState().source?.getCell(a.r, a.c);
    return drawStyleId(view);
  }

  /** The sheet id of a tab name (dialogs that take `Sheet.A1`). */
  sheetByName = (name: string): number | null => {
    const s = this.ctl.info?.sheets.find((x) => x.name.toLowerCase() === name.toLowerCase());
    return s ? s.id : null;
  };

  /** `A1` on the current sheet, or `Sheet.A1`, as a `SheetCell`. */
  parseCellRef(text: string): SheetCell | null {
    const m = /^(?:(?:'([^']+)'|([^.!':]+))[.!])?\$?([A-Za-z]{1,3})\$?(\d{1,7})$/.exec(text.trim());
    if (!m) return null;
    const name = m[1] ?? m[2] ?? null;
    const sheet = name === null ? this.sheet : this.sheetByName(name);
    if (sheet === null) return null;
    let c = 0;
    for (const ch of (m[3] ?? "").toUpperCase()) c = c * 26 + (ch.charCodeAt(0) - 64);
    const r = Number(m[4]) - 1;
    return r < 0 || r > LAST_ROW || c - 1 > LAST_COL ? null : { sheet, r, c: c - 1 };
  }

  /** Lets a dialog pick a range in the grid (the dialog hides meanwhile). */
  async pickRange(): Promise<RangeDto | null> {
    const grid = this.ctl.grid;
    if (!grid) return null;
    this.store.setPicking(true);
    try {
      grid.focus();
      return await grid.pickRange();
    } finally {
      this.store.setPicking(false);
    }
  }

  /** Fields of a pivot source (Edit layout from the Navigator). */
  pivotFields(source: PivotSource): Promise<PivotFieldItemsDto[]> {
    return this.client.pivotFieldItems(this.doc, source);
  }

  /** An empty style patch (dialogs). */
  emptyPatch(): StylePatch {
    return { ...EMPTY_PATCH };
  }

  /** Narrows an engine rejection to its code. */
  static errorCode(e: unknown): string | null {
    return isApiError(e) ? e.code : null;
  }

  /** Opens a dialog directly (tests, the Navigator). */
  open(d: W56Dialog): void {
    this.store.openDialog(d);
  }
}
