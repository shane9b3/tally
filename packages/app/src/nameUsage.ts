/**
 * Name usage flags (AOO Define Names "More >>" options: print range, filter,
 * repeat row, repeat column; `ScRangeData` RT_PRINTAREA / RT_CRITERIA /
 * RT_ROWHEADER / RT_COLHEADER) and the dialog lists that offer the flagged
 * names, as AOO 4.1 does:
 *
 * - Advanced Filter lists names marked "filter" and fills the criteria field
 *   with the name's reference (`ScSpecialFilterDlg::Init`, `FilterAreaSelHdl`).
 * - Format ▸ Print Ranges ▸ Edit lists names marked print range / repeat row /
 *   repeat column that are cell ranges; picking one fills the field with the
 *   range (`$A$1:$G$15`), the rows (`$1:$2`) or the columns (`$A:$B`)
 *   (`ScPrintAreasDlg::Impl_FillLists`, `lcl_GetRepeatRangeString`). Tally's
 *   equivalent is the Sheet tab of Page Setup.
 * - Typing in a field selects the matching list entry, else "user defined"
 *   (`Impl_ModifyHdl`, `FilterAreaModHdl`).
 */
import type { NameInfo, NameUsage, RangeDto } from "@tally/bridge";
import { colToLetters } from "@tally/grid";
import { parseAbsoluteRef } from "./dataOps";

export const NO_USAGE: NameUsage = { printRange: false, filter: false, repeatRow: false, repeatColumn: false };

/** One list entry: the name and the text it puts in the field. */
export interface NameChoice {
  name: string;
  text: string;
}

/** The cell range a name refers to (`$Sheet1.$A$1:$B$5`), or `null` for expressions. */
export function nameRange(n: NameInfo): { sheet: string; range: RangeDto } | null {
  return parseAbsoluteRef(n.refersTo);
}

/** `$1:$2` (rows) or `$A:$B` (columns) of a range, one part when it is a single row/column. */
export function repeatText(range: RangeDto, axis: "row" | "col"): string {
  const part = (i: number) => (axis === "row" ? `$${i + 1}` : `$${colToLetters(i)}`);
  const [a, b] = axis === "row" ? [range.r0, range.r1] : [range.c0, range.c1];
  return a === b ? part(a) : `${part(a)}:${part(b)}`;
}

/** `$A$1:$G$15` (no sheet: the print range belongs to the sheet the dialog edits). */
export function printRangeText(range: RangeDto): string {
  const cell = (r: number, c: number) => `$${colToLetters(c)}$${r + 1}`;
  const tl = cell(range.r0, range.c0);
  return range.r0 === range.r1 && range.c0 === range.c1 ? tl : `${tl}:${cell(range.r1, range.c1)}`;
}

/** Names marked "filter": the criteria field gets the name's reference text. */
export function criteriaChoices(names: readonly NameInfo[]): NameChoice[] {
  return names.filter((n) => n.usage?.filter).map((n) => ({ name: n.name, text: n.refersTo }));
}

/** Names marked for the print range / rows to repeat / columns to repeat lists (cell ranges only). */
export function printChoices(names: readonly NameInfo[], kind: "printRange" | "repeatRow" | "repeatColumn"): NameChoice[] {
  const out: NameChoice[] = [];
  for (const n of names) {
    if (!n.usage?.[kind]) continue;
    const r = nameRange(n);
    if (!r) continue;
    const text = kind === "printRange" ? printRangeText(r.range) : repeatText(r.range, kind === "repeatRow" ? "row" : "col");
    out.push({ name: n.name, text });
  }
  return out;
}

/**
 * The entry whose text equals the field, or `null`. AOO compares the text as
 * typed and upper-cased; Tally also ignores `$`, because its fields show
 * relative text (`A1:G15`) for the same range.
 */
export function matchChoice(choices: readonly NameChoice[], text: string): NameChoice | null {
  const norm = (s: string) => s.trim().replace(/\$/g, "").toUpperCase();
  const t = norm(text);
  if (t === "") return null;
  return choices.find((c) => norm(c.text) === t) ?? null;
}
