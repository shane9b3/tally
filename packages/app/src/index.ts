/**
 * @tally/app: the React application chrome and zustand stores. All engine
 * access goes through an `EngineClient` (and shell services through a
 * `ShellClient`) from @tally/bridge.
 */
export { App, type AppProps, type Platform } from "./App";
export { AppController, writeClipboard, type ControllerOptions } from "./controller";
export { buildCommands, currencyTarget, CHART_SOON, COMING_SOON, FILTER_SOON, WAVE5_SOON, type CommandUi } from "./commands";
export { t, setTranslations, translations, type Translations } from "./i18n";
export { CommandRegistry, type Command, type CommandArgs } from "./registry";
export { BINDINGS, isMacPlatform, platformBindings, resolveKey, resolveBinding, shortcutFor, type Binding, type KeyContext, type KeyLike } from "./keymap";
export { pretty } from "./pretty";
export { detectSumRange, sumFormula, sumScanRanges, rangeAddress } from "./sumRange";
export { CURRENCY_FORMAT, EMPTY_PATCH, PERCENT_FORMAT, ZOOM_PRESETS } from "./formats";
export { columnIndex, parseA1Range } from "./a1";
export { useDocStore, reduceResult, type DocState } from "./stores/docStore";
export { useSelectionStore, toSelectionDto, fromSelectionDto, selectionRanges } from "./stores/selectionStore";
export { useEditStore, type EditOrigin } from "./stores/editStore";
export { useUiStore, type Toast } from "./stores/uiStore";
export { guardDecision, isSequence, proceedAfter, useSavePromptStore, type CloseKind, type CloseShell, type SaveChoice } from "./unsaved";
export { canReuseWindow, type ReuseState, type ThemeChoice, type WindowShell } from "./windows";
export { allowedWhileReadOnly, displayTitle, readOnlyGate, ReadOnlyFeature, READ_ONLY_COMMANDS, useLockPromptStore } from "./readOnly";
export { canDetach, cycleTab, openDocuments, tabByNumber, useTabsStore, TabsFeature, type TabShell, type TabsState, type TabInfo } from "./tabs";
// `WebMockClient` is deliberately not exported here: the desktop bundle must
// not ship the dev mock (security review 3, MEDIUM-7). Dev/e2e builds load it
// through `@tally/app/dev` with a dynamic import.
export { fillRange } from "./fill";
export { formulaHighlights, applyPoint, assistFor, signatureAt } from "./formulaEdit";
export { analyzeGlance, currentRegion } from "./glance";
export { formatNumberLite } from "./numfmtLite";
export { chipColorsFor, hexToRgba, rgbaToHex } from "./palette";
export { useFunctionsStore } from "./stores/functionsStore";
export { nextThemePref, resolveTheme, type ThemePref, type ResolvedTheme } from "./theme/theme";
// Wave 5 (W5-6)
export { W56Features, type W56Shell } from "./data/w56";
export { useW56Store, type W56Dialog } from "./data/store";
export { applyAutoCorrect } from "./autocorrect/autocorrect";
export { completionFor, ghostSuffix } from "./autocorrect/autoinput";
export { DEFAULT_PREFS, parsePrefs, type AppPrefs } from "./options/prefs";
export { setUiLanguage, uiLanguage } from "./i18n/locales";
export { renderShortcutsDoc } from "./options/shortcutsDoc";
// Wave 5 (W5-5): charts, print, objects.
export type { PickedImage, W5Shell, WindowChrome } from "./objects/shell";
export { useChartStore, objectsFor, selectedChart, objectId, parseObjectId } from "./charts/chartStore";
export { toEchartsOption, AOO_PALETTE, percentStack, LIGHT_CHART_THEME, type ChartTheme, type EChartsOption } from "./charts/toEcharts";
export { renderChartSvg, chartSvgsFor } from "./charts/chartSvgs";
export { usePrintStore } from "./print/printStore";
export { useW5Store } from "./objects/w5Store";
export { W5Features } from "./objects/w5Features";
