/** Workbook info, sheets, active sheet, version, dirty, undo labels. */
import type { ApplyResult, UndoState, WorkbookInfo } from "@tally/bridge";
import type { EngineClientSource } from "@tally/grid";
import { create } from "zustand";
import { normalizeGroup, rangeBetween, toggle, visibleIds } from "../sheetGroup";

export const NO_UNDO: UndoState = { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null };

export interface DocState {
  info: WorkbookInfo | null;
  /** The sheet shown (UI state; follows the engine when it changes the active sheet). */
  activeSheet: number;
  undo: UndoState;
  /** Bumped on every engine result (status bar and formula bar refresh on it). */
  rev: number;
  /** The grid's data source for the current document. */
  source: EngineClientSource | null;
  /**
   * Multi-sheet selection (AOO "selected sheets"): the selected visible
   * sheets in tab order, the active one included; `[]` when only the active
   * sheet is selected. Pruned when sheets are deleted or hidden.
   */
  groupSheets: number[];
  /** Sets the group (cleaned: visible sheets in tab order, with the active one). */
  setGroup(ids: readonly number[]): void;
  /** Only `sheet` selected (the caller makes it active). */
  selectOnly(sheet: number): void;
  /** ⌘/Ctrl-click: a sheet in or out of the group (never the active one). */
  toggleSheet(sheet: number): void;
  /** Shift-click: the sheets from the active one to `sheet`. */
  extendTo(sheet: number): void;
  /** Every visible sheet. */
  selectAllSheets(): void;
  /** Back to the active sheet alone. */
  deselectSheets(): void;
  setSource(source: EngineClientSource | null): void;
  setInfo(info: WorkbookInfo): void;
  applyResult(result: ApplyResult): void;
  setActiveSheet(sheet: number): void;
  reset(): void;
}

const initial = { info: null, activeSheet: 0, undo: NO_UNDO, rev: 0, source: null, groupSheets: [] as number[] };

/**
 * The group cleaned against the current sheets and active sheet (same
 * array when unchanged); an active sheet outside it collapses it.
 */
function pruned(group: number[], info: WorkbookInfo | null, active: number): number[] {
  if (group.length === 0) return group;
  if (!group.includes(active)) return [];
  const next = info ? normalizeGroup(info.sheets, group, active) : [];
  return next.length === group.length && next.every((id, i) => id === group[i]) ? group : next;
}

/** Pure reducer behind {@link DocState.applyResult} (exported for tests). */
export function reduceResult(
  s: Pick<DocState, "info" | "activeSheet" | "undo">,
  result: ApplyResult,
): Pick<DocState, "info" | "activeSheet" | "undo"> {
  const prev = s.info;
  if (result.info) {
    const info = result.info;
    let active = s.activeSheet;
    const exists = info.sheets.some((sh) => sh.id === active);
    if (!exists || (prev !== null && prev.activeSheet !== info.activeSheet)) active = info.activeSheet;
    if (!info.sheets.some((sh) => sh.id === active)) active = info.sheets[0]?.id ?? active;
    return { info, activeSheet: active, undo: result.undo };
  }
  const changed = result.changed.length > 0 || result.layoutChanged.length > 0 || result.sheetsChanged;
  const info = prev && changed ? { ...prev, dirty: true, version: result.version } : prev;
  return { info, activeSheet: s.activeSheet, undo: result.undo };
}

export const useDocStore = create<DocState>()((set) => ({
  ...initial,
  setInfo: (info) =>
    set((s) => {
      const same = s.info?.doc === info.doc;
      const activeSheet = same && info.sheets.some((sh) => sh.id === s.activeSheet) ? s.activeSheet : info.activeSheet;
      return {
        info,
        activeSheet,
        undo: same ? s.undo : NO_UNDO,
        groupSheets: same ? pruned(s.groupSheets, info, activeSheet) : [],
        rev: s.rev + 1,
      };
    }),
  applyResult: (result) =>
    set((s) => {
      const next = reduceResult(s, result);
      return { ...next, groupSheets: pruned(s.groupSheets, next.info, next.activeSheet), rev: s.rev + 1 };
    }),
  setSource: (source) => set({ source }),
  setActiveSheet: (sheet) => set((s) => ({ activeSheet: sheet, groupSheets: s.groupSheets.includes(sheet) ? s.groupSheets : [] })),
  setGroup: (ids) => set((s) => ({ groupSheets: s.info ? normalizeGroup(s.info.sheets, ids, s.activeSheet) : [] })),
  selectOnly: () => set({ groupSheets: [] }),
  toggleSheet: (sheet) => set((s) => ({ groupSheets: s.info ? toggle(s.info.sheets, s.groupSheets, s.activeSheet, sheet) : [] })),
  extendTo: (sheet) => set((s) => ({ groupSheets: s.info ? rangeBetween(s.info.sheets, s.activeSheet, sheet) : [] })),
  selectAllSheets: () => set((s) => ({ groupSheets: s.info ? normalizeGroup(s.info.sheets, visibleIds(s.info.sheets), s.activeSheet) : [] })),
  deselectSheets: () => set({ groupSheets: [] }),
  reset: () => set(initial),
}));

/** Whether more than one sheet is selected. */
export function isGrouped(): boolean {
  return useDocStore.getState().groupSheets.length > 1;
}
