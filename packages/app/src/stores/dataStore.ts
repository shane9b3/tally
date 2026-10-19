/**
 * Sheet data the chrome shows besides cells: the active sheet's AutoFilter,
 * the defined names, the copied/cut range (dashed outline) and find
 * results. Refreshed by the controller from engine queries.
 */
import type { AutoFilterInfo, FindHit, NameInfo, RangeDto } from "@tally/bridge";
import { create } from "zustand";
import { DEFAULT_FIND, type FindOptions } from "../dataOps";

/** The last copy or cut: its token and source, for the dashed outline and cut → move. */
export interface ClipState {
  sheet: number;
  range: RangeDto;
  token: string;
  cut: boolean;
}

export interface FindState {
  text: string;
  options: FindOptions;
  hits: FindHit[];
  /** Index of the current hit, or −1. */
  index: number;
  truncated: boolean;
  /** The query that produced `hits` (to know when to search again). */
  searched: string | null;
}

export interface DataState {
  /** AutoFilter of the active sheet (`range: null` without one). */
  filter: AutoFilterInfo | null;
  names: NameInfo[];
  clip: ClipState | null;
  find: FindState;
  setFilter(f: AutoFilterInfo | null): void;
  setNames(n: NameInfo[]): void;
  setClip(c: ClipState | null): void;
  patchFind(p: Partial<FindState>): void;
  reset(): void;
}

export const INITIAL_FIND: FindState = { text: "", options: DEFAULT_FIND, hits: [], index: -1, truncated: false, searched: null };

export const useDataStore = create<DataState>()((set) => ({
  filter: null,
  names: [],
  clip: null,
  find: INITIAL_FIND,
  setFilter: (filter) => set({ filter }),
  setNames: (names) => set({ names }),
  setClip: (clip) => set({ clip }),
  patchFind: (p) => set((s) => ({ find: { ...s.find, ...p } })),
  reset: () => set({ filter: null, names: [], clip: null, find: INITIAL_FIND }),
}));
