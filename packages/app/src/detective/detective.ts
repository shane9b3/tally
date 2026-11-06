/**
 * Tools ▸ Detective (W5-5, §4 item 34; AOO 4.1 parity): every command goes
 * to the engine's `detective()` (AOO `ScDocFunc::Detective*`), which keeps
 * the arrows and the document's operation list (Refresh Traces replays it;
 * AutoRefresh replays it after changes and the result says
 * `detectiveChanged`). The arrows are drawn as grid `traces`; Fill Mode
 * (AOO's auditing shell) turns clicks on cells into Trace Precedents.
 * "Mark invalid data" circles the cells the engine's `invalid_cells` query
 * reports; `cellValid` / `invalidCells` are the browser mock's
 * approximation of that query.
 */
import type { CellPos, CellView, CmpOp, DetectiveAction, DetectiveArrowDto, RangeDto, ValidationDto } from "@tally/bridge";
import { cellSelection, type TraceArrow } from "@tally/grid";
import type { AppController } from "../controller";
import { useUiStore } from "../stores/uiStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useW5Store } from "../objects/w5Store";
import { useW56Store } from "../data/store";
import { useDocStore } from "../stores/docStore";
import { clientV4 } from "../objects/shell";
import { t } from "../i18n";

/** Cells the browser mock scans at most for "Mark invalid data". */
export const INVALID_SCAN_CAP = 50_000;

/** Grid arrows for the engine's arrows of a sheet. */
export function toTraceArrows(arrows: readonly DetectiveArrowDto[]): TraceArrow[] {
  const out: TraceArrow[] = [];
  for (const a of arrows) {
    if (!a.from && !a.to) continue;
    const from = a.from ? { r: a.from.r0, c: a.from.c0 } : { ...(a.to as CellPos) };
    const to = a.to ? { ...a.to } : { ...from };
    out.push({
      from,
      to,
      kind: a.error ? "error" : "precedent",
      fromRange: a.from && (a.from.r0 !== a.from.r1 || a.from.c0 !== a.from.c1) ? { ...a.from } : null,
      ...(a.from ? {} : { fromOtherSheet: true }),
      ...(a.to ? {} : { toOtherSheet: true }),
    });
  }
  return out;
}

/** The message for a command that changed nothing (AOO beeps). */
function nothingMessage(action: DetectiveAction): string {
  switch (action) {
    case "tracePrecedents":
      return t("detective.none.tracePrecedents", "No further precedents to trace.");
    case "traceDependents":
      return t("detective.none.traceDependents", "No further dependents to trace.");
    case "traceError":
      return t("detective.none.traceError", "This cell holds no error to trace.");
    case "refresh":
      return t("detective.none.refresh", "There are no traces to refresh.");
    default:
      return t("detective.none.remove", "There are no traces to remove here.");
  }
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
  /** The document AutoRefresh was last sent to. */
  private syncedDoc: number | null = null;
  /** Bumped by every arrow fetch, so a slow answer never overwrites a newer one. */
  private seq = 0;

  constructor(private readonly ctl: AppController) {
    // AutoRefresh follows the app option (this window's toggle or another window's prefs).
    useW56Store.subscribe((s, prev) => {
      if (s.prefs.detectiveAuto !== prev.prefs.detectiveAuto) void this.syncAuto();
    });
  }

  private toast(kind: "info" | "error", message: string): void {
    useUiStore.getState().pushToast(kind, message);
  }

  private setArrows(arrows: readonly DetectiveArrowDto[]): void {
    useW5Store.getState().setTraces(toTraceArrows(arrows));
  }

  /** A command that did something modified the document (engine `dirty`): the close guard must ask. */
  private noteDirty(doc: number, dirty: boolean): void {
    if (!dirty) return;
    useDocStore.setState((s) => (s.info && s.info.doc === doc && !s.info.dirty ? { info: { ...s.info, dirty: true } } : {}));
  }

  /** Runs a command on `cell` (the active cell by default) and shows the sheet's arrows. */
  async run(action: DetectiveAction, cell?: CellPos): Promise<boolean> {
    try {
      const doc = this.ctl.requireDoc();
      const sheet = this.ctl.sheet;
      const at = cell ?? { ...this.ctl.selection.active };
      const seq = ++this.seq;
      const st = await clientV4(this.ctl).detective(doc, sheet, at.r, at.c, action);
      this.noteDirty(doc, st.dirty);
      if (seq === this.seq && this.ctl.sheet === sheet) this.setArrows(st.arrows);
      if (!st.done) this.toast("info", nothingMessage(action));
      else if (st.incomplete) this.toast("info", t("detective.incomplete", "Not every trace could be drawn: the detective stopped at its work limit."));
      return st.done;
    } catch (e) {
      this.ctl.notifyError(e);
      return false;
    }
  }

  /** ⇧F7: one more level of arrows into the active cell. */
  async tracePrecedents(): Promise<void> {
    await this.run("tracePrecedents");
  }

  /** Remove Precedents: the outermost level of precedent arrows. */
  async removePrecedents(): Promise<void> {
    await this.run("removePrecedents");
  }

  /** ⇧F5: one more level of arrows out of the active cell. */
  async traceDependents(): Promise<void> {
    await this.run("traceDependents");
  }

  /** Remove Dependents: the outermost level of dependent arrows. */
  async removeDependents(): Promise<void> {
    await this.run("removeDependents");
  }

  /** Trace Error: arrows down to where the active cell's error starts. */
  async traceError(): Promise<void> {
    await this.run("traceError");
  }

  /** Refresh Traces: every arrow redrawn from the operation list. */
  async refreshTraces(): Promise<void> {
    await this.run("refresh");
  }

  /** Tools ▸ Detective ▸ Remove All Traces: the sheet's arrows and circles, and the operation list. */
  async removeTraces(): Promise<void> {
    const circles = useW5Store.getState().invalidCells.length > 0;
    useW5Store.getState().setInvalidCells([]);
    try {
      const doc = this.ctl.requireDoc();
      const sheet = this.ctl.sheet;
      // A refresh still in flight must not redraw the removed arrows.
      const seq = ++this.seq;
      const st = await clientV4(this.ctl).detective(doc, sheet, 0, 0, "removeAll");
      this.noteDirty(doc, st.dirty);
      if (seq === this.seq && this.ctl.sheet === sheet) this.setArrows(st.arrows);
      if (!st.done && !circles) this.toast("info", nothingMessage("removeAll"));
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  /** Tools ▸ Detective ▸ AutoRefresh (an application option, on by default as in AOO). */
  toggleAutoRefresh(): void {
    const p = useW56Store.getState().prefs;
    const on = !p.detectiveAuto;
    void this.ctl.w56.savePrefs({ ...p, detectiveAuto: on });
    this.toast("info", on ? t("detective.autoRefreshOn", "Detective AutoRefresh is on.") : t("detective.autoRefreshOff", "Detective AutoRefresh is off."));
  }

  /** Whether AutoRefresh is on. */
  get autoRefresh(): boolean {
    return useW56Store.getState().prefs.detectiveAuto;
  }

  /** Sends the AutoRefresh option to the engine for this document. */
  private async syncAuto(): Promise<void> {
    const doc = this.ctl.info?.doc;
    if (doc === undefined) return;
    this.syncedDoc = doc;
    try {
      await clientV4(this.ctl).setDetectiveAuto(doc, useW56Store.getState().prefs.detectiveAuto);
    } catch {
      // An engine without the detective: nothing to keep in sync.
    }
  }

  /** Re-reads the shown sheet's arrows. */
  async refreshArrows(): Promise<void> {
    const doc = this.ctl.info?.doc;
    if (doc === undefined) return;
    const sheet = this.ctl.sheet;
    const seq = ++this.seq;
    try {
      const st = await clientV4(this.ctl).detectiveArrows(doc, sheet);
      if (seq === this.seq && this.ctl.sheet === sheet && this.ctl.info?.doc === doc) this.setArrows(st.arrows);
    } catch {
      // Unsupported (older engines): no arrows.
    }
  }

  /** A document was loaded or the shown sheet changed. */
  onSheet(): void {
    const doc = this.ctl.info?.doc;
    if (doc !== undefined && doc !== this.syncedDoc) void this.syncAuto();
    void this.refreshArrows();
  }

  /** After an engine result: AutoRefresh or a row/column change moved the arrows. */
  afterResult(changed: boolean | null | undefined): void {
    if (changed) void this.refreshArrows();
  }

  // -- Fill Mode (AOO `ScAuditingShell`, `SID_DETECTIVE_FILLMODE`) -----------------------

  /** Ends Fill Mode (editing starts, another document loads). */
  endFillMode(): void {
    if (useW5Store.getState().detectiveFill) useW5Store.getState().setDetectiveFill(false);
  }

  /** Tools ▸ Detective ▸ Fill Mode: on (a click on a cell traces its precedents) or off. */
  toggleFillMode(): void {
    const on = !useW5Store.getState().detectiveFill;
    useW5Store.getState().setDetectiveFill(on);
    if (on) this.toast("info", t("detective.fillModeOn", "Fill mode: click a cell to trace its precedents. Press Esc to end."));
  }

  /** The grid's `onDetectiveFill`: a click on a cell, or `null` to end the mode (Esc, a click outside the cells). */
  onFill = (cell: CellPos | null): void => {
    if (cell === null) {
      useW5Store.getState().setDetectiveFill(false);
      return;
    }
    // AOO `SID_FILL_SELECT`: move the cursor there, then Trace Precedents.
    useSelectionStore.getState().setSelection(cellSelection(cell));
    void this.run("tracePrecedents", cell);
  };

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
