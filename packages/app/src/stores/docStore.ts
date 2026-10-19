/** Workbook info, sheets, active sheet, version, dirty, undo labels. */
import type { ApplyResult, UndoState, WorkbookInfo } from "@tally/bridge";
import type { EngineClientSource } from "@tally/grid";
import { create } from "zustand";

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
  setSource(source: EngineClientSource | null): void;
  setInfo(info: WorkbookInfo): void;
  applyResult(result: ApplyResult): void;
  setActiveSheet(sheet: number): void;
  reset(): void;
}

const initial = { info: null, activeSheet: 0, undo: NO_UNDO, rev: 0, source: null };

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
    set((s) => ({
      info,
      activeSheet:
        s.info?.doc === info.doc && info.sheets.some((sh) => sh.id === s.activeSheet) ? s.activeSheet : info.activeSheet,
      undo: s.info?.doc === info.doc ? s.undo : NO_UNDO,
      rev: s.rev + 1,
    })),
  applyResult: (result) => set((s) => ({ ...reduceResult(s, result), rev: s.rev + 1 })),
  setSource: (source) => set({ source }),
  setActiveSheet: (sheet) => set({ activeSheet: sheet }),
  reset: () => set(initial),
}));
