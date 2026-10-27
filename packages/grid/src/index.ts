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
  allRanges,
  addRange,
  moveBy,
  type KeyAction,
  type Limits,
  type Mods,
} from "./selection";
export { Geometry, HEADER_HEIGHT } from "./geometry";
export { fillTarget, fillDirection, type FillDir } from "./fillHandle";
export { wrapLines, luminance, textColor, cssColor, drawStyleId, cssFontFamily, cleanFamily, fontSubstitutes } from "./renderer";
export { drawCfIcon, drawDataBar, iconCount, ICON_COLORS } from "./cfIcons";
export { cellsOptionsActive, EngineClientSource, type EngineClientSourceOptions } from "./engineSource";
// Wave 5 seams (W5-0b)
export { ObjectLayer, anchorToPx, pxToAnchor, anchorRange, PX_PER_MM, OBJECT_HANDLES, type GridObject, type ObjectAnchor, type ObjectHandle, type AnchorAxes, type ObjectLayerEvents } from "./objectLayer";
export { AriaMirror, describeCell, describeSelection, DEFAULT_GRID_STRINGS, gridStrings, OFFSCREEN_STYLE, type AriaProps, type GridStrings } from "./aria";
export { SplitView, clampRatio, type SplitAxis, type SplitOptions } from "./split";
export { OUTLINE_LEVEL_PX, outlineLevels, outlineHit, type OutlineHit } from "./outline";
export { SyntheticSource, type SyntheticOptions } from "./synthetic";
