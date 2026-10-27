/**
 * The mock's `nextUnlocked`, the same walk as the engine's
 * `protection::next_unlocked` (AOO `ScTable::ValidNextPos`): the next unlocked
 * cell in reading order, skipping hidden rows and columns and the covered cells
 * of merges, wrapping, optionally inside a selection.
 */
import type { CellPos, Direction, RangeDto } from "@tally/bridge";

export interface UnlockedSheet {
  /** Unlocked cells (ranges). */
  unlocked: readonly RangeDto[];
  rowHidden(r: number): boolean;
  colHidden(c: number): boolean;
  /** A non-anchor cell of a merge. */
  covered(r: number, c: number): boolean;
  rows: number;
  cols: number;
}

type Rect = readonly [number, number, number, number];

function firstAfter(rects: readonly Rect[], R: number, C: number): [number, number] | null {
  let best: [number, number] | null = null;
  for (const [a0, b0, a1, b1] of rects) {
    let cand: [number, number] | null = null;
    if (R < a0) cand = [a0, b0];
    else if (R <= a1) {
      if (C < b1) cand = [R, Math.max(C + 1, b0)];
      else if (R < a1) cand = [R + 1, b0];
    }
    if (cand && (!best || cand[0] < best[0] || (cand[0] === best[0] && cand[1] < best[1]))) best = cand;
  }
  return best;
}

export function nextUnlocked(sh: UnlockedSheet, from: CellPos, dir: Direction, within: RangeDto | null): CellPos | null {
  const byCol = dir === "up" || dir === "down";
  const fwd = dir === "right" || dir === "down";
  const sg = fwd ? 1 : -1;
  const clip = within ?? { r0: 0, c0: 0, r1: sh.rows - 1, c1: sh.cols - 1 };
  const rects: Rect[] = [];
  for (const s of sh.unlocked) {
    const r0 = Math.max(s.r0, clip.r0);
    const r1 = Math.min(s.r1, clip.r1);
    const c0 = Math.max(s.c0, clip.c0);
    const c1 = Math.min(s.c1, clip.c1);
    if (r0 > r1 || c0 > c1) continue;
    const [a0, a1, b0, b1] = byCol ? [c0, c1, r0, r1] : [r0, r1, c0, c1];
    rects.push(fwd ? [a0, b0, a1, b1] : [-a1, -b1, -a0, -b0]);
  }
  if (rects.length === 0) return null;
  let major = (byCol ? from.c : from.r) * sg;
  let minor = (byCol ? from.r : from.c) * sg;
  let wrapped = false;
  for (let guard = 0; guard < 4_000_000; guard++) {
    const hit = firstAfter(rects, major, minor);
    if (!hit) {
      if (wrapped) return null;
      wrapped = true;
      major = -4_000_000;
      minor = 0;
      continue;
    }
    const a = hit[0] * sg;
    const b = hit[1] * sg;
    const r = byCol ? b : a;
    const c = byCol ? a : b;
    if (byCol ? sh.colHidden(c) : sh.rowHidden(r)) {
      major = hit[0];
      minor = 4_000_000;
      continue;
    }
    if (sh.colHidden(c) || sh.rowHidden(r) || sh.covered(r, c)) {
      major = hit[0];
      minor = hit[1];
      continue;
    }
    return { r, c };
  }
  return null;
}
