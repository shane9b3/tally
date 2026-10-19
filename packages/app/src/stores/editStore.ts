/**
 * The EditSession shared by the in-cell editor and the formula bar, plus
 * the active cell's committed input and hint (`cellEditInfo`).
 */
import type { CellPos } from "@tally/bridge";
import type { EditState } from "@tally/grid";
import { create } from "zustand";

/** Where the edit started (the formula bar keeps focus for "bar"). */
export type EditOrigin = "grid" | "bar";

export interface ActiveInfo {
  sheet: number;
  cell: CellPos;
  input: string;
  hint: string | null;
}

export interface EditStoreState {
  edit: EditState | null;
  origin: EditOrigin | null;
  /** `cellEditInfo` of the active cell (may lag the selection briefly). */
  active: ActiveInfo | null;
  start(cell: CellPos, text: string, origin: EditOrigin): void;
  change(text: string, caret?: number): void;
  end(): void;
  setActive(info: ActiveInfo | null): void;
  reset(): void;
}

export const useEditStore = create<EditStoreState>()((set) => ({
  edit: null,
  origin: null,
  active: null,
  start: (cell, text, origin) => set({ edit: { cell: { ...cell }, text, caret: text.length }, origin }),
  change: (text, caret) => set((s) => (s.edit ? { edit: { ...s.edit, text, caret: caret ?? text.length } } : {})),
  end: () => set({ edit: null, origin: null }),
  setActive: (active) => set({ active }),
  reset: () => set({ edit: null, origin: null, active: null }),
}));
