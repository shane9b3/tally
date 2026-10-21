/**
 * Outline gutter geometry (wave 5 seam, W5-0b): how much space the groups
 * take and what a click in the gutter means. Pure; the grid passes the sizes
 * into {@link Geometry} and the renderer draws the stub (level buttons in the
 * corner, a +/− box at each group's first row/column, a line along the group).
 */
import type { Geometry } from "./geometry";
import type { OutlineGroup, OutlineProps } from "./types";

/** Gutter width per level at zoom 100% (AOO uses about 13 px). */
export const OUTLINE_LEVEL_PX = 13;

/** Level columns the gutter needs for `groups`: the deepest level + 1 (the "all" button), 0 without groups. */
export function outlineLevels(groups: readonly OutlineGroup[]): number {
  let max = 0;
  for (const g of groups) max = Math.max(max, Math.min(8, Math.max(1, g.level)));
  return max === 0 ? 0 : max + 1;
}

/** What a gutter click hit. */
export type OutlineHit =
  | { kind: "level"; axis: "rows" | "cols"; level: number }
  | { kind: "group"; axis: "rows" | "cols"; group: OutlineGroup }
  | null;

/**
 * The outline element under canvas point (x, y). Level buttons sit in the
 * corner strip of each axis (`rows`: left of the row header, above the cell
 * area's top; `cols`: above the column header, left of the cell area); group
 * boxes sit in the level's column at the group's first row/column.
 */
export function outlineHit(geo: Geometry, outline: OutlineProps, x: number, y: number): OutlineHit {
  const levelPx = (outline.levelPx ?? OUTLINE_LEVEL_PX) * geo.zoom;
  if (geo.gutterW > 0 && x >= 0 && x < geo.gutterW) {
    const level = Math.floor(x / levelPx) + 1;
    if (y < geo.headerH) return { kind: "level", axis: "rows", level };
    const r = geo.rowAt(y);
    const group = outline.rows.find((g) => g.level === level && g.start === r) ?? outline.rows.find((g) => g.level === level && r >= g.start && r <= g.end);
    return group ? { kind: "group", axis: "rows", group } : null;
  }
  if (geo.gutterH > 0 && y >= 0 && y < geo.gutterH) {
    const level = Math.floor(y / levelPx) + 1;
    if (x < geo.headerW) return { kind: "level", axis: "cols", level };
    const c = geo.colAt(x);
    const group = outline.cols.find((g) => g.level === level && g.start === c) ?? outline.cols.find((g) => g.level === level && c >= g.start && c <= g.end);
    return group ? { kind: "group", axis: "cols", group } : null;
  }
  return null;
}
