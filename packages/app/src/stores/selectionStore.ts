/** The grid selection (single range in Phase A). */
import type { SelectionDto } from "@tally/bridge";
import { cellSelection, type Selection } from "@tally/grid";
import { create } from "zustand";

export interface SelectionState {
  selection: Selection;
  setSelection(sel: Selection): void;
  reset(): void;
}

export const useSelectionStore = create<SelectionState>()((set) => ({
  selection: cellSelection({ r: 0, c: 0 }),
  setSelection: (selection) => set({ selection }),
  reset: () => set({ selection: cellSelection({ r: 0, c: 0 }) }),
}));

/** Converts the grid selection to the engine DTO. */
export function toSelectionDto(sel: Selection, sheet: number): SelectionDto {
  return { sheet, ranges: [{ ...sel.range }], active: { ...sel.active } };
}

/** Converts an engine selection (e.g. from undo or the name box) to the grid's. */
export function fromSelectionDto(dto: SelectionDto): Selection {
  const range = dto.ranges[0] ?? { r0: dto.active.r, c0: dto.active.c, r1: dto.active.r, c1: dto.active.c };
  return { range: { ...range }, active: { ...dto.active }, anchor: { ...dto.active } };
}
