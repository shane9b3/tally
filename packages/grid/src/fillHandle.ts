/**
 * Fill-handle geometry as pure functions: which range a drag to `cell`
 * fills. The fill extends the source along ONE axis (the one the pointer
 * moved further outside the source on), like Calc's AutoFill.
 */
import type { CellPos, RangeDto } from "@tally/bridge";

export type FillDir = "down" | "up" | "right" | "left";

/**
 * The fill target for a drag from `source`'s handle to `cell`: the source
 * extended down/up/right/left, or `null` while the pointer is inside the
 * source (no fill). The target always includes the source.
 */
export function fillTarget(source: RangeDto, cell: CellPos): { target: RangeDto; dir: FillDir } | null {
  const below = cell.r - source.r1;
  const above = source.r0 - cell.r;
  const right = cell.c - source.c1;
  const left = source.c0 - cell.c;
  const vert = Math.max(below, above, 0);
  const horiz = Math.max(right, left, 0);
  if (vert === 0 && horiz === 0) return null;
  if (vert >= horiz) {
    if (below > 0) return { target: { ...source, r1: cell.r }, dir: "down" };
    return { target: { ...source, r0: cell.r }, dir: "up" };
  }
  if (right > 0) return { target: { ...source, c1: cell.c }, dir: "right" };
  return { target: { ...source, c0: cell.c }, dir: "left" };
}

/** Direction of a fill from `source` to `target` (target ⊇ source along one axis). */
export function fillDirection(source: RangeDto, target: RangeDto): FillDir | null {
  if (target.r1 > source.r1) return "down";
  if (target.r0 < source.r0) return "up";
  if (target.c1 > source.c1) return "right";
  if (target.c0 < source.c0) return "left";
  return null;
}
