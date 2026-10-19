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

/** Single-range selection (Phase A). `anchor` is where Shift-extension starts. */
export interface Selection {
  range: RangeDto;
  active: CellPos;
  anchor: CellPos;
}

/** An in-progress cell edit, owned by the app (shared with the formula bar). */
export interface EditState {
  cell: CellPos;
  text: string;
  caret: number;
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
  /** Column (Phase A) or row size set by dragging a header gutter. */
  onResize(axis: "col" | "row", start: number, end: number, px: number): void;
  /** Ctrl/Cmd+arrow: resolve the data edge (engine `navigateEdge`). */
  onNavigateEdge(from: CellPos, dir: Dir): Promise<CellPos>;
  /** Right click (selection already updated). Coordinates are client px. */
  onContextMenu(cell: CellPos, x: number, y: number): void;
  /** Delete/Backspace outside the editor: clear the selection's contents. */
  onDeleteContents(range: RangeDto): void;
  /**
   * DOM `copy`/`cut` outside the editor. The default is already prevented;
   * write synchronously to `data` if you can, or use the async Clipboard API.
   */
  onCopy(data: DataTransfer | null): void;
  onCut(data: DataTransfer | null): void;
  /** DOM `paste` outside the editor (default prevented). */
  onPaste(data: DataTransfer | null): void;
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
}

/** The visible cells (inclusive), plus pixel offsets of the scroll position. */
export interface Viewport extends RangeDto {
  scrollX: number;
  scrollY: number;
  width: number;
  height: number;
}
