/**
 * The browser mock's outline emulation (Data ▸ Group and Outline): pure
 * reducers over the groups of a sheet, used by `mockW56.ts` to answer the
 * outline ops and report the groups through `sheetLayout().outline` the
 * way the engine does. The app itself reads the groups from the engine's
 * `SheetLayout.outline` (`W56Features.syncOutline`).
 */
import type { Op, OutlineGroupDto } from "@tally/bridge";

export interface OutlineModel {
  rows: OutlineGroupDto[];
  cols: OutlineGroupDto[];
}

export const MAX_OUTLINE_LEVELS = 8;

export function emptyOutlineModel(): OutlineModel {
  return { rows: [], cols: [] };
}

export function groupsOf(m: OutlineModel, axis: "rows" | "cols"): OutlineGroupDto[] {
  return m[axis].map((g) => ({ ...g }));
}

function sorted(groups: OutlineGroupDto[]): OutlineGroupDto[] {
  return [...groups].sort((a, b) => a.level - b.level || a.start - b.start);
}

/**
 * Groups `start..=end`: the new group's level is one more than the deepest
 * existing group that contains it (AOO nests inward); a group with the
 * same span and level already present is left alone.
 */
export function addGroup(groups: readonly OutlineGroupDto[], start: number, end: number): OutlineGroupDto[] {
  const containing = groups.filter((g) => g.start <= start && g.end >= end);
  const level = Math.min(MAX_OUTLINE_LEVELS, containing.reduce((m, g) => Math.max(m, g.level), 0) + 1);
  if (groups.some((g) => g.start === start && g.end === end && g.level === level)) return [...groups];
  // Groups inside the new span that sat at this level (or deeper) move one level inward.
  const out = groups.map((g) => (g.start >= start && g.end <= end && !(g.start === start && g.end === end) && g.level >= level ? { ...g, level: Math.min(MAX_OUTLINE_LEVELS, g.level + 1) } : { ...g }));
  out.push({ start, end, level, collapsed: false });
  return sorted(out);
}

/** Removes the innermost group spanning exactly (or containing) `start..=end`. */
export function removeGroup(groups: readonly OutlineGroupDto[], start: number, end: number): OutlineGroupDto[] {
  const exact = groups.filter((g) => g.start === start && g.end === end).sort((a, b) => b.level - a.level)[0];
  const target = exact ?? groups.filter((g) => g.start <= start && g.end >= end).sort((a, b) => b.level - a.level)[0];
  if (!target) return [...groups];
  const rest = groups.filter((g) => g !== target);
  // Groups that were nested inside it move one level outward.
  return sorted(rest.map((g) => (g.start >= target.start && g.end <= target.end && g.level > target.level ? { ...g, level: g.level - 1 } : { ...g })));
}

export function setCollapsed(groups: readonly OutlineGroupDto[], start: number, end: number, collapsed: boolean): OutlineGroupDto[] {
  return groups.map((g) => (g.start === start && g.end === end ? { ...g, collapsed } : { ...g }));
}

/** Show level `n`: groups deeper than `n` collapse, the others expand. */
export function showLevel(groups: readonly OutlineGroupDto[], level: number): OutlineGroupDto[] {
  return groups.map((g) => ({ ...g, collapsed: g.level >= level }));
}

/**
 * AutoOutline over `range` from the formula cells: a row (column) whose
 * formulas reference the rows (columns) just above (left) ends a group of
 * those rows. The engine computes the real outline; this approximation
 * keeps the gutter in step for the common SUM-below-a-block layout.
 */
export function autoOutlineGroups(isFormula: (r: number, c: number) => boolean, range: { r0: number; c0: number; r1: number; c1: number }): OutlineModel {
  const rows: OutlineGroupDto[] = [];
  let runStart: number | null = null;
  for (let r = range.r0; r <= range.r1; r++) {
    let formulaRow = false;
    let dataRow = false;
    for (let c = range.c0; c <= range.c1; c++) {
      if (isFormula(r, c)) formulaRow = true;
      else dataRow = true;
    }
    if (formulaRow && !dataRow) {
      if (runStart !== null && r - 1 >= runStart) rows.push({ start: runStart, end: r - 1, level: 1, collapsed: false });
      runStart = null;
    } else if (runStart === null) runStart = r;
  }
  return { rows, cols: [] };
}

/** Applies an outline op to the model (other ops leave it unchanged). */
export function applyOutlineOp(m: OutlineModel, op: Op): OutlineModel {
  switch (op.type) {
    case "group":
      return { ...m, [op.axis]: addGroup(m[op.axis], op.start, op.end) };
    case "ungroup":
      return { ...m, [op.axis]: removeGroup(m[op.axis], op.start, op.end) };
    case "setOutlineCollapsed":
      return { ...m, [op.axis]: setCollapsed(m[op.axis], op.start, op.end, op.collapsed) };
    case "showOutlineLevel":
      return { ...m, [op.axis]: showLevel(m[op.axis], op.level) };
    case "clearOutline":
      return op.axis === null ? emptyOutlineModel() : { ...m, [op.axis]: [] };
    case "autoOutline":
      // The engine decides the groups; the gutter refreshes from `afterResult` when a query exists.
      return m;
    default:
      return m;
  }
}

/** The rows (or columns) hidden by collapsed groups. */
export function hiddenByOutline(groups: readonly OutlineGroupDto[]): number[] {
  const set = new Set<number>();
  for (const g of groups) if (g.collapsed) for (let i = g.start; i <= g.end; i++) set.add(i);
  return [...set].sort((a, b) => a - b);
}
