/**
 * Public types of @tally/grid (docs/architecture-plan.md §7 WP6).
 *
 * The grid is a view: it draws what a {@link GridDataSource} has, reports
 * user intent through {@link GridEvents} and never talks to the engine.
 */
import type { CellPos, CellView, RangeDto, SheetLayout, StyleDto } from "@tally/bridge";
import type { Theme } from "@tally/tokens";
import type { AriaProps } from "./aria";
import type { GridObject, ObjectHandle } from "./objectLayer";

export type { CellPos, CellView, RangeDto, SheetLayout, StyleDto };

/** Grid limits, matching `tally_types::{MAX_ROWS, MAX_COLS}`. */
export const MAX_ROWS = 1_048_576;
export const MAX_COLS = 16_384;

/** Arrow direction (same strings as the bridge `Direction`). */
export type Dir = "up" | "down" | "left" | "right";

/** Where the selection moves after an edit is committed. */
export type CommitMove = Dir | "none";

/**
 * The selection. `range` is the current (last added) range, which holds
 * `active`; `anchor` is where Shift-extension starts. Cmd/Ctrl+click adds
 * ranges: the earlier ones are kept in `others` (oldest first).
 */
export interface Selection {
  range: RangeDto;
  active: CellPos;
  anchor: CellPos;
  /** Earlier ranges of a multi-range selection (absent or empty for one range). */
  others?: RangeDto[];
}

/** A coloured reference outline drawn while editing a formula. */
export interface RefHighlight {
  range: RangeDto;
  /** CSS colour of the outline (the tint is derived from it). */
  color: string;
}

/** A coloured span of the editor text (UTF-16 offsets, end exclusive). */
export interface TextHighlight {
  start: number;
  end: number;
  color: string;
}

/** Point mode: the phase of a reference being picked with the mouse or keys. */
export type PointPhase = "start" | "move" | "end";

/** Which part of the grid was right-clicked. */
export type ContextArea = "cell" | "rowHeader" | "colHeader" | "corner";

/** An in-progress cell edit, owned by the app (shared with the formula bar). */
export interface EditState {
  cell: CellPos;
  text: string;
  caret: number;
  /**
   * Bumped by the app for programmatic changes (point mode, autocomplete,
   * editor undo): the editor then always takes `text`/`caret`, even if it
   * looks like an echo of something it typed earlier.
   */
  seq?: number;
}

/**
 * AutoFilter buttons: the ▾ drawn in each header cell (`row`, columns
 * `c0..=c1`); `active` columns have a filter set and are highlighted.
 */
export interface FilterButtons {
  row: number;
  c0: number;
  c1: number;
  active: number[];
}

/**
 * Page-break lines ("View ▸ Page breaks"): automatic breaks (light dashed)
 * and manual ones (bold dashed), above row `at` / left of column `at`.
 */
export interface PageBreaks {
  rows: { at: number; manual: boolean }[];
  cols: { at: number; manual: boolean }[];
}

/** One outline group (rows or columns `start..=end` at `level` 1–8). */
export interface OutlineGroup {
  start: number;
  end: number;
  level: number;
  collapsed: boolean;
}

/**
 * Outline gutter (wave 5 seam; Data ▸ Group and Outline): the groups of both
 * axes. The grid reserves `levelPx` (zoom 100%) per level left of the row
 * header / above the column header, draws level buttons in the corner and a
 * +/− box at each group's first row/column, and reports clicks through
 * `onOutlineToggle` / `onOutlineLevel`.
 */
export interface OutlineProps {
  rows: OutlineGroup[];
  cols: OutlineGroup[];
  /** Gutter width per level at zoom 100% (default 13). */
  levelPx?: number;
}

/** A detective arrow (Tools ▸ Detective) between two cells of the sheet. */
export interface TraceArrow {
  from: CellPos;
  to: CellPos;
  kind?: "precedent" | "dependent" | "error";
}

/** Screen geometry the app reads to place HTML over the canvas (CSS px). */
export interface GridGeometryInfo {
  /** Screen x of column `c`'s left edge (frozen columns do not scroll). */
  colLeft(c: number): number;
  /** Screen y of row `r`'s top edge. */
  rowTop(r: number): number;
  headerW: number;
  headerH: number;
  /** Outline gutter sizes included in `headerW` / `headerH`. */
  gutterW: number;
  gutterH: number;
  scrollX: number;
  scrollY: number;
  frozen: { rows: number; cols: number };
  zoom: number;
  width: number;
  height: number;
}

/** A clickable decoration inside a cell: the validation ▾ or the link ↗. */
export type CellButtonKind = "dropdown" | "link";

/** Client rect (px) of something the grid drew, to anchor a popover. */
export interface ClientRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** CellView flags (docs/api.md): formula, error, array, note, list dropdown, link. */
export const CELL_FLAGS = { formula: 1, error: 2, array: 4, note: 8, dropdown: 16, link: 32 } as const;

/** What a data-source notification covers. */
export type DirtySet = RangeDto[] | "all";

/**
 * What the grid reads. All methods are synchronous and must be cheap: the
 * grid calls `getCell` for every visible cell on every frame.
 *
 * The data source is usually asynchronous underneath (an engine over IPC):
 * `ensureRanges` starts fetching, `getCell` returns what is already there
 * (`undefined` = empty or not loaded yet) and `subscribe` listeners are told
 * when something arrived or changed, after which the grid repaints.
 *
 * Contract for notifications: `fn(ranges)` means "these cells may look
 * different now"; the grid repaints and calls `ensureRanges` again for
 * visible ranges, so `ensureRanges` must be idempotent (a no-op for data that
 * is present and fresh). `fn("all")` also makes the grid re-read
 * `getLayout`.
 */
export interface GridDataSource {
  /** Row/column geometry; return the same object until it changes. */
  getLayout(sheet: number): SheetLayout;
  /** The cell at (r, c), or `undefined` if empty or not loaded. */
  getCell(r: number, c: number): CellView | undefined;
  /** A resolved style, or `undefined` if not loaded (drawn as default). */
  getStyle(id: number): StyleDto | undefined;
  /** Requests that these ranges be loaded (tile-aligned, at most 8 per call). */
  ensureRanges(ranges: RangeDto[]): void;
  /** Change notifications; returns an unsubscribe function. */
  subscribe(fn: (dirty: DirtySet) => void): () => void;
}

/** Callbacks from the grid to the app. All are optional. */
export interface GridEvents {
  /** The user moved the selection (the grid already shows it). */
  onSelectionChange(sel: Selection): void;
  /**
   * The user started editing `cell`. With `seed` (typing, IME), the editor
   * replaces the content with `seed`; without (F2, double-click), the app
   * should set `edit` to the cell's input text (e.g. `cellEditInfo().input`).
   */
  onStartEdit(cell: CellPos, seed?: string): void;
  /** The editor text changed. */
  onEditChange(text: string, caret: number): void;
  /** Commit the edit; the grid moves the selection by `move` afterwards. */
  onCommitEdit(move: CommitMove): void;
  /** Discard the edit. */
  onCancelEdit(): void;
  /** Column or row size set by dragging a header gutter. */
  onResize(axis: "col" | "row", start: number, end: number, px: number): void;
  /** Double-click on a column header gutter: fit columns `start..=end` to their contents. */
  onAutoFit(axis: "col", start: number, end: number): void;
  /**
   * The fill handle was dragged. `source` is the selection range; `target`
   * is the whole filled range INCLUDING `source` (it extends `source` down,
   * up, right or left). The grid selects `target` afterwards.
   */
  onFill(source: RangeDto, target: RangeDto): void;
  /** Double-click on the fill handle (fill down to the adjacent data's end). */
  onFillHandleDoubleClick(source: RangeDto): void;
  /**
   * Point mode (only while `pointMode` is set): the user picked `range` with
   * a click/drag or an arrow key in the editor. The app inserts or replaces
   * the reference at the caret.
   */
  onPointRange(range: RangeDto, phase: PointPhase): void;
  /**
   * Called first for every key pressed in the open in-cell editor. Return
   * true if the app handled it (the grid then does nothing else). Used for
   * autocomplete navigation and editor-local undo.
   */
  onEditorKeyDown(e: KeyboardEvent): boolean;
  /** Ctrl/Cmd+arrow: resolve the data edge (engine `navigateEdge`). */
  onNavigateEdge(from: CellPos, dir: Dir): Promise<CellPos>;
  /** Right click (selection already updated). Coordinates are client px. */
  onContextMenu(cell: CellPos, x: number, y: number, area: ContextArea): void;
  /** Delete/Backspace outside the editor: clear the selection's contents. */
  onDeleteContents(range: RangeDto): void;
  /**
   * DOM `copy`/`cut` outside the editor. The default is already prevented;
   * write synchronously to `data` if you can, or use the async Clipboard API.
   */
  onCopy(data: DataTransfer | null): void;
  onCut(data: DataTransfer | null): void;
  /**
   * DOM `paste` outside the editor (default prevented). `plain` is set for
   * ⇧⌘V / Ctrl+Shift+V (paste values only).
   */
  onPaste(data: DataTransfer | null, opts?: { plain: boolean }): void;
  /**
   * An AutoFilter ▾ button was clicked: column `col`, and the button's
   * client rect (to anchor a popover).
   */
  onFilterButton(col: number, rect: { left: number; top: number; right: number; bottom: number }): void;
  /**
   * The validation ▾ of a list cell (flag 16) or the link ↗ of the active
   * link cell (flag 32) was clicked. The cell is already selected.
   */
  onCellButton(kind: CellButtonKind, cell: CellPos, rect: ClientRect): void;
  /** ⌘/Ctrl-click on a cell with a link (flag 32): open it (instead of adding a range). */
  onOpenLink(cell: CellPos, url: string): void;
  /**
   * The pointer rests on a cell with a note (flag 8), or left it (`null`).
   * `rect` is the cell's client rect.
   */
  onNoteHover(cell: CellPos | null, rect: ClientRect | null): void;
  // -- wave 5 seams (W5-0b) ---------------------------------------------------
  /** The visible range or scroll position changed (after layout, scroll or zoom). */
  onViewportChange(vp: Viewport): void;
  /** A +/− box in the outline gutter was clicked. */
  onOutlineToggle(axis: "rows" | "cols", group: OutlineGroup): void;
  /** A level button (1-based) in the outline corner was clicked. */
  onOutlineLevel(axis: "rows" | "cols", level: number): void;
  /** The selected object (chart/picture) changed; `null` when a cell was clicked. */
  onObjectSelect(id: string | null): void;
  /** Pointer down on an object's body or a resize handle (W5-5 starts the drag). */
  onObjectPointerDown(id: string, part: "body" | ObjectHandle, e: PointerEvent): void;
  /** Double-click on an object (open its editor). */
  onObjectDoubleClick(id: string): void;
  /** Delete / Backspace / Escape / arrows while an object is selected. */
  onObjectKey(id: string, key: string, e: KeyboardEvent): void;
  /**
   * Page Break Preview (W5-5): a break line was dragged to a new row/column
   * (`from` is where it was, `to` where it was dropped; `manual` says
   * whether the dragged line was a manual break). The app issues
   * `setPageBreak` ops.
   */
  onPageBreakDrag(axis: "row" | "col", from: number, to: number, manual: boolean): void;
}

/** Everything the grid is configured with. */
export interface GridProps extends Partial<GridEvents> {
  source: GridDataSource;
  /** Sheet id passed to `source.getLayout`. */
  sheet: number;
  theme: Theme;
  /** 1 = 100%. */
  zoom: number;
  selection: Selection;
  /** The edit in progress, or `null`. */
  edit: EditState | null;
  /**
   * While editing a formula with the caret where a reference may go: clicks
   * on cells (and, in "enter" mode, arrow keys) pick references
   * (`onPointRange`) instead of committing.
   */
  pointMode?: boolean;
  /** Coloured outlines of the references in the formula being edited. */
  refHighlights?: RefHighlight[];
  /** Coloured spans of the editor text (formula references). */
  editHighlights?: TextHighlight[];
  /** The array-formula block containing the active cell (drawn as a thin outline). */
  arrayRange?: RangeDto | null;
  /** A copied or cut range (dashed outline until the next paste or Esc). */
  clipRange?: RangeDto | null;
  /** AutoFilter header buttons, or `null` without an AutoFilter. */
  filterButtons?: FilterButtons | null;
  /** Page-break lines to draw, or `null` (the "Page breaks" view is off). */
  pageBreaks?: PageBreaks | null;
  /** The print range, outlined (with the page breaks). */
  printRange?: RangeDto | null;
  // -- wave 5 seams (W5-0b) ---------------------------------------------------
  /**
   * Range-pick mode without an open edit (dialogs asking for a range):
   * clicks and drags on cells, and arrow keys, report `onPointRange` instead
   * of changing the selection for good. `Grid.pickRange()` wraps it in a
   * promise.
   */
  rangePick?: boolean;
  /** Outline groups and the gutter, or `null`. */
  outline?: OutlineProps | null;
  /** Page Break Preview: a "Page N" watermark per page (needs `pageBreaks`). */
  pageBreakPreview?: boolean;
  /** Detective arrows to draw, or `null`. */
  traces?: TraceArrow[] | null;
  /** The ARIA mirror's inputs (sheet name, the active cell's formula). */
  aria?: AriaProps | null;
  /** Cell-anchored objects (charts, pictures) for the overlay layer, or `null`. */
  objects?: GridObject[] | null;
  /** Detective "Mark invalid data": cells circled in red (W5-5). */
  invalidCells?: CellPos[] | null;
}

/** The visible cells (inclusive), plus pixel offsets of the scroll position. */
export interface Viewport extends RangeDto {
  scrollX: number;
  scrollY: number;
  width: number;
  height: number;
}
