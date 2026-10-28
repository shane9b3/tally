/**
 * Multi-sheet selection ("selected sheets", AOO ScMarkData's table marks):
 * pure helpers over the tab order. A group is the selected visible sheets
 * in tab order and always holds the active sheet; an empty group means one
 * sheet (only the active one) is selected. Hidden sheets are never grouped.
 */

/** The parts of a sheet the helpers read (`SheetInfo` fits). */
export interface TabSheet {
  id: number;
  hidden: boolean;
}

/** The visible sheet ids in tab order. */
export function visibleIds(sheets: readonly TabSheet[]): number[] {
  return sheets.filter((s) => !s.hidden).map((s) => s.id);
}

/**
 * A group cleaned against the sheets: unknown and hidden ids dropped, tab
 * order, the active sheet added; one sheet or fewer is no group (`[]`).
 */
export function normalizeGroup(sheets: readonly TabSheet[], group: readonly number[], active: number): number[] {
  if (group.length === 0) return [];
  const want = new Set(group);
  want.add(active);
  const out = visibleIds(sheets).filter((id) => want.has(id));
  return out.length > 1 && out.includes(active) ? out : [];
}

/** The selected sheets: the group, or the active sheet alone. */
export function selectedIds(group: readonly number[], active: number): number[] {
  return group.length > 1 ? [...group] : [active];
}

/** The transaction group: the active sheet first, then the others in tab order (`null` for one sheet). */
export function txGroup(group: readonly number[], active: number): number[] | null {
  if (group.length < 2 || !group.includes(active)) return null;
  return [active, ...group.filter((id) => id !== active)];
}

/** ⌘/Ctrl-click: `id` in or out of the group. The active sheet and hidden sheets don't change it. */
export function toggle(sheets: readonly TabSheet[], group: readonly number[], active: number, id: number): number[] {
  const current = selectedIds(group, active);
  if (id === active || !visibleIds(sheets).includes(id)) return normalizeGroup(sheets, current, active);
  const next = current.includes(id) ? current.filter((x) => x !== id) : [...current, id];
  return normalizeGroup(sheets, next, active);
}

/** Shift-click: the visible sheets from the active one to `id`, both included. */
export function rangeBetween(sheets: readonly TabSheet[], active: number, id: number): number[] {
  const ids = visibleIds(sheets);
  const a = ids.indexOf(active);
  const b = ids.indexOf(id);
  if (a < 0 || b < 0) return [];
  return normalizeGroup(sheets, ids.slice(Math.min(a, b), Math.max(a, b) + 1), active);
}

/** The next visible sheet `delta` tabs away (`wrap` at the ends), or `null`. */
export function stepVisible(sheets: readonly TabSheet[], active: number, delta: 1 | -1, wrap: boolean): number | null {
  const ids = visibleIds(sheets);
  if (ids.length === 0) return null;
  const i = ids.indexOf(active);
  if (i < 0) return ids[0] ?? null;
  let j = i + delta;
  if (j < 0 || j >= ids.length) {
    if (!wrap) return null;
    j = (j + ids.length) % ids.length;
  }
  return j === i ? null : (ids[j] ?? null);
}

/**
 * ⇧⌃PgDn / ⇧⌃PgUp (AOO ScTabView::SelectNextTab with extend): the next
 * visible sheet becomes active and joins the group. At the end nothing
 * changes (`null`).
 */
export function extendStep(sheets: readonly TabSheet[], group: readonly number[], active: number, delta: 1 | -1): { active: number; group: number[] } | null {
  const next = stepVisible(sheets, active, delta, false);
  if (next === null) return null;
  return { active: next, group: normalizeGroup(sheets, [...selectedIds(group, active), next], next) };
}

/**
 * The sheet to show after `removed` are deleted or hidden: the first
 * visible survivor after the first removed tab, else the last one before
 * it; `null` when nothing visible is left.
 */
export function nextActiveAfterDelete(sheets: readonly TabSheet[], removed: readonly number[], active: number): number | null {
  const gone = new Set(removed);
  if (!gone.has(active) && sheets.some((s) => s.id === active && !s.hidden)) return active;
  const first = sheets.findIndex((s) => gone.has(s.id));
  const ok = (s: TabSheet | undefined) => s !== undefined && !s.hidden && !gone.has(s.id);
  for (let i = Math.max(0, first); i < sheets.length; i++) if (ok(sheets[i])) return sheets[i]!.id;
  for (let i = first - 1; i >= 0; i--) if (ok(sheets[i])) return sheets[i]!.id;
  return null;
}

/**
 * Moves the `moving` sheets (kept in tab order, as one block) so they sit
 * before the sheet at full index `insertAt` of `order` (`order.length` =
 * the end). Returns the `moveSheet` steps that reach that order, applied
 * one after another (`to` is the index after the step), or `[]`.
 */
export function moveBlockSteps(order: readonly number[], moving: readonly number[], insertAt: number): { sheet: number; to: number }[] {
  const set = new Set(moving);
  const block = order.filter((id) => set.has(id));
  const rest = order.filter((id) => !set.has(id));
  const before = order.slice(0, Math.max(0, Math.min(order.length, insertAt))).filter((id) => !set.has(id)).length;
  const target = [...rest.slice(0, before), ...block, ...rest.slice(before)];
  const cur = [...order];
  const steps: { sheet: number; to: number }[] = [];
  for (let j = 0; j < target.length; j++) {
    const want = target[j]!;
    if (cur[j] === want) continue;
    cur.splice(cur.indexOf(want), 1);
    cur.splice(j, 0, want);
    steps.push({ sheet: want, to: j });
  }
  return steps;
}
