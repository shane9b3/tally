/**
 * Selection model and keyboard actions as pure functions (no DOM), so they
 * can be unit-tested. A selection is one current range plus, for
 * Cmd/Ctrl+click multi-selection, earlier ranges in `others`.
 */
import type { CellPos, RangeDto } from "@tally/bridge";
import type { Dir, Selection } from "./types";

export interface Limits {
  rows: number;
  cols: number;
  /** Rows per page for PageUp/PageDown. */
  pageRows: number;
  /** Columns per page for Alt+PageUp/PageDown. */
  pageCols: number;
  /** Used range of the sheet (Ctrl+A, End, Ctrl+End). */
  used: RangeDto | null;
}

export interface Mods {
  shift: boolean;
  /** Ctrl on Windows/Linux, Cmd on macOS. */
  mod: boolean;
  alt: boolean;
}

export const NO_MODS: Mods = { shift: false, mod: false, alt: false };

/** What a key press asks the grid to do. */
export type KeyAction =
  | { type: "select"; selection: Selection }
  | { type: "navigateEdge"; dir: Dir; extend: boolean }
  | { type: "startEdit" }
  | { type: "delete" }
  | { type: "none" };

export function cellSelection(cell: CellPos): Selection {
  return { range: { r0: cell.r, c0: cell.c, r1: cell.r, c1: cell.c }, active: { ...cell }, anchor: { ...cell } };
}

export function rangeOf(a: CellPos, b: CellPos): RangeDto {
  return { r0: Math.min(a.r, b.r), c0: Math.min(a.c, b.c), r1: Math.max(a.r, b.r), c1: Math.max(a.c, b.c) };
}

export function isSingleCell(r: RangeDto): boolean {
  return r.r0 === r.r1 && r.c0 === r.c1;
}

export function sameRange(a: RangeDto, b: RangeDto): boolean {
  return a.r0 === b.r0 && a.c0 === b.c0 && a.r1 === b.r1 && a.c1 === b.c1;
}

function sameOthers(a: RangeDto[] | undefined, b: RangeDto[] | undefined): boolean {
  const x = a ?? [];
  const y = b ?? [];
  return x.length === y.length && x.every((r, i) => {
    const o = y[i];
    return o !== undefined && sameRange(r, o);
  });
}

export function sameSelection(a: Selection, b: Selection): boolean {
  return (
    sameOthers(a.others, b.others) &&
    sameRange(a.range, b.range) &&
    a.active.r === b.active.r &&
    a.active.c === b.active.c &&
    a.anchor.r === b.anchor.r &&
    a.anchor.c === b.anchor.c
  );
}

export function inRange(r: RangeDto, cell: CellPos): boolean {
  return cell.r >= r.r0 && cell.r <= r.r1 && cell.c >= r.c0 && cell.c <= r.c1;
}

function clampCell(cell: CellPos, lim: Pick<Limits, "rows" | "cols">): CellPos {
  return {
    r: Math.max(0, Math.min(lim.rows - 1, cell.r)),
    c: Math.max(0, Math.min(lim.cols - 1, cell.c)),
  };
}

/**
 * The moving corner of a Shift-extended selection: the corner of the range
 * opposite the anchor.
 */
export function extentCorner(sel: Selection): CellPos {
  const { range, anchor } = sel;
  return { r: anchor.r === range.r0 ? range.r1 : range.r0, c: anchor.c === range.c0 ? range.c1 : range.c0 };
}

/** Extends the current range from its anchor to `cell` (Shift+click, drag); earlier ranges stay. */
export function extendTo(sel: Selection, cell: CellPos): Selection {
  const next: Selection = { range: rangeOf(sel.anchor, cell), active: { ...sel.anchor }, anchor: { ...sel.anchor } };
  if (sel.others && sel.others.length > 0) next.others = sel.others.map((r) => ({ ...r }));
  return next;
}

/** Every range of the selection, oldest first (the current range last). */
export function allRanges(sel: Selection): RangeDto[] {
  return [...(sel.others ?? []), sel.range];
}

/**
 * Cmd/Ctrl+click: keeps the current ranges and starts a new one at `cell`
 * (or at `range`, e.g. a header click).
 */
export function addRange(sel: Selection, cell: CellPos, range?: RangeDto): Selection {
  return {
    others: allRanges(sel).map((r) => ({ ...r })),
    range: range ? { ...range } : { r0: cell.r, c0: cell.c, r1: cell.r, c1: cell.c },
    active: { ...cell },
    anchor: { ...cell },
  };
}

/** Whole rows `r0..=r1` (header click); `active` defaults to the first cell. */
export function selectRows(r0: number, r1: number, lim: Pick<Limits, "cols">, anchorRow = r0, activeCol = 0): Selection {
  const a = { r: anchorRow, c: activeCol };
  return { range: { r0: Math.min(r0, r1), c0: 0, r1: Math.max(r0, r1), c1: lim.cols - 1 }, active: a, anchor: { ...a } };
}

/** Whole columns `c0..=c1` (header click). */
export function selectCols(c0: number, c1: number, lim: Pick<Limits, "rows">, anchorCol = c0, activeRow = 0): Selection {
  const a = { r: activeRow, c: anchorCol };
  return { range: { r0: 0, c0: Math.min(c0, c1), r1: lim.rows - 1, c1: Math.max(c0, c1) }, active: a, anchor: { ...a } };
}

export function selectAll(lim: Pick<Limits, "rows" | "cols">, active: CellPos = { r: 0, c: 0 }): Selection {
  return { range: { r0: 0, c0: 0, r1: lim.rows - 1, c1: lim.cols - 1 }, active: { ...active }, anchor: { ...active } };
}

/** Whether a range spans every row (a column selection). */
export function isWholeCols(r: RangeDto, lim: Pick<Limits, "rows">): boolean {
  return r.r0 === 0 && r.r1 === lim.rows - 1;
}

/** Whether a range spans every column (a row selection). */
export function isWholeRows(r: RangeDto, lim: Pick<Limits, "cols">): boolean {
  return r.c0 === 0 && r.c1 === lim.cols - 1;
}

const DELTA: Record<Dir, [number, number]> = {
  up: [-1, 0],
  down: [1, 0],
  left: [0, -1],
  right: [0, 1],
};

/** Moves (or, with `extend`, extends) the selection by (dr, dc). */
export function moveBy(sel: Selection, dr: number, dc: number, extend: boolean, lim: Limits): Selection {
  if (extend) {
    const corner = extentCorner(sel);
    return extendTo(sel, clampCell({ r: corner.r + dr, c: corner.c + dc }, lim));
  }
  return cellSelection(clampCell({ r: sel.active.r + dr, c: sel.active.c + dc }, lim));
}

/** Moves to (or extends to) an absolute cell. */
export function moveTo(sel: Selection, cell: CellPos, extend: boolean, lim: Limits): Selection {
  const target = clampCell(cell, lim);
  return extend ? extendTo(sel, target) : cellSelection(target);
}

/**
 * Tab/Enter inside a multi-cell selection: the active cell walks the range
 * (row-major for Tab, column-major for Enter) and wraps; the range stays.
 * In a single-cell selection it is a plain move.
 */
export function stepWithin(sel: Selection, dir: Dir, lim: Limits): Selection {
  const { range, active } = sel;
  if (isSingleCell(range)) {
    const [dr, dc] = DELTA[dir];
    return moveBy(sel, dr, dc, false, lim);
  }
  let { r, c } = active;
  const horizontal = dir === "left" || dir === "right";
  const fwd = dir === "right" || dir === "down";
  if (horizontal) {
    c += fwd ? 1 : -1;
    if (c > range.c1) {
      c = range.c0;
      r = r >= range.r1 ? range.r0 : r + 1;
    } else if (c < range.c0) {
      c = range.c1;
      r = r <= range.r0 ? range.r1 : r - 1;
    }
  } else {
    r += fwd ? 1 : -1;
    if (r > range.r1) {
      r = range.r0;
      c = c >= range.c1 ? range.c0 : c + 1;
    } else if (r < range.r0) {
      r = range.r1;
      c = c <= range.c0 ? range.c1 : c - 1;
    }
  }
  const cell = { r, c };
  return { range: { ...range }, active: cell, anchor: { ...cell } };
}

/** Ctrl/Cmd+A: the used range first, then the whole sheet. */
export function selectAllAction(sel: Selection, lim: Limits): Selection {
  const used = lim.used;
  if (used && !sameRange(sel.range, used) && !(sel.range.r0 <= used.r0 && sel.range.r1 >= used.r1 && sel.range.c0 <= used.c0 && sel.range.c1 >= used.c1)) {
    const a = { r: used.r0, c: used.c0 };
    return { range: { ...used }, active: a, anchor: { ...a } };
  }
  return selectAll(lim, inRange(sel.range, sel.active) ? sel.active : { r: 0, c: 0 });
}

/**
 * Keyboard reducer for the grid when no edit is in progress.
 * `key` is `KeyboardEvent.key`.
 */
export function reduceKey(sel: Selection, key: string, mods: Mods, lim: Limits): KeyAction {
  const select = (selection: Selection): KeyAction => ({ type: "select", selection });
  const { shift, mod, alt } = mods;
  switch (key) {
    case "ArrowUp":
    case "ArrowDown":
    case "ArrowLeft":
    case "ArrowRight": {
      const dir = key.slice(5).toLowerCase() as Dir;
      if (mod) return { type: "navigateEdge", dir, extend: shift };
      const [dr, dc] = DELTA[dir];
      return select(moveBy(sel, dr, dc, shift, lim));
    }
    case "Tab":
      return select(stepWithin(sel, shift ? "left" : "right", lim));
    case "Enter":
      if (mod || alt) return { type: "none" };
      return select(stepWithin(sel, shift ? "up" : "down", lim));
    case "Home": {
      const row = shift ? extentCorner(sel).r : sel.active.r;
      return select(moveTo(sel, mod ? { r: 0, c: 0 } : { r: row, c: 0 }, shift, lim));
    }
    case "End": {
      const used = lim.used;
      const lastR = used ? used.r1 : 0;
      const lastC = used ? used.c1 : 0;
      const row = shift ? extentCorner(sel).r : sel.active.r;
      return select(moveTo(sel, mod ? { r: lastR, c: lastC } : { r: row, c: lastC }, shift, lim));
    }
    case "PageUp":
    case "PageDown": {
      const sign = key === "PageDown" ? 1 : -1;
      if (alt) return select(moveBy(sel, 0, sign * Math.max(1, lim.pageCols), shift, lim));
      if (mod) return { type: "none" }; // Ctrl+PageUp/Down switches sheets (app keymap)
      return select(moveBy(sel, sign * Math.max(1, lim.pageRows), 0, shift, lim));
    }
    case "F2":
      // ⇧F2 is the app's "Insert note".
      return shift ? { type: "none" } : { type: "startEdit" };
    case "Delete":
    case "Backspace":
      return mod || alt ? { type: "none" } : { type: "delete" };
    case "a":
    case "A":
      if (mod && !alt && !shift) return select(selectAllAction(sel, lim));
      return { type: "none" };
    case "Escape":
    default:
      return { type: "none" };
  }
}

/** The cell an edge jump starts from: the moving corner when extending. */
export function edgeOrigin(sel: Selection, extend: boolean): CellPos {
  return extend ? extentCorner(sel) : sel.active;
}

/** Applies an edge-jump result. */
export function applyEdge(sel: Selection, target: CellPos, extend: boolean, lim: Limits): Selection {
  return moveTo(sel, target, extend, lim);
}

/** Fallback edge jump when the app does not provide `onNavigateEdge`: the sheet edge. */
export function sheetEdge(from: CellPos, dir: Dir, lim: Pick<Limits, "rows" | "cols">): CellPos {
  switch (dir) {
    case "up":
      return { r: 0, c: from.c };
    case "down":
      return { r: lim.rows - 1, c: from.c };
    case "left":
      return { r: from.r, c: 0 };
    case "right":
      return { r: from.r, c: lim.cols - 1 };
  }
}

// ---------------------------------------------------------------------------
// Merged cells
// ---------------------------------------------------------------------------

function overlaps(a: RangeDto, b: RangeDto): boolean {
  return a.r0 <= b.r1 && b.r0 <= a.r1 && a.c0 <= b.c1 && b.c0 <= a.c1;
}

/** The merged area containing `cell`, if any. */
export function mergeAt(merges: readonly RangeDto[], cell: CellPos): RangeDto | undefined {
  return merges.find((m) => inRange(m, cell));
}

/** Grows `range` until every merged area it touches lies inside it. */
export function expandToMerges(range: RangeDto, merges: readonly RangeDto[]): RangeDto {
  const out = { ...range };
  let grown = true;
  while (grown) {
    grown = false;
    for (const m of merges) {
      if (!overlaps(out, m)) continue;
      if (m.r0 < out.r0 || m.c0 < out.c0 || m.r1 > out.r1 || m.c1 > out.c1) {
        out.r0 = Math.min(out.r0, m.r0);
        out.c0 = Math.min(out.c0, m.c0);
        out.r1 = Math.max(out.r1, m.r1);
        out.c1 = Math.max(out.c1, m.c1);
        grown = true;
      }
    }
  }
  return out;
}

/**
 * Snaps a selection to merged areas (AOO/Excel behaviour): the active cell
 * becomes the anchor of its merged area, the range covers whole merged
 * areas, and a single-cell move that stays inside the area it started from
 * (an arrow key from the anchor) continues past the area instead.
 */
export function snapToMerges(sel: Selection, prev: Selection | null, merges: readonly RangeDto[], lim: Pick<Limits, "rows" | "cols">): Selection {
  if (merges.length === 0) return sel;
  let next = sel;
  const single = isSingleCell(sel.range) && (sel.others ?? []).length === 0;
  const m = mergeAt(merges, sel.active);
  if (single && m && prev && inRange(m, prev.active) && (isSingleCell(prev.range) || sameRange(prev.range, m)) && !(sel.active.r === m.r0 && sel.active.c === m.c0)) {
    const dr = Math.sign(sel.active.r - prev.active.r);
    const dc = Math.sign(sel.active.c - prev.active.c);
    if (dr !== 0 || dc !== 0) {
      const r = dr > 0 ? m.r1 + 1 : dr < 0 ? m.r0 - 1 : prev.active.r;
      const c = dc > 0 ? m.c1 + 1 : dc < 0 ? m.c0 - 1 : prev.active.c;
      if (r >= 0 && c >= 0 && r < lim.rows && c < lim.cols) next = cellSelection({ r, c });
      else next = cellSelection({ r: m.r0, c: m.c0 });
    }
  }
  const am = mergeAt(merges, next.active);
  const active = am ? { r: am.r0, c: am.c0 } : next.active;
  const range = expandToMerges(next.range, merges);
  const anchorMerge = mergeAt(merges, next.anchor);
  const anchor = isSingleCell(next.range) ? { ...active } : anchorMerge && sameRange(anchorMerge, range) ? { r: anchorMerge.r0, c: anchorMerge.c0 } : next.anchor;
  const out: Selection = { range, active, anchor };
  if (next.others && next.others.length > 0) out.others = next.others.map((r) => expandToMerges(r, merges));
  return sameSelection(out, sel) ? sel : out;
}
