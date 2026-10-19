/**
 * The EditSession shared by the in-cell editor and the formula bar, plus
 * the active cell's committed input and hint (`cellEditInfo`).
 *
 * The session also keeps an editor-local undo history (⌘Z while editing
 * undoes typing, not the document), the span of the reference inserted by
 * point mode (so the next pick replaces it) and the autocomplete state.
 */
import type { CellPos, RangeDto } from "@tally/bridge";
import type { EditState } from "@tally/grid";
import { create } from "zustand";

/** Where the edit started (the formula bar keeps focus for "bar"). */
export type EditOrigin = "grid" | "bar";

export interface ActiveInfo {
  sheet: number;
  cell: CellPos;
  input: string;
  hint: string | null;
  /** The array-formula block containing the cell (its input is `{=…}`). */
  arrayRange?: RangeDto | null;
}

interface Snapshot {
  text: string;
  caret: number;
}

/** Undo depth of the editor-local history. */
export const EDIT_HISTORY = 200;

export interface EditStoreState {
  edit: EditState | null;
  origin: EditOrigin | null;
  /** `cellEditInfo` of the active cell (may lag the selection briefly). */
  active: ActiveInfo | null;
  /** Editor-local history: past states (oldest first) and redo states. */
  past: Snapshot[];
  future: Snapshot[];
  /** The reference last inserted by point mode (replaced by the next pick). */
  pointSpan: { start: number; end: number } | null;
  /** The last history step was plain typing (the next keystroke joins it). */
  typing: boolean;
  /** Highlighted row of the function autocomplete list. */
  assistIndex: number;
  /** Autocomplete hidden for this identifier (Esc) until the text changes. */
  assistDismissed: string | null;
  start(cell: CellPos, text: string, origin: EditOrigin): void;
  /** Typing: records history (coalescing single-character steps). */
  change(text: string, caret?: number): void;
  /** A programmatic change (point mode, autocomplete): its own history step. */
  replace(text: string, caret: number, pointSpan?: { start: number; end: number } | null): void;
  undo(): boolean;
  redo(): boolean;
  setOrigin(origin: EditOrigin): void;
  setAssist(index: number): void;
  dismissAssist(key: string | null): void;
  end(): void;
  setActive(info: ActiveInfo | null): void;
  reset(): void;
}

const blank = { past: [] as Snapshot[], future: [] as Snapshot[], pointSpan: null, typing: false, assistIndex: 0, assistDismissed: null };

/** Whether `b` is one typed character more or less than `a` (coalesced in history). */
function isSmallStep(a: string, b: string): boolean {
  if (Math.abs(a.length - b.length) !== 1) return false;
  const [s, l] = a.length < b.length ? [a, b] : [b, a];
  let i = 0;
  while (i < s.length && s[i] === l[i]) i++;
  const ch = l[i] ?? "";
  // Word boundaries start a new step.
  return s.slice(i) === l.slice(i + 1) && !/[\s=+\-*/^&(),;:<>]/.test(ch);
}

export const useEditStore = create<EditStoreState>()((set, get) => ({
  edit: null,
  origin: null,
  active: null,
  ...blank,
  start: (cell, text, origin) => set({ edit: { cell: { ...cell }, text, caret: text.length }, origin, ...blank }),
  change: (text, caret) =>
    set((s) => {
      if (!s.edit) return {};
      if (text === s.edit.text) return { edit: { ...s.edit, caret: caret ?? s.edit.caret } };
      const prev = { text: s.edit.text, caret: s.edit.caret };
      const small = isSmallStep(prev.text, text);
      const coalesce = small && s.typing && s.future.length === 0;
      const past = coalesce ? s.past : [...s.past, prev].slice(-EDIT_HISTORY);
      return {
        edit: { ...s.edit, text, caret: caret ?? text.length },
        past,
        typing: small,
        future: [],
        pointSpan: null,
        assistDismissed: null,
        assistIndex: 0,
      };
    }),
  replace: (text, caret, pointSpan = null) =>
    set((s) => {
      if (!s.edit) return {};
      return {
        edit: { ...s.edit, text, caret, seq: (s.edit.seq ?? 0) + 1 },
        past: [...s.past, { text: s.edit.text, caret: s.edit.caret }].slice(-EDIT_HISTORY),
        future: [],
        pointSpan,
        typing: false,
        assistIndex: 0,
      };
    }),
  undo: () => {
    const s = get();
    const prev = s.past[s.past.length - 1];
    if (!s.edit || !prev) return false;
    set({
      edit: { ...s.edit, text: prev.text, caret: prev.caret, seq: (s.edit.seq ?? 0) + 1 },
      past: s.past.slice(0, -1),
      future: [...s.future, { text: s.edit.text, caret: s.edit.caret }],
      pointSpan: null,
      typing: false,
    });
    return true;
  },
  redo: () => {
    const s = get();
    const next = s.future[s.future.length - 1];
    if (!s.edit || !next) return false;
    set({
      edit: { ...s.edit, text: next.text, caret: next.caret, seq: (s.edit.seq ?? 0) + 1 },
      future: s.future.slice(0, -1),
      past: [...s.past, { text: s.edit.text, caret: s.edit.caret }],
      pointSpan: null,
      typing: false,
    });
    return true;
  },
  setOrigin: (origin) => set({ origin }),
  setAssist: (assistIndex) => set({ assistIndex }),
  dismissAssist: (assistDismissed) => set({ assistDismissed }),
  end: () => set({ edit: null, origin: null, ...blank }),
  setActive: (active) => set({ active }),
  reset: () => set({ edit: null, origin: null, active: null, ...blank }),
}));
