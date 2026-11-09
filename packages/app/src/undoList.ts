/**
 * The toolbar Undo / Redo history lists (AOO's `SvxUndoRedoControl`,
 * `svx/source/tbxctrls/lboxctrl.cxx`): the steps, next one first; pointing
 * at or arrowing to entry n marks entries 1..n, the footer counts them
 * ("Actions to undo: 3", `RID_SVXSTR_NUM_UNDO_ACTIONS`) and choosing it
 * undoes or redoes n steps as one user action.
 */
import { t } from "./i18n";

export type UndoListKind = "undo" | "redo";

/** What a key does in the list: a new count, pick the marked count, close, or nothing. */
export type UndoListKey = { count: number } | "pick" | "close" | null;

/** Rows a Page Up / Page Down moves. */
export const UNDO_LIST_PAGE = 10;

/** The list's response to a key, with `count` (1..len) entries marked. */
export function undoListKey(key: string, count: number, len: number): UndoListKey {
  const clamp = (n: number) => ({ count: Math.max(1, Math.min(len, n)) });
  switch (key) {
    case "ArrowDown":
      return clamp(count + 1);
    case "ArrowUp":
      return clamp(count - 1);
    case "PageDown":
      return clamp(count + UNDO_LIST_PAGE);
    case "PageUp":
      return clamp(count - UNDO_LIST_PAGE);
    case "Home":
      return clamp(1);
    case "End":
      return clamp(len);
    case "Enter":
    case " ":
      return "pick";
    case "Tab":
      return "close";
    default:
      return null;
  }
}

/** The footer: "Actions to undo: n" / "Actions to redo: n" (AOO uses one form for 1 and more). */
export function undoListFooter(kind: UndoListKind, count: number): string {
  return kind === "undo" ? t("undoList.undoCount", "Actions to undo: {n}", { n: String(count) }) : t("undoList.redoCount", "Actions to redo: {n}", { n: String(count) });
}

/** A step's label as listed (a step without one gets a placeholder). */
export function undoListLabel(label: string): string {
  return label.trim() === "" ? t("undoList.unnamed", "(unnamed action)") : label;
}

/** The list's accessible name. */
export function undoListTitle(kind: UndoListKind): string {
  return kind === "undo" ? t("undoList.undoTitle", "Undo history") : t("undoList.redoTitle", "Redo history");
}
