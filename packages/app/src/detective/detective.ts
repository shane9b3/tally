/**
 * Tools ▸ Detective (W5-5, §4 item 34): trace precedents / dependents
 * through the engine's `precedents()` / `dependents()` queries, drawn as
 * grid `traces` arrows; "Mark invalid data" circles the cells the
 * engine's `invalid_cells` query reports. `cellValid` / `invalidCells`
 * are the browser mock's approximation of that query.
 */
import type { CellPos, CellView, CmpOp, RangeDto, TraceDto, ValidationDto } from "@tally/bridge";
import type { TraceArrow } from "@tally/grid";
import type { AppController } from "../controller";
import { useUiStore } from "../stores/uiStore";
import { useW5Store } from "../objects/w5Store";
import { clientV4 } from "../objects/shell";

/** Cells the browser mock scans at most for "Mark invalid data". */
export const INVALID_SCAN_CAP = 50_000;

/** Arrows for the traces of `cell` (precedents point at it, dependents away from it); other sheets are skipped. */
export function traceArrows(cell: CellPos, traces: readonly TraceDto[], sheet: number, direction: "precedent" | "dependent"): { arrows: TraceArrow[]; otherSheets: number } {
  const arrows: TraceArrow[] = [];
  let otherSheets = 0;
  for (const t of traces) {
    if (t.sheet !== sheet) {
      otherSheets++;
      continue;
    }
    const target = { r: t.range.r0, c: t.range.c0 };
    const kind = t.error ? "error" : direction;
    arrows.push(direction === "precedent" ? { from: target, to: { ...cell }, kind } : { from: { ...cell }, to: target, kind });
  }
  return { arrows, otherSheets };
}

const inRange = (rg: RangeDto, r: number, c: number): boolean => r >= rg.r0 && r <= rg.r1 && c >= rg.c0 && c <= rg.c1;

/** Compares `a` with the bounds `b`/`b2` for a `CmpOp`. */
function compare(op: CmpOp, a: number, b: number, b2: number | null): boolean {
  switch (op) {
    case "between":
      return a >= b && a <= (b2 ?? b);
    case "notBetween":
      return a < b || a > (b2 ?? b);
    case "eq":
      return a === b;
    case "ne":
      return a !== b;
    case "lt":
      return a < b;
    case "le":
      return a <= b;
    case "gt":
      return a > b;
    case "ge":
      return a >= b;
    default:
      return true;
  }
}

/** A numeric literal bound of a validation formula, or `null` (references are not evaluated here). */
function bound(formula: string | undefined): number | null {
  if (formula === undefined) return null;
  const n = Number(formula.trim().replace(/^=/, ""));
  return Number.isFinite(n) ? n : null;
}

/**
 * Whether a cell view satisfies a validation: lists of literal values,
 * whole/decimal/text-length rules with literal bounds; rules with
 * references, dates, times and custom formulas pass (the engine would be
 * needed).
 */
export function cellValid(v: ValidationDto, cell: CellView | undefined): boolean {
  const text = cell?.text ?? "";
  const empty = !cell || cell.kind === "empty" || text === "";
  if (empty) return v.allowBlank;
  const k = v.kind;
  const num = cell.num ?? (cell.kind === "number" ? Number(text) : null);
  switch (k.type) {
    case "list":
      return k.source.type === "values" ? k.source.items.some((it) => it === text) : true;
    case "whole":
    case "decimal": {
      if (num === null || !Number.isFinite(num)) return false;
      if (k.type === "whole" && !Number.isInteger(num)) return false;
      const a = bound(k.formulas[0]);
      return a === null ? true : compare(k.op, num, a, bound(k.formulas[1]));
    }
    case "textLength": {
      const a = bound(k.formulas[0]);
      return a === null ? true : compare(k.op, text.length, a, bound(k.formulas[1]));
    }
    default:
      return true;
  }
}

/** Cells of `cells` that break one of `validations`. */
export function invalidCells(validations: readonly ValidationDto[], cells: readonly CellView[]): CellPos[] {
  const out: CellPos[] = [];
  const byKey = new Map(cells.map((c) => [`${c.r},${c.c}`, c]));
  for (const v of validations) {
    for (const rg of v.ranges) {
      for (const cell of cells) {
        if (!inRange(rg, cell.r, cell.c)) continue;
        if (!cellValid(v, byKey.get(`${cell.r},${cell.c}`))) out.push({ r: cell.r, c: cell.c });
      }
    }
  }
  return out;
}

export class Detective {
  constructor(private readonly ctl: AppController) {}

  private toast(kind: "info" | "error", message: string): void {
    useUiStore.getState().pushToast(kind, message);
  }

  /** ⇧F7: arrows from the cells the active formula reads. */
  async tracePrecedents(): Promise<void> {
    await this.trace("precedent");
  }

  /** ⇧F5: arrows to the formulas reading the active cell. */
  async traceDependents(): Promise<void> {
    await this.trace("dependent");
  }

  private async trace(direction: "precedent" | "dependent"): Promise<void> {
    try {
      const doc = this.ctl.requireDoc();
      const sheet = this.ctl.sheet;
      const cell = { ...this.ctl.selection.active };
      const list = direction === "precedent" ? await clientV4(this.ctl).precedents(doc, sheet, cell.r, cell.c) : await clientV4(this.ctl).dependents(doc, sheet, cell.r, cell.c);
      const { arrows, otherSheets } = traceArrows(cell, list, sheet, direction);
      if (arrows.length === 0 && otherSheets === 0) {
        this.toast("info", direction === "precedent" ? "This cell has no precedents (it is not a formula, or it reads nothing)." : "No formula reads this cell.");
        return;
      }
      useW5Store.getState().addTraces(arrows);
      if (otherSheets > 0) this.toast("info", `${otherSheets} ${direction === "precedent" ? "precedent" : "dependent"}${otherSheets === 1 ? "" : "s"} on other sheets (not drawn).`);
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  /** Tools ▸ Detective ▸ Remove All Traces. */
  removeTraces(): void {
    useW5Store.getState().setTraces([]);
    useW5Store.getState().setInvalidCells([]);
  }

  /** Tools ▸ Detective ▸ Mark Invalid Data: circles the cells the engine's `invalid_cells` reports (≤ 10,000). */
  async markInvalid(): Promise<void> {
    try {
      const doc = this.ctl.requireDoc();
      const sheet = this.ctl.sheet;
      const bad = await clientV4(this.ctl).invalidCells(doc, sheet, null);
      if (this.ctl.sheet !== sheet) return;
      useW5Store.getState().setInvalidCells(bad);
      this.toast("info", bad.length === 0 ? "No invalid data found." : `${bad.length} invalid ${bad.length === 1 ? "cell" : "cells"} circled.`);
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }
}
