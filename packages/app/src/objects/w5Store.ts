/**
 * View state of the W5-5 features that is not an object: the split window,
 * Page Break Preview (mode + breaks from `page_breaks`), detective arrows
 * and "Mark invalid data" circles. Never undoable (the engine keeps the
 * persisted parts: `setSplit`, `setPageBreak`).
 */
import type { CellPos, PageBreaksDto } from "@tally/bridge";
import type { PageBreaks, SplitAxis, TraceArrow } from "@tally/grid";
import { create } from "zustand";

export interface SplitState {
  axis: SplitAxis;
  /** Share of the first pane, 0.1–0.9. */
  ratio: number;
}

export interface W5State {
  split: SplitState | null;
  /** View ▸ Page Break Preview. */
  pageBreakPreview: boolean;
  /** Breaks from `page_breaks` (null: unsupported → the v0.3 estimate is drawn instead). */
  pageBreaks: PageBreaksDto | null;
  traces: TraceArrow[];
  invalidCells: CellPos[];
  setSplit(split: SplitState | null): void;
  setPageBreakPreview(on: boolean): void;
  setPageBreaks(b: PageBreaksDto | null): void;
  setTraces(t: TraceArrow[]): void;
  addTraces(t: TraceArrow[]): void;
  setInvalidCells(cells: CellPos[]): void;
  /** Sheet change: traces, circles and breaks are per sheet. */
  resetSheet(): void;
}

export const useW5Store = create<W5State>()((set) => ({
  split: null,
  pageBreakPreview: false,
  pageBreaks: null,
  traces: [],
  invalidCells: [],
  setSplit: (split) => set({ split }),
  setPageBreakPreview: (pageBreakPreview) => set({ pageBreakPreview }),
  setPageBreaks: (pageBreaks) => set({ pageBreaks }),
  setTraces: (traces) => set({ traces }),
  addTraces: (t) => set((s) => ({ traces: dedupe([...s.traces, ...t]) })),
  setInvalidCells: (invalidCells) => set({ invalidCells }),
  resetSheet: () => set({ traces: [], invalidCells: [], pageBreaks: null, split: null }),
}));

function dedupe(traces: TraceArrow[]): TraceArrow[] {
  const seen = new Set<string>();
  return traces.filter((t) => {
    const k = `${t.from.r},${t.from.c}>${t.to.r},${t.to.c}:${t.kind ?? ""}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}

/** Grid page-break lines from a `PageBreaksDto`. */
export function toGridBreaks(b: PageBreaksDto): PageBreaks {
  return { rows: b.rowBreaks.map((x) => ({ at: x.at, manual: x.manual })), cols: b.colBreaks.map((x) => ({ at: x.at, manual: x.manual })) };
}
