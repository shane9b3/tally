/**
 * Public types of @tally/grid (docs/architecture-plan.md §7 WP6).
 *
 * The grid is a view: it draws what a {@link GridDataSource} has, reports
 * user intent through {@link GridEvents} and never talks to the engine.
 */
import type { CellPos, CellView, RangeDto, SheetLayout, StyleDto } from "@tally/bridge";
import type { Theme } from "@tally/tokens";

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
}

/** The visible cells (inclusive), plus pixel offsets of the scroll position. */
export interface Viewport extends RangeDto {
  scrollX: number;
  scrollY: number;
  width: number;
  height: number;
}
