/**
 * @tally/app: the React application chrome and zustand stores. All engine
 * access goes through an `EngineClient` (and shell services through a
 * `ShellClient`) from @tally/bridge.
 */
export { App, type AppProps, type Platform } from "./App";
export { AppController, writeClipboard, type ControllerOptions } from "./controller";
export { buildCommands, currencyTarget, CHART_SOON, COMING_SOON, FILTER_SOON, type CommandUi } from "./commands";
export { CommandRegistry, type Command, type CommandArgs } from "./registry";
export { BINDINGS, resolveKey, resolveBinding, shortcutFor, type Binding, type KeyContext, type KeyLike } from "./keymap";
export { pretty } from "./pretty";
export { detectSumRange, sumFormula, sumScanRanges, rangeAddress } from "./sumRange";
export { CURRENCY_FORMAT, EMPTY_PATCH, PERCENT_FORMAT, ZOOM_PRESETS } from "./formats";
export { columnIndex, parseA1Range } from "./a1";
export { useDocStore, reduceResult, type DocState } from "./stores/docStore";
export { useSelectionStore, toSelectionDto, fromSelectionDto, selectionRanges } from "./stores/selectionStore";
export { useEditStore, type EditOrigin } from "./stores/editStore";
export { useUiStore, type Toast } from "./stores/uiStore";
export { WebMockClient } from "./dev/webMock";
export { fillRange } from "./fill";
export { formulaHighlights, applyPoint, assistFor, signatureAt } from "./formulaEdit";
export { analyzeGlance, currentRegion } from "./glance";
export { formatNumberLite } from "./numfmtLite";
export { chipColorsFor, hexToRgba, rgbaToHex } from "./palette";
export { useFunctionsStore } from "./stores/functionsStore";
export { nextThemePref, resolveTheme, type ThemePref, type ResolvedTheme } from "./theme/theme";
