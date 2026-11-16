/**
 * The status bar's selection function, as in AOO 4.1: one function at a
 * time, shown as `Sum=11`, chosen from a context menu and kept as an app
 * setting (`ScAppOptions::GetStatusFunc`, default Sum; menu
 * `RID_SVXMNU_PSZ_FUNC` in svx `stbctrls.src`; text built by
 * `ScTabViewShell::GetFunction`).
 */
import type { SelectionStats } from "@tally/bridge";
import { t } from "./i18n";
import type { StatusFunction } from "./options/prefs";

export { STATUS_FUNCTIONS, type StatusFunction } from "./options/prefs";

/** The menu entry of a function (AOO: Average, CountA, Count, Maximum, Minimum, Sum, None). */
export function statusFunctionMenuLabel(fn: StatusFunction): string {
  switch (fn) {
    case "average":
      return t("status.average", "Average");
    case "countA":
      return t("status.countA", "CountA");
    case "count":
      return t("status.count", "Count");
    case "max":
      return t("status.max", "Maximum");
    case "min":
      return t("status.min", "Minimum");
    case "sum":
      return t("status.sum", "Sum");
    case "none":
      return t("status.none", "None");
  }
}

/** The name in front of the `=` (AOO `STR_FUN_TEXT_*`: Max and Min are short there). */
function shortName(fn: Exclude<StatusFunction, "none">): string {
  switch (fn) {
    case "max":
      return t("status.maxShort", "Max");
    case "min":
      return t("status.minShort", "Min");
    default:
      return statusFunctionMenuLabel(fn);
  }
}

/**
 * The status bar text of `fn` for `stats`: `Sum=11`, `CountA=3`, or `""`
 * for None.
 *
 * - Count (numbers) and CountA (non-empty cells) always have a value.
 * - Sum of no numbers is 0; Average, Max and Min of no numbers show the
 *   name and `=` only (AOO: `GetSelectionFunction` fails, the value stays
 *   empty).
 * - A result of exactly 0 is written `0` whatever the cell's format
 *   (AOO: `if ( nVal == 0.0 ) aStr += '0'`); other results come formatted
 *   like the active cell.
 * - With an error in the selection Sum, Average, Max and Min show the
 *   error's text (`stats.formattedSum`).
 */
export function statusFunctionText(fn: StatusFunction, stats: SelectionStats | null): string {
  if (fn === "none") return "";
  const head = `${shortName(fn)}=`;
  if (stats === null) return head;
  if (fn === "count") return head + String(stats.countNumbers);
  if (fn === "countA") return head + String(stats.count);
  if (stats.sum === null) {
    // An error anywhere in the selection, or no numbers at all.
    if (stats.formattedSum !== null) return head + stats.formattedSum;
    return fn === "sum" ? `${head}0` : head;
  }
  const [value, text] =
    fn === "sum"
      ? [stats.sum, stats.formattedSum]
      : fn === "average"
        ? [stats.average, stats.formattedAverage]
        : fn === "max"
          ? [stats.max, stats.formattedMax]
          : [stats.min, stats.formattedMin];
  if (value === null) return head;
  if (value === 0) return `${head}0`;
  return head + (text ?? String(value));
}

/** The locked selection mode (AOO `SID_STATUS_SELMODE`: 0 STD, 1 EXT, 2 ADD). */
export type SelectionMode = "std" | "ext" | "add";

export function selectionModeOf(extend: boolean, add: boolean): SelectionMode {
  return extend ? "ext" : add ? "add" : "std";
}

/** A click on the status bar field: STD → EXT → ADD → STD (AOO `ScCellShell::Execute`, `SID_STATUS_SELMODE`). */
export function nextSelectionMode(mode: SelectionMode): SelectionMode {
  return mode === "std" ? "ext" : mode === "ext" ? "add" : "std";
}

/** The three-letter text of the field (AOO `RID_SVXSTR_SELMODE_*`). */
export function selectionModeLabel(mode: SelectionMode): string {
  return mode === "ext" ? t("status.selModeExt", "EXT") : mode === "add" ? t("status.selModeAdd", "ADD") : t("status.selModeStd", "STD");
}

/** What the mode does, for the tooltip and assistive technology. */
export function selectionModeHint(mode: SelectionMode): string {
  return mode === "ext"
    ? t("status.selModeExtHint", "Extend selection (F8)")
    : mode === "add"
      ? t("status.selModeAddHint", "Add to selection (⇧F8)")
      : t("status.selModeStdHint", "Standard selection");
}
