/** The grid selection (a current range plus earlier ranges of a multi-selection). */
import type { RangeDto, SelectionDto } from "@tally/bridge";
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

/** Every range of the selection, oldest first (the current range last). */
export function selectionRanges(sel: Selection): RangeDto[] {
  return [...(sel.others ?? []), sel.range].map((r) => ({ ...r }));
}

/** Converts the grid selection to the engine DTO (all ranges; the active cell is in the last). */
export function toSelectionDto(sel: Selection, sheet: number): SelectionDto {
  return { sheet, ranges: selectionRanges(sel), active: { ...sel.active } };
}

/** Converts an engine selection (e.g. from undo or the name box) to the grid's. */
export function fromSelectionDto(dto: SelectionDto): Selection {
  const inside = (r: RangeDto): boolean =>
    dto.active.r >= r.r0 && dto.active.r <= r.r1 && dto.active.c >= r.c0 && dto.active.c <= r.c1;
  const ranges = dto.ranges.length > 0 ? dto.ranges : [{ r0: dto.active.r, c0: dto.active.c, r1: dto.active.r, c1: dto.active.c }];
  // The current range is the last one holding the active cell.
  let idx = ranges.length - 1;
  for (let i = ranges.length - 1; i >= 0; i--) {
    const r = ranges[i];
    if (r && inside(r)) {
      idx = i;
      break;
    }
  }
  const range = ranges[idx] ?? ranges[0]!;
  const others = ranges.filter((_, i) => i !== idx).map((r) => ({ ...r }));
  const sel: Selection = { range: { ...range }, active: { ...dto.active }, anchor: { ...dto.active } };
  if (others.length > 0) sel.others = others;
  return sel;
}
