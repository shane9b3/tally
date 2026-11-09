/**
 * Function List panel model (AOO `ScFunctionDockWin`, dwfunctr.cxx): the
 * category list, the functions of a category, the description text and the
 * text Insert writes. Pure, so the panel and the unit tests share it.
 */
import type { FunctionInfo } from "@tally/bridge";

/** The "Last Used" entry of the category box (first, `aCatBox` position 0). */
export const LAST_USED = "__last";
/** The "All" entry (position 1). */
export const ALL = "__all";

/**
 * AOO's category order (`RID_FUNCTION_CATEGORIES`, sc/source/ui/src/scfuncs.src):
 * Database, Date&Time, Financial, Information, Logical, Mathematical, Array,
 * Statistics, Spreadsheet, Text, Add-in. Engine categories AOO does not list
 * follow alphabetically.
 */
export const AOO_CATEGORY_ORDER: readonly string[] = [
  "Database",
  "Date&Time",
  "Financial",
  "Information",
  "Logical",
  "Mathematical",
  "Array",
  "Statistical",
  "Spreadsheet",
  "Text",
  "Add-in",
];

/** Last Used, All, then the engine's categories in AOO's order. */
export function listCategories(functions: readonly FunctionInfo[]): string[] {
  const present = new Set(functions.map((f) => f.category));
  const known = AOO_CATEGORY_ORDER.filter((c) => present.has(c));
  const extra = [...present].filter((c) => !AOO_CATEGORY_ORDER.includes(c)).sort((a, b) => a.localeCompare(b));
  return [LAST_USED, ALL, ...known, ...extra];
}

/**
 * The functions of `category`. "Last Used" follows `recent` (canonical names,
 * most recent first) and resolves each to its entry in `functions`, whose
 * `name` may be the localized spelling: `canonical` maps a recent name to the
 * entry's name. Other categories keep the list's own (name) order.
 */
export function functionsOf(functions: readonly FunctionInfo[], category: string, recent: readonly string[], localize: (canonical: string) => string = (n) => n): FunctionInfo[] {
  if (category === LAST_USED) {
    const byName = new Map(functions.map((f) => [f.name, f]));
    return recent.map((n) => byName.get(localize(n))).filter((f): f is FunctionInfo => f !== undefined);
  }
  if (category === ALL) return [...functions];
  return functions.filter((f) => f.category === category);
}

/** `, ` between the arguments of an engine signature -> the UI's argument separator (`; ` or `, `). */
export function signatureWithSeparator(signature: string, sep: ";" | ","): string {
  return signature.replace(/;\s|,\s/g, sep === ";" ? "; " : ", ");
}

/** What Insert writes: `NAME()` at the caret, the caret between the parentheses. */
export function insertFunctionText(text: string, caret: number, name: string): { text: string; caret: number } {
  if (text === "") return { text: `=${name}()`, caret: name.length + 2 };
  const at = Math.max(0, Math.min(caret, text.length));
  const ins = `${name}()`;
  return { text: text.slice(0, at) + ins + text.slice(at), caret: at + name.length + 1 };
}

/** Type-ahead: the index of the first entry at or after `from` whose name starts with `buffer` (wrapping), or -1. */
export function typeAheadIndex(names: readonly string[], buffer: string, from: number): number {
  const q = buffer.toUpperCase();
  if (q === "") return -1;
  for (let k = 0; k < names.length; k++) {
    const i = (from + k) % names.length;
    if ((names[i] ?? "").toUpperCase().startsWith(q)) return i;
  }
  return -1;
}
