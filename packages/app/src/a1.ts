/** A1 reference parsing for assist action arguments (`F2:F15`, `G`, `B7`). */
import type { RangeDto } from "@tally/bridge";

const LAST_ROW = 1_048_575;
const LAST_COL = 16_383;

/** 0-based column of letters such as `A`, `AB`; `null` if invalid. */
export function columnIndex(letters: string): number | null {
  if (!/^[A-Za-z]{1,3}$/.test(letters)) return null;
  let n = 0;
  for (const ch of letters.toUpperCase()) n = n * 26 + (ch.charCodeAt(0) - 64);
  return n - 1 <= LAST_COL ? n - 1 : null;
}

function cell(text: string): { r: number; c: number } | null {
  const m = /^\$?([A-Za-z]{1,3})\$?(\d{1,7})$/.exec(text.trim());
  if (!m) return null;
  const c = columnIndex(m[1] ?? "");
  const r = Number(m[2]) - 1;
  if (c === null || r < 0 || r > LAST_ROW) return null;
  return { r, c };
}

/** `A1` or `A1:B5` (either corner order) as a range; `null` if invalid. */
export function parseA1Range(text: string): RangeDto | null {
  const [a, b, extra] = text.split(":");
  if (a === undefined || extra !== undefined) return null;
  const p = cell(a);
  const q = b === undefined ? p : cell(b);
  if (!p || !q) return null;
  return { r0: Math.min(p.r, q.r), c0: Math.min(p.c, q.c), r1: Math.max(p.r, q.r), c1: Math.max(p.c, q.c) };
}
