/**
 * ARIA mirror of the canvas grid (wave 5, §4 item 46; W5-0b lands it fully).
 *
 * The canvas draws thousands of cells, so the accessibility tree mirrors only
 * what a screen reader needs:
 *
 * - an offscreen `role="grid"` node inside the grid root carries
 *   `aria-rowcount` / `aria-colcount` (the sheet's limits),
 *   `aria-multiselectable` and an `aria-label` naming the sheet; it holds
 *   only rows (ARIA: a grid owns rows), so the cell editor and the live
 *   region sit next to it in the root;
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

/**
 * The grid's screen-reader strings. The grid never imports the app's i18n;
 * the app passes translated strings through `AriaProps.strings` (any subset;
 * the rest stay English). `{name}` placeholders are filled by the grid.
 */
export interface GridStrings {
  /** `aria-label` of the grid when no sheet name is known. */
  spreadsheet: string;
  /** `aria-label` of the grid: `{name}` is the sheet's name. */
  sheet: string;
  /** Value announced for a blank cell. */
  empty: string;
  /** Appended for a formula cell: `{formula}`. */
  formula: string;
  /** Appended for a multi-cell selection: `{from}`, `{to}`, `{rows}`, `{cols}`. */
  selected: string;
  /** Appended for extra ranges: `{count}` (1). */
  plusRange: string;
  /** Appended for extra ranges: `{count}` (2 or more). */
  plusRanges: string;
  /** `aria-label` of the cell editor textarea. */
  cellEditor: string;
  /** `aria-label` of the drawing-object layer. */
  objects: string;
  /** Appended for a push-button cell (the DataPilot Filter button). */
  cellButton: string;
}

export const DEFAULT_GRID_STRINGS: GridStrings = {
  spreadsheet: "Spreadsheet",
  sheet: "Sheet {name}",
  empty: "empty",
  formula: ", formula {formula}",
  selected: ", selected {from} to {to} ({rows} rows, {cols} columns)",
  plusRange: ", plus {count} more range",
  plusRanges: ", plus {count} more ranges",
  cellEditor: "Cell editor",
  objects: "Objects",
  cellButton: ", button, Alt+Down presses it",
};

/** Fills `{name}` placeholders. */
function fill(template: string, params: Readonly<Record<string, string | number>>): string {
  return template.replace(/\{(\w+)\}/g, (m, k: string) => (k in params ? String(params[k]) : m));
}

/** Merges a partial table over the English defaults. */
export function gridStrings(partial?: Partial<GridStrings> | null): GridStrings {
  return partial ? { ...DEFAULT_GRID_STRINGS, ...partial } : DEFAULT_GRID_STRINGS;
}

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
  /** Translated strings (any subset); English when absent. */
  strings?: Partial<GridStrings>;
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
export function describeCell(
  address: string,
  text: string | undefined,
  formula: string | null | undefined,
  strings: Partial<GridStrings> = DEFAULT_GRID_STRINGS,
  button = false,
): string {
  const S = gridStrings(strings);
  const value = text === undefined || text === "" ? S.empty : text;
  const f = formula && formula.startsWith("=") ? fill(S.formula, { formula }) : "";
  return `${address}: ${value}${f}${button ? S.cellButton : ""}`;
}

/** The announcement for a selection: the active cell, then the range when it spans more than one cell. */
export function describeSelection(
  sel: Selection,
  text: string | undefined,
  formula: string | null | undefined,
  strings: Partial<GridStrings> = DEFAULT_GRID_STRINGS,
  button = false,
): string {
  const S = gridStrings(strings);
  const { range: r, active } = sel;
  let s = describeCell(cellAddress(active.r, active.c), text, formula, S, button);
  const rows = r.r1 - r.r0 + 1;
  const cols = r.c1 - r.c0 + 1;
  if (rows > 1 || cols > 1) {
    s += fill(S.selected, { from: cellAddress(r.r0, r.c0), to: cellAddress(r.r1, r.c1), rows, cols });
  }
  const extra = sel.others?.length ?? 0;
  if (extra > 0) s += fill(extra === 1 ? S.plusRange : S.plusRanges, { count: extra });
  return s;
}

let nextId = 1;

export class AriaMirror {
  /** The offscreen `role="grid"` node (named after the sheet). */
  readonly grid: HTMLDivElement;
  /** The offscreen row holding the active-cell description. */
  readonly row: HTMLDivElement;
  /** The active-cell node (`aria-activedescendant` target). */
  readonly cell: HTMLDivElement;
  /** The polite live region. */
  readonly live: HTMLDivElement;
  private lastAnnounced = "";
  private props: AriaProps = {};
  private last: { sel: Selection; view: CellView | undefined } | null = null;
  /** Called with the merged strings whenever props change (the grid relabels the editor and object layer). */
  onStrings?: (strings: GridStrings) => void;
  /** Whether a cell is a push button (described as one). */
  isButton?: (r: number, c: number) => boolean;

  constructor(
    doc: Document,
    private readonly root: HTMLElement,
    private readonly focusTarget: HTMLElement,
    limits: { rows: number; cols: number },
  ) {
    const id = `tally-grid-active-${nextId++}`;
    const grid = doc.createElement("div");
    grid.setAttribute("role", "grid");
    grid.setAttribute("aria-rowcount", String(limits.rows));
    grid.setAttribute("aria-colcount", String(limits.cols));
    grid.setAttribute("aria-multiselectable", "true");
    grid.setAttribute("aria-label", DEFAULT_GRID_STRINGS.spreadsheet);
    grid.dataset.testid = "aria-grid";
    Object.assign(grid.style, OFFSCREEN_STYLE);

    const row = doc.createElement("div");
    row.setAttribute("role", "row");
    Object.assign(row.style, OFFSCREEN_STYLE);
    const cell = doc.createElement("div");
    cell.id = id;
    cell.setAttribute("role", "gridcell");
    cell.setAttribute("aria-selected", "true");
    cell.dataset.testid = "aria-active-cell";
    row.appendChild(cell);
    grid.appendChild(row);

    const live = doc.createElement("div");
    live.setAttribute("role", "status");
    live.setAttribute("aria-live", "polite");
    live.setAttribute("aria-atomic", "true");
    live.dataset.testid = "aria-live";
    Object.assign(live.style, OFFSCREEN_STYLE);

    root.append(grid, live);
    this.grid = grid;
    this.row = row;
    this.cell = cell;
    this.live = live;
    focusTarget.setAttribute("aria-activedescendant", id);
  }

  setProps(props: AriaProps): void {
    const stringsChanged = props.strings !== this.props.strings;
    this.props = props;
    const S = gridStrings(props.strings);
    this.grid.setAttribute("aria-label", props.sheetName ? fill(S.sheet, { name: props.sheetName }) : S.spreadsheet);
    this.onStrings?.(S);
    // A language switch: rewrite the active-cell node and the live text in the new language at once
    // (the live region is updated without re-announcing when the words did not change meaning).
    if (stringsChanged && this.last) {
      const { sel, view } = this.last;
      const formula = props.activeFormula ?? null;
      const button = this.isButton?.(sel.active.r, sel.active.c) ?? false;
      this.cell.textContent = describeCell(cellAddress(sel.active.r, sel.active.c), view?.text, formula, props.strings, button);
      const text = describeSelection(sel, view?.text, formula, props.strings, button);
      if (this.live.textContent !== "") this.live.textContent = text;
      this.lastAnnounced = text;
    }
  }

  /**
   * Mirrors the selection: updates the active-cell node and announces the
   * change when the description differs from the last one (`force` repeats
   * it, e.g. after a sheet switch).
   */
  update(sel: Selection, view: CellView | undefined, force = false): void {
    const { active } = sel;
    this.last = { sel, view };
    const formula = this.props.activeFormula ?? null;
    const address = cellAddress(active.r, active.c);
    this.row.setAttribute("aria-rowindex", String(active.r + 1));
    this.cell.setAttribute("aria-colindex", String(active.c + 1));
    this.cell.setAttribute("aria-rowindex", String(active.r + 1));
    this.cell.dataset.address = address;
    const button = this.isButton?.(active.r, active.c) ?? false;
    this.cell.textContent = describeCell(address, view?.text, formula, this.props.strings, button);
    const text = describeSelection(sel, view?.text, formula, this.props.strings, button);
    if (!force && text === this.lastAnnounced) return;
    this.lastAnnounced = text;
    if (this.props.silent) return;
    // Re-setting identical text does not re-announce; clear first.
    this.live.textContent = "";
    this.live.textContent = text;
  }

  /**
   * Announces `text` in the live region now (an app event the selection
   * does not describe, e.g. a formula edit showing another sheet).
   */
  announce(text: string): void {
    if (this.props.silent || text === "") return;
    this.lastAnnounced = text;
    this.live.textContent = "";
    this.live.textContent = text;
  }

  /** The last announcement (tests). */
  get announcement(): string {
    return this.live.textContent ?? "";
  }

  destroy(): void {
    this.grid.remove();
    this.live.remove();
    this.focusTarget.removeAttribute("aria-activedescendant");
  }
}
