/**
 * @tally/grid: canvas renderer, virtualization, tile cache, hit testing,
 * selection model, keyboard actions and the in-cell editor host.
 *
 * Framework-agnostic; the React wrapper is `@tally/grid/react`. No module
 * touches the DOM at import time.
 */
export { Grid, type GridOptions } from "./grid";
export * from "./types";
export { AxisModel, ScrollAxisMap, MAX_SCROLL_EXTENT } from "./layout";
export { TileTracker, TILE_ROWS, TILE_COLS, MAX_TILES_PER_BATCH, tileRange, intersects } from "./tiles";
export { fitNumber, generalText, isGeneralNumberText } from "./fitNumber";
export { colToLetters, lettersToCol, cellAddress } from "./colLetters";
export {
  reduceKey,
  cellSelection,
  extendTo,
  selectRows,
  selectCols,
  selectAll,
  stepWithin,
  rangeOf,
  sameSelection,
  type KeyAction,
  type Limits,
  type Mods,
} from "./selection";
export { Geometry, HEADER_HEIGHT } from "./geometry";
export { EngineClientSource, type EngineClientSourceOptions } from "./engineSource";
export { SyntheticSource, type SyntheticOptions } from "./synthetic";
