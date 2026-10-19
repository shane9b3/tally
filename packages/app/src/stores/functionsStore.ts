/** The engine's function list (`listFunctions`), loaded once per app start. */
import type { FunctionInfo } from "@tally/bridge";
import { create } from "zustand";

export interface FunctionsState {
  functions: FunctionInfo[];
  set(functions: FunctionInfo[]): void;
}

export const useFunctionsStore = create<FunctionsState>()((set) => ({
  functions: [],
  set: (functions) => set({ functions: [...functions].sort((a, b) => a.name.localeCompare(b.name)) }),
}));
