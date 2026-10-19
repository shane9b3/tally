/**
 * @tally/app: the React application chrome and zustand stores. All engine
 * access goes through an `EngineClient` from @tally/bridge.
 */
export { App, type AppProps, type Platform } from "./App";
export { AppController, writeClipboard, type ControllerOptions } from "./controller";
export { buildCommands, COMING_SOON, type CommandUi } from "./commands";
export { CommandRegistry, type Command } from "./registry";
export { BINDINGS, resolveKey, resolveBinding, shortcutFor, type Binding, type KeyContext, type KeyLike } from "./keymap";
export { pretty } from "./pretty";
export { detectSumRange, sumFormula, sumScanRanges, rangeAddress } from "./sumRange";
export { CURRENCY_FORMAT, PERCENT_FORMAT, ZOOM_PRESETS } from "./formats";
export { createBrowserShell, parseCsv, EMPTY_PATCH, type BrowserTemplate, type ShellApi, type TemplateInfo } from "./shell";
export { useDocStore, reduceResult, type DocState } from "./stores/docStore";
export { useSelectionStore, toSelectionDto, fromSelectionDto } from "./stores/selectionStore";
export { useEditStore, type EditOrigin } from "./stores/editStore";
export { useUiStore, type Toast } from "./stores/uiStore";
