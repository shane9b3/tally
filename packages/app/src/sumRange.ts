/**
 * Σ Add up: the range to sum for the active cell. The contiguous block of
 * numeric cells directly above the active cell wins; otherwise the block
 * directly to the left; otherwise `null`.
 */
import type { CellPos, CellView, RangeDto } from "@tally/bridge";
import { cellAddress } from "@tally/grid";

/** Cell lookup used by {@link detectSumRange}; `undefined` = empty. */
export type CellLookup = (r: number, c: number) => CellView | undefined;

/** How far the scan goes (and how much {@link sumScanRanges} fetches). */
export const SUM_SCAN_ROWS = 2000;
export const SUM_SCAN_COLS = 256;

function isNumeric(cell: CellView | undefined): boolean {
  return cell !== undefined && cell.kind === "number";
}

export function detectSumRange(active: CellPos, get: CellLookup): RangeDto | null {
  const { r, c } = active;
  let top = r;
  while (top > 0 && r - top < SUM_SCAN_ROWS && isNumeric(get(top - 1, c))) top--;
  if (top < r) return { r0: top, c0: c, r1: r - 1, c1: c };
  let left = c;
  while (left > 0 && c - left < SUM_SCAN_COLS && isNumeric(get(r, left - 1))) left--;
  if (left < c) return { r0: r, c0: left, r1: r, c1: c - 1 };
  return null;
}

/** The ranges to fetch so {@link detectSumRange} sees every cell it may read. */
export function sumScanRanges(active: CellPos): RangeDto[] {
  const out: RangeDto[] = [];
  if (active.r > 0) out.push({ r0: Math.max(0, active.r - SUM_SCAN_ROWS), c0: active.c, r1: active.r - 1, c1: active.c });
  if (active.c > 0) out.push({ r0: active.r, c0: Math.max(0, active.c - SUM_SCAN_COLS), r1: active.r, c1: active.c - 1 });
  return out;
}

/** `A1` or `A1:B5`. */
export function rangeAddress(range: RangeDto): string {
  const a = cellAddress(range.r0, range.c0);
  return range.r0 === range.r1 && range.c0 === range.c1 ? a : `${a}:${cellAddress(range.r1, range.c1)}`;
}

/** The formula Σ inserts. */
export function sumFormula(range: RangeDto | null): string {
  return range ? `=SUM(${rangeAddress(range)})` : "=SUM()";
}
