/**
 * The DataPilot source filter (T17): the "Filter" button above a pivot
 * table (AOO `lcl_DoFilterButton`, dpoutput.cxx) opens AOO's
 * `ScPivotFilterDlg` (pfiltdlg.cxx): three conditions on the source
 * columns joined by AND/OR, only `=`, `<`, `>`, `<=`, `>=`, `<>`
 * (filter.src `RID_SCDLG_PIVOTFILTER`), and the options Case sensitive,
 * Regular expression and No duplications. OK sets the source query and
 * updates the pivot (`ScGridWindow::DoPushButton`, gridwin2.cxx).
 */
import type { CellPos, PivotDto, PivotSourceFilterDto } from "@tally/bridge";
import { t } from "../i18n";
import { STD_FILTER_OPS, type FilterRow } from "../data/builders";

/** The six conditions of AOO's DataPilot filter dialog. */
export const PIVOT_FILTER_OPS = STD_FILTER_OPS.slice(0, 6);

/** Condition rows in AOO's dialog. */
export const PIVOT_FILTER_ROWS = 3;

export interface PivotFilterOptions {
  caseSensitive: boolean;
  regex: boolean;
  noDuplicates: boolean;
}

/** The dialog rows for a pivot's current filter (`col` = source column; unused rows `null`). */
export function pivotFilterRows(f: PivotSourceFilterDto | null | undefined): FilterRow[] {
  const rows: FilterRow[] = [];
  for (const c of (f?.conditions ?? []).slice(0, PIVOT_FILTER_ROWS)) rows.push({ connector: c.connector, col: c.col, op: c.op, value: c.value });
  while (rows.length < PIVOT_FILTER_ROWS) rows.push({ connector: "and", col: null, op: "eq", value: "" });
  return rows;
}

export function pivotFilterOptions(f: PivotSourceFilterDto | null | undefined): PivotFilterOptions {
  return { caseSensitive: f?.caseSensitive ?? false, regex: f?.regex ?? false, noDuplicates: f?.noDuplicates ?? false };
}

/**
 * The filter the dialog sets: the used rows (up to the first unused one,
 * as AOO's `GetOutputItem`), or `null` when the first field is "- none -"
 * (no filter).
 */
export function pivotFilterDto(rows: readonly FilterRow[], o: PivotFilterOptions): PivotSourceFilterDto | null {
  const conditions: PivotSourceFilterDto["conditions"] = [];
  for (const r of rows) {
    if (r.col === null) break;
    conditions.push({ connector: conditions.length === 0 ? "and" : r.connector, col: r.col, op: r.op, value: r.value });
  }
  if (conditions.length === 0) return null;
  return { conditions, caseSensitive: o.caseSensitive, regex: o.regex, noDuplicates: o.noDuplicates };
}

/** Why the filter cannot be set (`null` = fine): an invalid regular expression. */
export function pivotFilterError(f: PivotSourceFilterDto | null): string | null {
  if (!f?.regex) return null;
  for (const c of f.conditions) {
    if (c.op !== "eq" && c.op !== "ne") continue;
    try {
      new RegExp(c.value);
    } catch {
      return t("lbl.err.regex", "“{value}” is not a valid regular expression.", { value: c.value });
    }
  }
  return null;
}

/** The "Filter" button cell of a pivot (the first output cell), if it shows one. */
export function filterButtonCell(p: PivotDto): CellPos | null {
  return p.showFilterButton && p.output ? { r: p.output.r0, c: p.output.c0 } : null;
}

/** The filter button cells on `sheet` (for the grid). */
export function pivotFilterButtons(pivots: readonly PivotDto[], sheet: number): CellPos[] {
  const out: CellPos[] = [];
  for (const p of pivots) {
    if (p.targetSheet !== sheet) continue;
    const cell = filterButtonCell(p);
    if (cell) out.push(cell);
  }
  return out;
}

/** The pivot whose filter button is `cell` on `sheet`. */
export function pivotWithButtonAt(pivots: readonly PivotDto[], sheet: number, cell: CellPos): PivotDto | null {
  return pivots.find((p) => p.targetSheet === sheet && filterButtonCell(p)?.r === cell.r && filterButtonCell(p)?.c === cell.c) ?? null;
}

/** The pivot with `sourceFilter` set to `f` (the `updatePivot` payload). */
export function withSourceFilter(p: PivotDto, f: PivotSourceFilterDto | null): PivotDto {
  return { ...p, sourceFilter: f };
}
