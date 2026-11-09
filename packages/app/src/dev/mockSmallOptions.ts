/**
 * The web mock's stand-ins for two small engine options (the real ones are
 * in tally-numfmt / tally-types):
 *
 * - the two-digit year window (P17): `M/D/YY` input becomes a date whose
 *   year is expanded like AOO's `SvNumberFormatter::ExpandTwoDigitYear`;
 * - "Limit decimals for general number format": General numbers shown with
 *   at most N decimals (AOO `ImpGetOutputStandard`).
 */

/** AOO's default window start (`Office.Common/DateFormat/TwoDigitYear`). */
export const DEFAULT_TWO_DIGIT_YEAR = 1930;

/** A two-digit year in the window starting at `start` (AOO ExpandTwoDigitYear). */
export function expandTwoDigitYear(yy: number, start: number): number {
  if (yy < 0 || yy > 99) return yy;
  const century = Math.floor(start / 100) * 100;
  return yy < start % 100 ? yy + century + 100 : yy + century;
}

/** Days since 1899-12-30 (the 1899 epoch) of a calendar date. */
export function serialOf(y: number, m: number, d: number): number {
  return Math.round((Date.UTC(y, m - 1, d) - Date.UTC(1899, 11, 30)) / 86_400_000);
}

export interface TypedDate {
  serial: number;
  y: number;
  m: number;
  d: number;
}

/** `M/D/YY` (en-US, two-digit year only) as a date in the window; `null` for anything else. */
export function recognizeTwoDigitDate(input: string, start: number): TypedDate | null {
  const m = /^\s*(\d{1,2})\/(\d{1,2})\/(\d{2})\s*$/.exec(input);
  if (!m) return null;
  const month = Number(m[1]);
  const day = Number(m[2]);
  const y = expandTwoDigitYear(Number(m[3]), start);
  const dt = new Date(Date.UTC(y, month - 1, day));
  if (month < 1 || month > 12 || dt.getUTCDate() !== day || dt.getUTCMonth() !== month - 1) return null;
  return { serial: serialOf(y, month, day), y, m: month, d: day };
}

const pad = (n: number, w = 2) => String(n).padStart(w, "0");

/** The cell text (`MM/DD/YY`, the implied format) and input-line text (`MM/DD/YYYY`) of a typed date. */
export function dateTexts(t: TypedDate): { cell: string; edit: string } {
  return { cell: `${pad(t.m)}/${pad(t.d)}/${pad(t.y % 100)}`, edit: `${pad(t.m)}/${pad(t.d)}/${pad(t.y, 4)}` };
}

/** General with at most `decimals` decimals: rounded, no trailing zeros, no `-0`; scientific beyond 1E15. */
export function limitGeneral(n: number, decimals: number): string {
  if (!Number.isFinite(n)) return "#NUM!";
  if (Math.abs(n) > 1e15) {
    const [mant, exp] = n.toExponential(Math.min(decimals, 14)).split("e");
    const e = Number(exp);
    return `${mant}E${e < 0 ? "-" : "+"}${pad(Math.abs(e))}`;
  }
  let s = n.toFixed(decimals);
  if (s.includes(".")) s = s.replace(/0+$/, "").replace(/\.$/, "");
  return /^-0$/.test(s) ? "0" : s;
}
