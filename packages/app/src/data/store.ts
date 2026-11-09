/**
 * Wave-5 (W5-6) UI state: the data-tool / style / options dialogs, the
 * right-side panels (Styles, Navigator), the active sheet's outline groups
 * and view options, the document's pivot tables, the AutoInput ghost, the
 * selection modes (F8 / ⇧F8) and the UI language revision. Never undoable.
 */
import type { MixedFields, TriProtection } from "../styles/formatCells";
import type {
  CalcSettingsDto,
  CellPos,
  CellStyleDto,
  DatabaseRangeDto,
  DocumentStatsDto,
  OutlineGroupDto,
  PivotDto,
  PivotFieldItemsDto,
  RangeDto,
  ScenarioDto,
  StyleDto,
  ViewOptionsDto,
} from "@tally/bridge";
import { create } from "zustand";
import type { PivotLayoutState } from "../pivot/pivotModel";
import type { AppPrefs } from "../options/prefs";
import { DEFAULT_PREFS } from "../options/prefs";

/** A W5-6 modal dialog. */
export type W56Dialog =
  | { kind: "dataPilot"; layout: PivotLayoutState; fields: PivotFieldItemsDto[]; existing: PivotDto | null }
  | { kind: "standardFilter"; range: RangeDto; labels: string[]; hasHeader: boolean; advanced: boolean }
  | { kind: "subtotals"; range: RangeDto; labels: string[] }
  | { kind: "consolidate" }
  | { kind: "textToColumns"; range: RangeDto; texts: string[] }
  | { kind: "removeDuplicates"; range: RangeDto; labels: string[]; hasHeader: boolean }
  | { kind: "scenarios"; list: ScenarioDto[] }
  | { kind: "defineDatabaseRange"; range: RangeDto; existing: DatabaseRangeDto[] }
  /** Data ▸ Select Range (AOO SID_SELECT_DB): the database ranges by name. */
  | { kind: "selectDatabaseRange"; ranges: DatabaseRangeDto[] }
  | { kind: "createNamesFromLabels"; range: RangeDto }
  | { kind: "fillSeries"; range: RangeDto }
  | { kind: "sortOptions"; range: RangeDto; labels: string[]; hasHeader: boolean; lists: string[][] }
  | { kind: "deleteContents"; ranges: RangeDto[] }
  | { kind: "multipleOperations"; range: RangeDto }
  | { kind: "insertCells"; range: RangeDto }
  | { kind: "deleteCells"; range: RangeDto }
  /** `sheets`: several selected sheets, copied as a block with engine names. */
  | { kind: "copySheet"; sheet: number; name: string; sheets?: number[] }
  | { kind: "showSheet"; hidden: { id: number; name: string }[] }
  | { kind: "tabColor"; sheet: number; color: number | null }
  | { kind: "showOutlineLevel"; axis: "rows" | "cols"; max: number }
  | { kind: "solver"; objective: string }
  | { kind: "mergePrompt"; range: RangeDto; hidden: number; onMove: () => void; onKeep: () => void }
  | { kind: "autoFormat"; range: RangeDto; select?: string }
  | { kind: "autoFormatName"; range: RangeDto; mode: "add" | "rename" | "delete"; name: string }
  | { kind: "formatCells"; style: StyleDto; ranges: RangeDto[]; mixed?: MixedFields; sample?: string; sampleNum?: number | null; protection?: TriProtection }
  | { kind: "customizeToolbars" }
  /** View ▸ Zoom (AOO Zoom & View Layout): `optimal` is offered for a marked range. */
  | { kind: "zoom"; optimal: boolean }
  | { kind: "styleEditor"; style: CellStyleDto; isNew: boolean; fromSelection: StyleDto | null }
  | { kind: "options"; tab: OptionsTab; calc: CalcSettingsDto; view: ViewOptionsDto; lists: string[][] }
  | { kind: "documentStats"; stats: DocumentStatsDto }
  | { kind: "recovery"; files: RecoveryFile[] }
  | { kind: "saveAsTemplate"; name: string }
  | { kind: "selectionList"; cell: CellPos; items: string[]; rect: { left: number; top: number; bottom: number } };

export type OptionsTab = "calculate" | "view" | "input" | "formula" | "sortLists" | "loadSave" | "language";

/** A crash-recovery snapshot offered on launch (`list_recovery`). */
export interface RecoveryFile {
  path: string;
  title: string;
  savedAt: string;
}

export type W56Panel = "styles" | "navigator" | "functions";

export interface OutlineState {
  rows: OutlineGroupDto[];
  cols: OutlineGroupDto[];
}

export interface W56State {
  dialog: W56Dialog | null;
  /** The dialog is hidden while the user picks a range in the grid. */
  picking: boolean;
  panel: W56Panel | null;
  /** The active sheet's outline groups. */
  outline: OutlineState;
  /** The active sheet's view options. */
  view: ViewOptionsDto;
  /** Every pivot table of the document. */
  pivots: PivotDto[];
  /** The pivot the active cell is in (`pivots` id), or `null`. */
  activePivot: number | null;
  /** Named cell styles (Styles panel). */
  cellStyles: CellStyleDto[];
  /** Paint-can mode: the style applied to clicked cells. */
  paintStyle: string | null;
  /** AutoInput: the completion offered for the current edit (full text). */
  ghost: string | null;
  /** F8 extend-selection mode / ⇧F8 add-selection mode. */
  extendMode: boolean;
  addMode: boolean;
  /** App preferences (prefs.json). */
  prefs: AppPrefs;
  /** Bumped when the UI language changes (components re-read `t()`). */
  i18nRev: number;
  /** The last repeatable command id (⌘⇧Y). */
  lastCommand: string | null;
  openDialog(d: W56Dialog | null): void;
  setPicking(on: boolean): void;
  setPanel(p: W56Panel | null): void;
  togglePanel(p: W56Panel): void;
  setOutline(o: OutlineState): void;
  setView(v: ViewOptionsDto): void;
  setPivots(p: PivotDto[]): void;
  setActivePivot(id: number | null): void;
  setCellStyles(s: CellStyleDto[]): void;
  setPaintStyle(name: string | null): void;
  setGhost(g: string | null): void;
  setExtendMode(on: boolean): void;
  setAddMode(on: boolean): void;
  setPrefs(p: AppPrefs): void;
  bumpI18n(): void;
  setLastCommand(id: string | null): void;
  reset(): void;
}

export const DEFAULT_VIEW: ViewOptionsDto = { showFormulas: false, valueHighlighting: false, showZeroValues: true, showGridlines: true, showHeaders: true };
const EMPTY_OUTLINE: OutlineState = { rows: [], cols: [] };

export const useW56Store = create<W56State>()((set, get) => ({
  dialog: null,
  picking: false,
  panel: null,
  outline: EMPTY_OUTLINE,
  view: DEFAULT_VIEW,
  pivots: [],
  activePivot: null,
  cellStyles: [],
  paintStyle: null,
  ghost: null,
  extendMode: false,
  addMode: false,
  prefs: DEFAULT_PREFS,
  i18nRev: 0,
  lastCommand: null,
  openDialog: (dialog) => set({ dialog, picking: false }),
  setPicking: (picking) => set({ picking }),
  setPanel: (panel) => set({ panel }),
  togglePanel: (p) => set({ panel: get().panel === p ? null : p }),
  setOutline: (outline) => set({ outline }),
  setView: (view) => set({ view }),
  setPivots: (pivots) => set({ pivots }),
  setActivePivot: (activePivot) => set({ activePivot }),
  setCellStyles: (cellStyles) => set({ cellStyles }),
  setPaintStyle: (paintStyle) => set({ paintStyle }),
  setGhost: (ghost) => set({ ghost }),
  setExtendMode: (extendMode) => set({ extendMode, addMode: extendMode ? false : get().addMode }),
  setAddMode: (addMode) => set({ addMode, extendMode: addMode ? false : get().extendMode }),
  setPrefs: (prefs) => set({ prefs }),
  bumpI18n: () => set((s) => ({ i18nRev: s.i18nRev + 1 })),
  setLastCommand: (lastCommand) => set({ lastCommand }),
  reset: () => set({ dialog: null, picking: false, outline: EMPTY_OUTLINE, view: DEFAULT_VIEW, pivots: [], activePivot: null, paintStyle: null, ghost: null, extendMode: false, addMode: false }),
}));
