/**
 * ARIA mirror of the canvas grid (wave 5, §4 item 46; W5-0b lands it fully).
 *
 * The canvas draws thousands of cells, so the accessibility tree mirrors only
 * what a screen reader needs:
 *
 * - the grid root is `role="grid"` with `aria-rowcount` / `aria-colcount`
 *   (the sheet's limits), `aria-multiselectable` and an `aria-label`
 *   naming the sheet;
 * - an offscreen `role="row"` › `role="gridcell"` node describes the active
 *   cell (address, value, formula; `aria-rowindex` / `aria-colindex` are
 *   1-based) and the focused textarea points at it with
 *   `aria-activedescendant`;
 * - a polite live region announces "address, value, formula" (and the range
 *   size) whenever the selection or the active cell's content changes.
 *
 * `docs/qa/a11y.md` is the VoiceOver walkthrough; `tests/e2e/a11y.spec.ts`
 * checks the tree with Playwright's aria snapshots.
 */
import type { CellView } from "@tally/bridge";
import { cellAddress } from "./colLetters";
import type { Selection } from "./types";

/** What the app feeds the mirror (`GridProps.aria`). */
export interface AriaProps {
  /** The sheet's name (`aria-label`: "Sheet Budget"); `null` keeps "Spreadsheet". */
  sheetName?: string | null;
  /**
   * The active cell's input when it is a formula (`=A1*2`), from the app's
   * edit info; the grid itself only has the displayed text.
   */
  activeFormula?: string | null;
  /** Turns announcements off (e.g. while a dialog owns the focus). */
  silent?: boolean;
}

/** Visually hidden but readable by assistive technology. */
export const OFFSCREEN_STYLE: Partial<CSSStyleDeclaration> = {
  position: "absolute",
  width: "1px",
  height: "1px",
  margin: "-1px",
  padding: "0",
  overflow: "hidden",
  clip: "rect(0 0 0 0)",
  whiteSpace: "nowrap",
  border: "0",
  pointerEvents: "none",
};

/** The text announced for a cell: "B2: 42" / "B2: empty" / "C1: 6, formula =A1*B1". */
export function describeCell(address: string, text: string | undefined, formula: string | null | undefined): string {
  const value = text === undefined || text === "" ? "empty" : text;
  const f = formula && formula.startsWith("=") ? `, formula ${formula}` : "";
  return `${address}: ${value}${f}`;
}

/** The announcement for a selection: the active cell, then the range when it spans more than one cell. */
export function describeSelection(sel: Selection, text: string | undefined, formula: string | null | undefined): string {
  const { range: r, active } = sel;
  let s = describeCell(cellAddress(active.r, active.c), text, formula);
  const rows = r.r1 - r.r0 + 1;
  const cols = r.c1 - r.c0 + 1;
  if (rows > 1 || cols > 1) {
    s += `, selected ${cellAddress(r.r0, r.c0)} to ${cellAddress(r.r1, r.c1)} (${rows} rows, ${cols} columns)`;
  }
  const extra = sel.others?.length ?? 0;
  if (extra > 0) s += `, plus ${extra} more ${extra === 1 ? "range" : "ranges"}`;
  return s;
}

let nextId = 1;

export class AriaMirror {
  /** The offscreen row holding the active-cell description. */
  readonly row: HTMLDivElement;
  /** The active-cell node (`aria-activedescendant` target). */
  readonly cell: HTMLDivElement;
  /** The polite live region. */
  readonly live: HTMLDivElement;
  private lastAnnounced = "";
  private props: AriaProps = {};

  constructor(
    doc: Document,
    private readonly root: HTMLElement,
    private readonly focusTarget: HTMLElement,
    limits: { rows: number; cols: number },
  ) {
    const id = `tally-grid-active-${nextId++}`;
    root.setAttribute("role", "grid");
    root.setAttribute("aria-rowcount", String(limits.rows));
    root.setAttribute("aria-colcount", String(limits.cols));
    root.setAttribute("aria-multiselectable", "true");
    root.setAttribute("aria-label", "Spreadsheet");

    const row = doc.createElement("div");
    row.setAttribute("role", "row");
    Object.assign(row.style, OFFSCREEN_STYLE);
    const cell = doc.createElement("div");
    cell.id = id;
    cell.setAttribute("role", "gridcell");
    cell.setAttribute("aria-selected", "true");
    cell.dataset.testid = "aria-active-cell";
    row.appendChild(cell);

    const live = doc.createElement("div");
    live.setAttribute("role", "status");
    live.setAttribute("aria-live", "polite");
    live.setAttribute("aria-atomic", "true");
    live.dataset.testid = "aria-live";
    Object.assign(live.style, OFFSCREEN_STYLE);

    root.append(row, live);
    this.row = row;
    this.cell = cell;
    this.live = live;
    focusTarget.setAttribute("aria-activedescendant", id);
    root.setAttribute("aria-activedescendant", id);
  }

  setProps(props: AriaProps): void {
    this.props = props;
    this.root.setAttribute("aria-label", props.sheetName ? `Sheet ${props.sheetName}` : "Spreadsheet");
  }

  /**
   * Mirrors the selection: updates the active-cell node and announces the
   * change when the description differs from the last one (`force` repeats
   * it, e.g. after a sheet switch).
   */
  update(sel: Selection, view: CellView | undefined, force = false): void {
    const { active } = sel;
    const formula = this.props.activeFormula ?? null;
    const address = cellAddress(active.r, active.c);
    this.row.setAttribute("aria-rowindex", String(active.r + 1));
    this.cell.setAttribute("aria-colindex", String(active.c + 1));
    this.cell.setAttribute("aria-rowindex", String(active.r + 1));
    this.cell.dataset.address = address;
    this.cell.textContent = describeCell(address, view?.text, formula);
    const text = describeSelection(sel, view?.text, formula);
    if (!force && text === this.lastAnnounced) return;
    this.lastAnnounced = text;
    if (this.props.silent) return;
    // Re-setting identical text does not re-announce; clear first.
    this.live.textContent = "";
    this.live.textContent = text;
  }

  /** The last announcement (tests). */
  get announcement(): string {
    return this.live.textContent ?? "";
  }

  destroy(): void {
    this.row.remove();
    this.live.remove();
    this.focusTarget.removeAttribute("aria-activedescendant");
    for (const a of ["role", "aria-rowcount", "aria-colcount", "aria-multiselectable", "aria-label", "aria-activedescendant"]) {
      this.root.removeAttribute(a);
    }
  }
}
