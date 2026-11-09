/**
 * The "Last Used" function list shared by the Function Wizard and the
 * Function List panel (AOO keeps one LRU list, `ScAppOptions::GetLRUFuncList`):
 * AOO's default five, later inserts move to the front, ten at most
 * (`LRU_MAX`). Session only.
 */
import { create } from "zustand";

/** AOO's default "Last Used" list. */
export const DEFAULT_LAST_USED: readonly string[] = ["SUM", "AVERAGE", "MIN", "MAX", "IF"];
/** AOO's `LRU_MAX`. */
export const LAST_USED_MAX = 10;

export const useLastUsedStore = create<{ names: string[] }>()(() => ({ names: [...DEFAULT_LAST_USED] }));

/** The "Last Used" names (canonical), most recent first. */
export function lastUsedFunctions(): readonly string[] {
  return useLastUsedStore.getState().names;
}

/** Records an insert (moves `name` to the front, keeps ten). */
export function noteFunctionUsed(name: string): void {
  const names = useLastUsedStore.getState().names;
  useLastUsedStore.setState({ names: [name, ...names.filter((n) => n !== name)].slice(0, LAST_USED_MAX) });
}

/** Resets the "Last Used" list (tests). */
export function resetLastUsed(): void {
  useLastUsedStore.setState({ names: [...DEFAULT_LAST_USED] });
}
