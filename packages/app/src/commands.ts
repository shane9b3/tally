/**
 * Every toolbar, menu and keymap action as a CommandRegistry command.
 * Menu ids (apps/desktop/src-tauri/src/menu.rs) and assist action ids map
 * here one to one.
 */
import { openCustomize, toggleBar } from "./chrome/chromeActions";
import { columnIndex, parseA1Range } from "./a1";
import type { AppController } from "./controller";
import { CURRENCY_FORMAT, NUMBER_FORMATS, PERCENT_FORMAT } from "./formats";
import { shortcutFor } from "./keymap";
import type { Command, CommandArgs } from "./registry";
import type { RangeDto } from "@tally/bridge";
import { isGrouped, useDocStore } from "./stores/docStore";
import { useEditStore } from "./stores/editStore";
import { useUiStore } from "./stores/uiStore";
import { useDataStore } from "./stores/dataStore";
import { BORDER_PRESETS, borderLine, type BorderPreset } from "./dataOps";
import { useV3Store } from "./stores/v3Store";
import { HIGHLIGHT_PRESETS, UI_LOCALES } from "./v03";
import { t } from "./i18n";
import { useW56Store } from "./data/store";

/** English palette titles of the border presets (the translated titles are `msg.palette.borders.<id>`). */
const BORDER_TITLES: Record<BorderPreset, string> = {
  all: "Borders: all borders",
  outer: "Borders: outer borders",
  inner: "Borders: inner borders",
  top: "Borders: top border",
  bottom: "Borders: bottom border",
  left: "Borders: left border",
  right: "Borders: right border",
  none: "Borders: no borders",
};

export const COMING_SOON = "Coming soon";
/** Reason shown for the wave-5 commands registered as seams (W5-0b). */
export const WAVE5_SOON = "coming in wave 5";
export const FILTER_SOON = "Filtering is coming in v0.2";
export const CHART_SOON = "Charts are coming in v0.3";

/**
 * Target of `format.currency` from assist args: `range` (e.g. `F2:F15`, the
 * column body), else `column` (that column of the selected rows), else
 * `null` (use the selection).
 */
export function currencyTarget(args: CommandArgs | undefined, selection: RangeDto): RangeDto | null {
  if (!args) return null;
  const range = args.range ? parseA1Range(args.range) : null;
  if (range) return range;
  const col = args.column ? columnIndex(args.column) : null;
  return col === null ? null : { r0: selection.r0, c0: col, r1: selection.r1, c1: col };
}

/**
 * Commands that are off while several sheets are selected (AOO greys them
 * out under a multi-sheet selection: ScTabViewShell::GetStateTable and the
 * slots that test `GetSelectCount() > 1`). The toolbar, the menus, the
 * palette and the keymap all go through the registry, so this list is the
 * one guard; the cell context menu reads it too.
 */
export const GROUP_DISABLED_COMMANDS: readonly string[] = [
  // Sort
  "data.sortAsc",
  "data.sortDesc",
  "data.sort",
  "data.sortOptions",
  // Filters
  "data.filter",
  "filter.byColumn",
  "data.standardFilter",
  "data.advancedFilter",
  "data.resetFilter",
  "data.refreshFilter",
  "data.clearFilters",
  // Data tools
  "data.subtotals",
  "data.dataPilot",
  "data.pivotFilter",
  "data.consolidate",
  "data.textToColumns",
  "data.removeDuplicates",
  "data.multipleOperations",
  "format.autoFormat",
  "tools.scenarios",
  "tools.solver",
  "tools.goalSeek",
  // Rules and links edited by id on the active sheet (new CF / validity rules stay group edits)
  "format.colorByValue",
  "insert.hyperlink",
  "insert.removeHyperlink",
  // Objects
  "insert.chart",
  "chart.byColumn",
  "insert.image",
  // Names and database ranges
  "data.defineName",
  "data.defineDatabaseRange",
  "data.refreshRange",
  "data.createNamesFromLabels",
  // Sheets
  "sheet.insert",
  "sheet.rename",
];

/**
 * Wraps the {@link GROUP_DISABLED_COMMANDS} so they are disabled while
 * `grouped()`, with a reason saying why (their own reason otherwise).
 */
export function guardGroup(cmds: Command[], grouped: () => boolean = isGrouped): Command[] {
  const off = new Set(GROUP_DISABLED_COMMANDS);
  const why = t("msg.reason.sheetsGrouped", "Not available with several sheets selected");
  return cmds.map((c) => {
    if (!off.has(c.id)) return c;
    const own = c.enabled;
    const ownReason = Object.getOwnPropertyDescriptor(c, "reason");
    const guarded = copyCommand(c, { enabled: () => !grouped() && (own?.() ?? true) });
    Object.defineProperty(guarded, "reason", { enumerable: true, configurable: true, get: () => (grouped() ? why : ownReason?.get ? ownReason.get.call(c) : ownReason?.value) });
    return guarded;
  });
}

/** A copy of a command with `patch` applied, keeping getters (a computed `reason`) live. */
function copyCommand(c: Command, patch: Partial<Command>): Command {
  const out = Object.defineProperties({}, Object.getOwnPropertyDescriptors(c)) as Command;
  for (const [k, v] of Object.entries(patch)) Object.defineProperty(out, k, { value: v, enumerable: true, configurable: true, writable: true });
  return out;
}

/** Hooks the UI provides for commands that need a component (rename prompt, about). */
export interface CommandUi {
  renameActiveSheet(): void;
  showAbout(): void;
  /** Opens the Custom number format dialog. */
  customNumberFormat(): void;
  /** Opens the Row height / Column width dialog for the selection. */
  resize(axis: "row" | "col"): void;
}


export function buildCommands(ctl: AppController, ui: CommandUi): Command[] {
  const sc = (id: string): { shortcut?: string } => {
    const s = shortcutFor(id);
    return s === undefined ? {} : { shortcut: s };
  };
  const undo = useDocStore.getState;
  const editing = (): boolean => useEditStore.getState().edit !== null;
  const notEditing = { enabled: () => !editing(), reason: t("msg.reason.finishEditing", "Finish editing the cell first") };
  const soon = { enabled: () => false, reason: t("msg.reason.comingSoon", COMING_SOON) };
  const w5 = { enabled: () => false, reason: t("msg.reason.wave5", WAVE5_SOON) };
  const dialog = useUiStore.getState;
  const hasFilter = (): boolean => useDataStore.getState().filter !== null;
  const grouped = isGrouped;
  // W5-6E: every title goes through `t("cmd.<id>")` (the English literal stays the fallback).
  return guardGroup(localizeTitles([
    // Edit
    { id: "edit.undo", title: "Undo", kind: "Edit", keywords: ["revert", "back"], ...sc("edit.undo"), enabled: () => editing() || undo().undo.canUndo, run: ctl.undoAny },
    { id: "edit.redo", title: "Redo", kind: "Edit", keywords: ["again"], ...sc("edit.redo"), enabled: () => editing() || undo().undo.canRedo, run: ctl.redoAny },
    { id: "edit.copy", title: "Copy", kind: "Edit", shortcut: "⌘C", run: () => void ctl.copy() },
    { id: "edit.cut", title: "Cut", kind: "Edit", shortcut: "⌘X", run: ctl.cut },
    { id: "edit.paste", title: "Paste", kind: "Edit", shortcut: "⌘V", run: () => ctl.pasteFromClipboard(false) },
    { id: "edit.pasteValues", title: "Paste values only", kind: "Edit", keywords: ["special", "plain", "text"], shortcut: "⇧⌘V", run: () => ctl.pasteFromClipboard(true) },
    { id: "edit.delete", title: "Clear contents", kind: "Edit", keywords: ["delete", "erase", "remove"], shortcut: "⌫", run: ctl.clearSelection },
    { id: "edit.clearFormats", title: "Clear formats", kind: "Edit", keywords: ["reset", "default", "plain"], run: async () => void (await ctl.clearFormats()) },
    { id: "edit.pasteSpecial", title: "Paste special…", kind: "Edit", keywords: ["transpose", "values", "formats", "skip empty", "add", "multiply"], ...sc("edit.pasteSpecial"), run: () => useUiStore.getState().openDialog({ kind: "pasteSpecial" }) },
    { id: "edit.find", title: "Find…", kind: "Edit", keywords: ["search", "look for"], ...sc("edit.find"), run: () => useUiStore.getState().setFind(true) },
    {
      id: "edit.findReplace",
      title: "Find & replace…",
      kind: "Edit",
      keywords: ["replace", "substitute", "search"],
      ...sc("edit.findReplace"),
      run: () => useUiStore.getState().openDialog({ kind: "findReplace", text: useDataStore.getState().find.text }),
    },
    { id: "edit.findNext", title: "Find next", kind: "Edit", hidden: true, ...sc("edit.findNext"), run: () => ctl.findStep(1) },
    { id: "edit.findPrev", title: "Find previous", kind: "Edit", hidden: true, ...sc("edit.findPrev"), run: () => ctl.findStep(-1) },
    { id: "edit.fillDown", title: "Fill down", kind: "Edit", keywords: ["copy down", "series"], ...sc("edit.fillDown"), ...notEditing, run: () => ctl.fillDir("down") },
    { id: "edit.fillRight", title: "Fill right", kind: "Edit", keywords: ["copy right", "series"], ...sc("edit.fillRight"), ...notEditing, run: () => ctl.fillDir("right") },
    { id: "edit.fillUp", title: t("cmd.edit.fillUp", "Fill up"), kind: "Edit", keywords: ["copy up"], ...notEditing, run: () => ctl.fillDir("up") },
    { id: "edit.fillLeft", title: t("cmd.edit.fillLeft", "Fill left"), kind: "Edit", keywords: ["copy left"], ...notEditing, run: () => ctl.fillDir("left") },
    {
      id: "edit.fillSheet",
      title: "Fill sheets…",
      kind: "Edit",
      keywords: ["copy to sheets", "selected sheets", "group"],
      // Only with several sheets selected (AOO Edit ▸ Fill ▸ Sheet); the dialog is data/FillSheetDialog.tsx.
      enabled: () => grouped() && !editing() && ctl.ranges.length === 1,
      // AOO refuses a multiple selection ("This function cannot be used with multiple selections").
      get reason() {
        return grouped() ? t("msg.reason.multiSelection", "This function cannot be used with multiple selections") : t("msg.reason.selectSheets", "Select two or more sheets first");
      },
      run: () => dialog().openDialog({ kind: "fillSheet" }),
    },
    { id: "edit.cancelClip", title: "Cancel copy", kind: "Edit", hidden: true, ...sc("edit.cancelClip"), run: ctl.cancelClip },
    // Format
    { id: "format.bold", title: "Bold", kind: "Format", keywords: ["strong", "weight"], ...sc("format.bold"), run: () => ctl.toggleStyle("bold") },
    { id: "format.italic", title: "Italic", kind: "Format", keywords: ["slant", "emphasis"], ...sc("format.italic"), run: () => ctl.toggleStyle("italic") },
    { id: "format.underline", title: "Underline", kind: "Format", ...sc("format.underline"), run: () => ctl.toggleStyle("underline") },
    { id: "format.strike", title: "Strikethrough", kind: "Format", keywords: ["cross out", "strike"], run: () => ctl.toggleStyle("strike") },
    { id: "format.wrap", title: "Wrap text", kind: "Format", keywords: ["lines", "multi-line"], run: () => ctl.toggleStyle("wrap") },
    { id: "format.merge", title: "Merge cells", kind: "Format", keywords: ["combine", "join", "span", "header"], run: () => ctl.mergeCells() },
    { id: "format.unmerge", title: "Unmerge cells", kind: "Format", keywords: ["split"], run: async () => void (await ctl.unmergeCells()) },
    { id: "format.toggleMerge", title: "Merge / unmerge", kind: "Format", hidden: true, run: () => ctl.toggleMerge() },
    ...BORDER_PRESETS.map(
      (b): Command => ({
        id: `format.borders.${b.id}`,
        title: t(`msg.palette.borders.${b.id}`, BORDER_TITLES[b.id]),
        kind: "Format",
        keywords: ["border", "lines", "outline", "box", "grid"],
        run: async () => void (await ctl.applyBorders(b.id, borderLine("thin"))),
      }),
    ),
    { id: "format.alignLeft", title: "Align left", kind: "Format", ...sc("format.alignLeft"), run: async () => void (await ctl.setAlign("left")) },
    { id: "format.alignCenter", title: "Align centre", kind: "Format", keywords: ["center", "middle"], ...sc("format.alignCenter"), run: async () => void (await ctl.setAlign("center")) },
    { id: "format.alignRight", title: "Align right", kind: "Format", ...sc("format.alignRight"), run: async () => void (await ctl.setAlign("right")) },
    {
      id: "format.colorByValue",
      title: "Colour by value",
      kind: "Format",
      keywords: ["chips", "status", "highlight", "color", "category"],
      run: () => ctl.colorByValue(),
    },
    ...NUMBER_FORMATS.filter((f) => f.id !== "currency" && f.id !== "percent").map(
      (f): Command => ({
        id: `format.number.${f.id}`,
        title: t("msg.palette.numberFormat", "Number format: {name}", { name: f.label }),
        kind: "Format",
        keywords: ["number format", f.code],
        ...sc(`format.number.${f.id}`),
        run: async () => void (await ctl.setNumberFormat(f.code, f.label)),
      }),
    ),
    { id: "format.number.custom", title: "Custom number format…", kind: "Format", keywords: ["number format", "code"], run: () => ui.customNumberFormat() },
    {
      id: "format.currency",
      title: "Format as currency",
      kind: "Format",
      keywords: ["dollar", "money", "$", "price"],
      run: async (args) => {
        const target = currencyTarget(args, ctl.selection.range);
        await (target ? ctl.setNumberFormatOn([target], CURRENCY_FORMAT, "Currency") : ctl.setNumberFormat(CURRENCY_FORMAT, "Currency"));
      },
    },
    {
      id: "format.percent",
      title: "Format as percent",
      kind: "Format",
      keywords: ["%", "percentage", "ratio"],
      run: async () => {
        await ctl.setNumberFormat(PERCENT_FORMAT, "Percent");
      },
    },
    // Data / insert
    { id: "insert.addUp", title: "Add up (Σ)", kind: "Formula", keywords: ["sum", "total", "autosum"], run: () => ctl.addUp() },
    { id: "data.sortAsc", title: "Sort ascending (A → Z)", kind: "Sort", keywords: ["order", "smallest"], run: () => ctl.sortByActive(true) },
    { id: "data.sortDesc", title: "Sort descending (Z → A)", kind: "Sort", keywords: ["order", "largest", "reverse"], run: () => ctl.sortByActive(false) },
    { id: "data.sort", title: "Sort…", kind: "Sort", keywords: ["order", "keys", "columns"], run: () => ctl.openSortDialog() },
    { id: "data.filter", title: "AutoFilter on / off", kind: "Filter", keywords: ["where", "only", "dropdown"], run: () => ctl.toggleAutoFilter() },
    { id: "data.clearFilters", title: "Clear filters", kind: "Filter", keywords: ["show all"], enabled: hasFilter, reason: t("msg.reason.noAutoFilter", "No AutoFilter on this sheet"), run: () => ctl.clearFilters() },
    { id: "data.defineName", title: "Define name…", kind: "Data", keywords: ["named range", "name"], ...sc("data.defineName"), run: ctl.openDefineName },
    { id: "data.manageNames", title: "Manage names…", kind: "Data", keywords: ["named ranges", "names"], run: () => useUiStore.getState().openDialog({ kind: "manageNames" }) },
    { id: "data.recalc", title: "Recalculate all", kind: "Data", keywords: ["hard recalc", "refresh", "F9"], ...sc("data.recalc"), run: ctl.recalcAll },
    { id: "insert.chart", title: "Chart", kind: "Chart", keywords: ["graph", "plot"], ...notEditing, run: () => ctl.w5.charts.insertChart() },
    // Assist actions (run from ⌘K suggestions with args; hidden from the list).
    { id: "filter.byColumn", title: "Filter by column", kind: "Filter", hidden: true, run: (args) => ctl.filterByColumn(args) },
    { id: "chart.byColumn", title: "Chart by column", kind: "Chart", hidden: true, run: () => ctl.w5.charts.insertChart() },
    // Sheet structure (the Sheet menu in the palette)
    { id: "sheet.insertRowAbove", title: "Insert rows above", kind: "Sheet", keywords: ["add row"], run: async () => void (await ctl.insertRows("before")) },
    { id: "sheet.insertRowBelow", title: "Insert rows below", kind: "Sheet", keywords: ["add row"], run: async () => void (await ctl.insertRows("after")) },
    { id: "sheet.insertColLeft", title: "Insert columns left", kind: "Sheet", keywords: ["add column"], run: async () => void (await ctl.insertCols("before")) },
    { id: "sheet.insertColRight", title: "Insert columns right", kind: "Sheet", keywords: ["add column"], run: async () => void (await ctl.insertCols("after")) },
    { id: "sheet.deleteRows", title: "Delete rows", kind: "Sheet", keywords: ["remove row"], run: async () => void (await ctl.deleteRows()) },
    { id: "sheet.deleteCols", title: "Delete columns", kind: "Sheet", keywords: ["remove column"], run: async () => void (await ctl.deleteCols()) },
    { id: "sheet.hideRows", title: "Hide rows", kind: "Sheet", run: () => ctl.setHidden("row", true) },
    { id: "sheet.showRows", title: "Show rows", kind: "Sheet", keywords: ["unhide"], run: () => ctl.setHidden("row", false) },
    { id: "sheet.hideCols", title: "Hide columns", kind: "Sheet", run: () => ctl.setHidden("col", true) },
    { id: "sheet.showCols", title: "Show columns", kind: "Sheet", keywords: ["unhide"], run: () => ctl.setHidden("col", false) },
    { id: "sheet.rowHeight", title: "Row height…", kind: "Sheet", keywords: ["resize"], run: () => ui.resize("row") },
    { id: "sheet.colWidth", title: "Column width…", kind: "Sheet", keywords: ["resize"], run: () => ui.resize("col") },
    // Sheets
    { id: "sheet.insert", title: "Insert sheet", kind: "Sheet", keywords: ["add", "new tab"], run: async () => void (await ctl.addSheet()) },
    { id: "sheet.rename", title: "Rename sheet", kind: "Sheet", keywords: ["tab name"], run: () => ui.renameActiveSheet() },
    {
      id: "sheet.delete",
      title: "Delete sheet",
      kind: "Sheet",
      keywords: ["remove tab"],
      // With several sheets selected, deletes them all unless that would leave none.
      enabled: () => (undo().info?.sheets.filter((s) => !s.hidden).length ?? 0) > Math.max(1, undo().groupSheets.length),
      run: async () => void (await ctl.deleteSelectedSheets()),
    },
    { id: "sheet.moveLeft", title: "Move sheet left", kind: "Sheet", keywords: ["reorder", "tab"], run: () => ctl.moveSheetBy(-1) },
    { id: "sheet.moveRight", title: "Move sheet right", kind: "Sheet", keywords: ["reorder", "tab"], run: () => ctl.moveSheetBy(1) },
    { id: "sheet.next", title: "Next sheet", kind: "Sheet", ...sc("sheet.next"), run: () => ctl.stepSheet(1) },
    { id: "sheet.prev", title: "Previous sheet", kind: "Sheet", ...sc("sheet.prev"), run: () => ctl.stepSheet(-1) },
    { id: "sheet.selectNext", title: "Add next sheet to selection", kind: "Sheet", keywords: ["select sheets", "group", "multiple"], ...sc("sheet.selectNext"), run: () => ctl.selectStep(1) },
    { id: "sheet.selectPrev", title: "Add previous sheet to selection", kind: "Sheet", keywords: ["select sheets", "group", "multiple"], ...sc("sheet.selectPrev"), run: () => ctl.selectStep(-1) },
    {
      id: "sheet.selectAll",
      title: "Select all sheets",
      kind: "Sheet",
      keywords: ["group", "multiple", "every sheet"],
      enabled: () => (undo().info?.sheets.filter((s) => !s.hidden).length ?? 0) > 1,
      reason: t("msg.reason.oneSheet", "There is only one visible sheet"),
      run: ctl.selectAllSheets,
    },
    { id: "sheet.deselectAll", title: "Deselect all sheets", kind: "Sheet", keywords: ["ungroup sheets", "single sheet"], enabled: grouped, reason: t("msg.reason.oneSheetSelected", "Only one sheet is selected"), run: ctl.deselectAllSheets },
    // File
    { id: "file.new", title: "New workbook", kind: "File", keywords: ["create", "blank"], ...sc("file.new"), run: ctl.newWorkbook },
    { id: "file.open", title: "Open…", kind: "File", keywords: ["load", "file"], ...sc("file.open"), run: () => ctl.openDialog() },
    { id: "file.openReadOnly", title: t("cmd.file.openReadOnly", "Open read-only…"), kind: "File", keywords: ["load", "file", "view only", "read only"], run: () => ctl.openDialog(true) },
    {
      id: "edit.editMode",
      title: t("cmd.edit.editMode", "Edit mode"),
      kind: "Edit",
      keywords: ["read-only", "read only", "edit file", "edit document", "lock"],
      ...sc("edit.editMode"),
      enabled: () => ctl.readOnly.canToggle,
      reason: t("msg.reason.editModeNoFile", "Only a document saved in a file can switch to read-only."),
      run: () => ctl.readOnly.toggleEditMode(),
    },
    { id: "file.save", title: "Save", kind: "File", ...sc("file.save"), run: () => ctl.save(false) },
    { id: "file.saveAs", title: "Save as…", kind: "File", ...sc("file.saveAs"), run: () => ctl.save(true) },
    { id: "file.exportCsv", title: "Export as CSV…", kind: "File", keywords: ["share", "download", "csv"], run: () => ctl.save(true, "csv") },
    { id: "file.exportXlsx", title: "Export Excel (XLSX)…", kind: "File", keywords: ["share", "download", "excel", "xlsx", "microsoft"], run: () => ctl.save(true, "xlsx") },
    { id: "file.exportOds", title: "Export ODS…", kind: "File", keywords: ["share", "download", "opendocument", "ods"], run: () => ctl.save(true, "ods") },
    { id: "file.close", title: "Close workbook", kind: "File", run: ctl.closeDocument },
    // Window: document tabs (desktop only; tabs.ts). The native Window menu
    // handles its own items in the shell; these serve the keymap and palette.
    ...windowCommands(ctl, sc),
    // View
    { id: "view.toggleSidebar", title: "Toggle sidebar", kind: "View", keywords: ["panel", "workbooks"], ...sc("view.toggleSidebar"), run: () => useUiStore.getState().toggleSidebar() },
    { id: "view.toolbar", title: t("cmd.view.toolbar", "Toolbar"), kind: "View", keywords: ["toolbars", "show", "hide", "top bar"], run: () => toggleBar(ctl, "toolbar") },
    { id: "view.formattingBar", title: t("cmd.view.formattingBar", "Formatting bar"), kind: "View", keywords: ["toolbars", "format bar", "show", "hide"], run: () => toggleBar(ctl, "formatting") },
    { id: "view.formulaBar", title: t("cmd.view.formulaBar", "Formula bar"), kind: "View", keywords: ["toolbars", "input line", "show", "hide"], run: () => toggleBar(ctl, "formulaBar") },
    { id: "view.statusBar", title: t("cmd.view.statusBar", "Status bar"), kind: "View", keywords: ["toolbars", "show", "hide"], run: () => toggleBar(ctl, "statusBar") },
    { id: "view.customizeToolbars", title: t("cmd.view.customizeToolbars", "Customize toolbars…"), kind: "View", keywords: ["toolbars", "visible buttons", "customise", "hide buttons"], run: () => openCustomize() },
    { id: "view.glance", title: "At a glance", kind: "View", keywords: ["summary", "insights", "kpi", "panel", "chart"], ...sc("view.glance"), run: () => useUiStore.getState().toggleGlance() },
    {
      id: "view.theme",
      title: "Theme: system / light / dark",
      kind: "View",
      keywords: ["dark mode", "light mode", "appearance", "colour scheme"],
      run: () => {
        useUiStore.getState().cycleTheme();
        const p = useUiStore.getState().themePref;
        useUiStore.getState().pushToast("info", p === "system" ? t("msg.toast.themeSystem", "Theme: match system") : p === "light" ? t("msg.toast.themeLight", "Theme: light") : t("msg.toast.themeDark", "Theme: dark"));
      },
    },
    {
      id: "view.freeze",
      title: "Freeze panes at the active cell",
      kind: "View",
      keywords: ["freeze", "lock", "header", "pin rows", "keep visible"],
      run: () => ctl.freeze("both"),
    },
    { id: "view.freezeRows", title: "Freeze rows above the active cell", kind: "View", keywords: ["freeze", "header row", "lock"], run: () => ctl.freeze("rows") },
    { id: "view.freezeCols", title: "Freeze columns left of the active cell", kind: "View", keywords: ["freeze", "lock"], run: () => ctl.freeze("cols") },
    { id: "view.unfreeze", title: "Unfreeze panes", kind: "View", keywords: ["freeze", "unlock"], run: async () => void (await ctl.unfreeze()) },
    { id: "view.zoomIn", title: "Zoom in", kind: "View", ...sc("view.zoomIn"), run: () => useUiStore.getState().zoomBy(1) },
    { id: "view.zoomOut", title: "Zoom out", kind: "View", ...sc("view.zoomOut"), run: () => useUiStore.getState().zoomBy(-1) },
    { id: "view.zoomReset", title: "Actual size", kind: "View", keywords: ["100%", "zoom"], ...sc("view.zoomReset"), run: () => useUiStore.getState().setZoom(1) },
    { id: "palette.toggle", title: "Command palette", kind: "View", hidden: true, ...sc("palette.toggle"), run: () => useUiStore.getState().setPalette(!useUiStore.getState().paletteOpen) },
    // v0.3: conditional formatting, validation, notes, links
    { id: "format.conditional", title: "Conditional formatting…", kind: "Format", keywords: ["highlight", "rules", "colour scale", "data bar", "icon set", "conditional"], run: () => useUiStore.getState().openDialog({ kind: "cfManager" }) },
    ...HIGHLIGHT_PRESETS.map(
      (p): Command => ({
        id: `format.highlight.${p.id}`,
        title: `${p.dialogTitle}${p.values > 0 ? "…" : ""}`,
        kind: "Format",
        keywords: ["conditional formatting", "highlight", "rule"],
        run: () => ctl.v3.highlight(p.id),
      }),
    ),
    { id: "data.validation", title: "Data validation…", kind: "Data", keywords: ["dropdown", "list", "allowed values", "restrict", "validity"], run: () => ctl.v3.openValidation() },
    { id: "insert.note", title: "Insert / edit note", kind: "Insert", keywords: ["comment", "annotation"], ...sc("insert.note"), ...notEditing, run: () => ctl.v3.editNote() },
    { id: "insert.deleteNote", title: "Delete note", kind: "Insert", keywords: ["comment", "remove"], run: () => ctl.v3.deleteNote() },
    { id: "insert.hyperlink", title: "Hyperlink…", kind: "Insert", keywords: ["link", "url", "web", "mail"], ...sc("insert.hyperlink"), ...notEditing, run: () => ctl.v3.openHyperlink() },
    { id: "insert.removeHyperlink", title: "Remove hyperlink", kind: "Insert", keywords: ["link", "unlink"], run: () => ctl.v3.removeLink() },
    { id: "insert.pageBreak", title: "Insert page break above row", kind: "Insert", keywords: ["print", "page"], run: () => ctl.v3.insertPageBreak("row") },
    { id: "insert.colPageBreak", title: "Insert page break left of column", kind: "Insert", keywords: ["print", "page"], run: () => ctl.v3.insertPageBreak("col") },
    { id: "insert.removePageBreak", title: "Remove page breaks at the cursor", kind: "Insert", keywords: ["print", "page"], run: () => ctl.v3.removePageBreaks() },
    { id: "format.fitRows", title: "Fit row height to contents", kind: "Format", keywords: ["auto fit", "resize rows", "wrap"], run: () => ctl.v3.fitRows() },
    { id: "data.refreshFilter", title: "Refresh filter", kind: "Filter", keywords: ["reapply", "autofilter", "update"], enabled: hasFilter, reason: t("msg.reason.noAutoFilter", "No AutoFilter on this sheet"), run: () => ctl.v3.refreshFilter() },
    // v0.3: tools, file, view
    { id: "tools.protectSheet", title: "Protect / unprotect sheet…", kind: "Tools", keywords: ["lock", "password", "protection"], run: () => ctl.v3.protectSheetCommand() },
    { id: "tools.protectDocument", title: "Protect / unprotect document…", kind: "Tools", keywords: ["structure", "lock", "password"], run: () => ctl.v3.protectDocumentCommand() },
    { id: "tools.goalSeek", title: "Goal seek…", kind: "Tools", keywords: ["what if", "solve", "target"], run: () => ctl.v3.openGoalSeek() },
    { id: "file.properties", title: "Document properties…", kind: "File", keywords: ["title", "author", "metadata", "info"], run: () => ctl.v3.openProperties() },
    { id: "file.pageSetup", title: "Page setup…", kind: "File", keywords: ["print", "paper", "margins", "orientation", "header", "footer"], run: () => ctl.v3.openPageSetup() },
    {
      id: "view.pageBreaks",
      title: "Show / hide page breaks",
      kind: "View",
      keywords: ["print", "page", "preview"],
      run: () => ctl.v3.setPageBreaksOn(!useV3Store.getState().pageBreaksOn),
    },
    ...UI_LOCALES.map(
      (l): Command => ({
        id: l.command,
        title: `Locale: ${l.label}`,
        kind: "View",
        keywords: ["language", "region", "number format", l.tag],
        run: () => ctl.v3.setLocale(l.tag),
      }),
    ),
    // Wave 5 seams (W5-0b): registered so menu.rs, the keymap and the palette
    // agree on the ids; `enabled: false` until W5-5 / W5-6 implement them.
    // Labels go through `t()` with the `cmd.<id>` keys of tally-l10n/ui/en.json.
    { id: "insert.function", title: t("cmd.insert.function", "Function…"), kind: "Formula", keywords: ["wizard", "fx", "insert function"], ...sc("insert.function"), run: () => dialog().openDialog({ kind: "functionWizard" }) },
    { id: "insert.functionList", title: t("cmd.insert.functionList", "Function list"), kind: "Formula", keywords: ["functions", "fx", "sidebar", "insert function"], run: () => ctl.w56.toggleFunctionList() },
    { id: "insert.image", title: t("cmd.insert.image", "Image…"), kind: "Insert", keywords: ["picture", "png", "jpeg"], ...notEditing, run: () => ctl.w5.objects.insertImage() },
    { id: "chart.edit", title: t("cmd.chart.edit", "Edit chart"), kind: "Chart", keywords: ["graph"], run: () => ctl.w5.charts.openEditor() },
    { id: "chart.delete", title: t("cmd.chart.delete", "Delete chart"), kind: "Chart", keywords: ["graph", "remove"], run: () => ctl.w5.charts.deleteSelected() },
    { id: "chart.copyImage", title: t("cmd.chart.copyImage", "Copy chart as image"), kind: "Chart", keywords: ["png", "clipboard"], run: () => ctl.w5.charts.copyAsImage() },
    { id: "data.dataPilot", title: t("cmd.data.dataPilot", "DataPilot…"), kind: "Data", keywords: ["pivot table", "summarise", "crosstab"], run: () => void ctl.w56.openDataPilot() },
    { id: "data.pivotFilter", title: t("cmd.data.pivotFilter", "DataPilot filter…"), kind: "Data", keywords: ["pivot", "datapilot", "filter button", "source"], enabled: () => useW56Store.getState().pivots.some((p) => p.id === useW56Store.getState().activePivot && p.showFilterButton), reason: t("msg.reason.pivotFilterButton", "Put the cursor inside a pivot table with a Filter button"), run: () => void ctl.w56.openPivotFilter() },
    { id: "data.refreshPivot", title: t("cmd.data.refreshPivot", "Refresh pivot table"), kind: "Data", keywords: ["pivot", "datapilot"], enabled: () => useW56Store.getState().activePivot !== null, reason: t("msg.reason.cursorInPivot", "Put the cursor inside a pivot table"), run: () => void ctl.w56.refreshPivot() },
    { id: "data.standardFilter", title: t("cmd.data.standardFilter", "Standard filter…"), kind: "Filter", keywords: ["conditions", "and", "or", "regex"], run: () => void ctl.w56.openStandardFilter(false) },
    { id: "data.advancedFilter", title: t("cmd.data.advancedFilter", "Advanced filter…"), kind: "Filter", keywords: ["criteria range"], run: () => void ctl.w56.openStandardFilter(true) },
    { id: "data.resetFilter", title: t("cmd.data.resetFilter", "Reset filter"), kind: "Filter", keywords: ["show all rows"], run: () => void ctl.w56.resetFilter() },
    { id: "data.subtotals", title: t("cmd.data.subtotals", "Subtotals…"), kind: "Data", keywords: ["group", "sum", "outline"], run: () => void ctl.w56.openSubtotals() },
    { id: "data.consolidate", title: t("cmd.data.consolidate", "Consolidate…"), kind: "Data", keywords: ["combine ranges", "sum sheets"], run: () => ctl.w56.openConsolidate() },
    { id: "data.textToColumns", title: t("cmd.data.textToColumns", "Text to columns…"), kind: "Data", keywords: ["split", "csv", "delimiter"], run: () => void ctl.w56.openTextToColumns() },
    { id: "data.removeDuplicates", title: t("cmd.data.removeDuplicates", "Remove duplicates…"), kind: "Data", keywords: ["unique", "dedupe"], run: () => void ctl.w56.openRemoveDuplicates() },
    { id: "data.multipleOperations", title: t("cmd.data.multipleOperations", "Multiple operations…"), kind: "Data", keywords: ["what if", "data table"], run: () => ctl.w56.openMultipleOperations() },
    { id: "data.group", title: t("cmd.data.group", "Group"), kind: "Data", keywords: ["outline", "collapse rows"], ...sc("data.group"), ...notEditing, run: () => void ctl.w56.group() },
    { id: "data.ungroup", title: t("cmd.data.ungroup", "Ungroup"), kind: "Data", keywords: ["outline"], ...sc("data.ungroup"), ...notEditing, run: () => void ctl.w56.ungroup() },
    { id: "data.autoOutline", title: t("cmd.data.autoOutline", "AutoOutline"), kind: "Data", keywords: ["outline", "group"], run: () => void ctl.w56.autoOutline() },
    { id: "data.showOutlineLevel", title: t("cmd.data.showOutlineLevel", "Show outline level…"), kind: "Data", keywords: ["outline", "collapse", "expand"], run: () => ctl.w56.openShowOutlineLevel() },
    { id: "data.defineDatabaseRange", title: t("cmd.data.defineDatabaseRange", "Define database range…"), kind: "Data", keywords: ["db range", "named range"], run: () => void ctl.w56.openDefineDatabaseRange() },
    { id: "data.selectDatabaseRange", title: t("cmd.data.selectDatabaseRange", "Select range…"), kind: "Data", keywords: ["database range", "go to", "db range"], run: () => void ctl.w56.openSelectDatabaseRange() },
    { id: "data.refreshRange", title: t("cmd.data.refreshRange", "Refresh range"), kind: "Data", keywords: ["database range", "repeat", "sort", "filter", "subtotals"], ...notEditing, run: () => void ctl.w56.refreshDatabaseRange() },
    { id: "data.createNamesFromLabels", title: t("cmd.data.createNamesFromLabels", "Create names from labels…"), kind: "Data", keywords: ["names", "header row"], run: () => ctl.w56.openCreateNames() },
    { id: "edit.fillSeries", title: t("cmd.edit.fillSeries", "Fill series…"), kind: "Edit", keywords: ["linear", "growth", "dates"], ...notEditing, run: () => ctl.w56.openFillSeries() },
    { id: "format.cells", title: t("cmd.format.cells", "Format cells…"), kind: "Format", keywords: ["rotation", "indent", "shrink", "diagonal"], ...sc("format.cells"), run: () => void ctl.w56.openFormatCells() },
    { id: "format.autoFormat", title: t("cmd.format.autoFormat", "AutoFormat…"), kind: "Format", keywords: ["table style", "preset"], run: () => void ctl.w56.openAutoFormat() },
    { id: "format.loadStyles", title: t("cmd.format.loadStyles", "Load Styles…"), kind: "Format", keywords: ["styles", "template", "import styles"], run: () => ctl.w56.openLoadStyles() },
    { id: "view.stylesPanel", title: t("cmd.view.stylesPanel", "Styles panel"), kind: "View", keywords: ["cell styles", "heading", "result"], ...sc("view.stylesPanel"), run: () => ctl.w56.toggleStylesPanel() },
    { id: "view.pageBreakPreview", title: t("cmd.view.pageBreakPreview", "Page break preview"), kind: "View", keywords: ["print", "pages"], run: () => ctl.w5.view.togglePageBreakPreview() },
    { id: "view.split", title: t("cmd.view.split", "Split window"), kind: "View", keywords: ["panes", "divider"], run: () => ctl.w5.view.toggleSplit() },
    { id: "view.removeSplit", title: t("cmd.view.removeSplit", "Remove split"), kind: "View", keywords: ["panes", "divider", "window"], run: () => ctl.w5.view.removeSplit() },
    { id: "view.navigator", title: t("cmd.view.navigator", "Navigator"), kind: "View", keywords: ["sheets", "names", "charts", "notes"], ...sc("view.navigator"), run: () => ctl.w56.toggleNavigator() },
    { id: "view.showFormulas", title: t("cmd.view.showFormulas", "Show formulas"), kind: "View", keywords: ["formula view"], ...sc("view.showFormulas"), run: () => ctl.w56.toggleView("showFormulas") },
    { id: "view.valueHighlighting", title: t("cmd.view.valueHighlighting", "Value highlighting"), kind: "View", keywords: ["colour by type"], ...sc("view.valueHighlighting"), run: () => ctl.w56.toggleView("valueHighlighting") },
    { id: "view.zeroValues", title: t("cmd.view.zeroValues", "Zero values"), kind: "View", keywords: ["show zeros", "hide zeros"], run: () => ctl.w56.toggleView("showZeroValues") },
    { id: "view.gridLines", title: t("cmd.view.gridLines", "Grid lines"), kind: "View", keywords: ["gridlines"], run: () => ctl.w56.toggleView("showGridlines") },
    { id: "view.headers", title: t("cmd.view.headers", "Column and row headers"), kind: "View", keywords: ["headings"], run: () => ctl.w56.toggleView("showHeaders") },
    { id: "view.zoomOptimal", title: t("cmd.view.zoomOptimal", "Optimal zoom"), kind: "View", keywords: ["zoom", "fit selection"], run: () => ctl.w5.view.zoomOptimal() },
    { id: "view.zoomPageWidth", title: t("cmd.view.zoomPageWidth", "Zoom to page width"), kind: "View", keywords: ["zoom"], run: () => ctl.w5.view.zoomToPage("width") },
    { id: "view.zoomWholePage", title: t("cmd.view.zoomWholePage", "Zoom to whole page"), kind: "View", keywords: ["zoom"], run: () => ctl.w5.view.zoomToPage("both") },
    { id: "view.zoomDialog", title: t("cmd.view.zoomDialog", "Zoom…"), kind: "View", keywords: ["zoom", "scale", "magnify", "percent"], run: () => ctl.w5.view.openZoomDialog() },
    { id: "view.fullScreen", title: t("cmd.view.fullScreen", "Full screen"), kind: "View", keywords: ["fullscreen"], run: () => ctl.w5.view.toggleFullScreen() },
    { id: "tools.options", title: t("cmd.tools.options", "Options…"), kind: "Tools", keywords: ["preferences", "settings", "calculate", "iterations"], ...sc("tools.options"), run: () => void ctl.w56.openOptions() },
    { id: "tools.solver", title: t("cmd.tools.solver", "Solver…"), kind: "Tools", keywords: ["optimise", "linear", "constraints"], run: () => ctl.w56.openSolver() },
    { id: "tools.scenarios", title: t("cmd.tools.scenarios", "Scenarios…"), kind: "Tools", keywords: ["what if"], run: () => void ctl.w56.openScenarios() },
    { id: "tools.tracePrecedents", title: t("cmd.tools.tracePrecedents", "Trace precedents"), kind: "Tools", keywords: ["detective", "arrows"], ...sc("tools.tracePrecedents"), ...notEditing, run: () => ctl.w5.detective.tracePrecedents() },
    { id: "tools.removePrecedents", title: t("cmd.tools.removePrecedents", "Remove precedents"), kind: "Tools", keywords: ["detective", "arrows", "clear"], ...notEditing, run: () => ctl.w5.detective.removePrecedents() },
    { id: "tools.traceDependents", title: t("cmd.tools.traceDependents", "Trace dependents"), kind: "Tools", keywords: ["detective", "arrows"], ...sc("tools.traceDependents"), ...notEditing, run: () => ctl.w5.detective.traceDependents() },
    { id: "tools.removeDependents", title: t("cmd.tools.removeDependents", "Remove dependents"), kind: "Tools", keywords: ["detective", "arrows", "clear"], ...notEditing, run: () => ctl.w5.detective.removeDependents() },
    { id: "tools.removeTraces", title: t("cmd.tools.removeTraces", "Remove all traces"), kind: "Tools", keywords: ["detective"], ...notEditing, run: () => ctl.w5.detective.removeTraces() },
    { id: "tools.traceError", title: t("cmd.tools.traceError", "Trace error"), kind: "Tools", keywords: ["detective", "arrows", "error source"], ...notEditing, run: () => ctl.w5.detective.traceError() },
    { id: "tools.markInvalid", title: t("cmd.tools.markInvalid", "Mark invalid data"), kind: "Tools", keywords: ["detective", "validation", "circles"], run: () => ctl.w5.detective.markInvalid() },
    { id: "tools.refreshTraces", title: t("cmd.tools.refreshTraces", "Refresh traces"), kind: "Tools", keywords: ["detective", "arrows", "redraw"], ...notEditing, run: () => ctl.w5.detective.refreshTraces() },
    { id: "tools.autoRefreshTraces", title: t("cmd.tools.autoRefreshTraces", "AutoRefresh"), kind: "Tools", keywords: ["detective", "arrows", "automatic", "refresh traces"], run: () => ctl.w5.detective.toggleAutoRefresh() },
    { id: "tools.detectiveFillMode", title: t("cmd.tools.detectiveFillMode", "Fill mode"), kind: "Tools", keywords: ["detective", "arrows", "click", "trace precedents"], ...notEditing, run: () => ctl.w5.detective.toggleFillMode() },
    { id: "tools.autoCorrect", title: t("cmd.tools.autoCorrect", "AutoCorrect options…"), kind: "Tools", keywords: ["capitalise", "replace"], run: () => void ctl.w56.openOptions("input") },
    { id: "tools.autoInput", title: t("cmd.tools.autoInput", "AutoInput"), kind: "Tools", keywords: ["autocomplete", "column completion"], run: () => ctl.w56.toggleAutoInput() },
    { id: "file.printPreview", title: t("cmd.file.printPreview", "Print preview"), kind: "File", keywords: ["pages"], run: () => ctl.w5.print.openPreview() },
    { id: "file.print", title: t("cmd.file.print", "Print…"), kind: "File", keywords: ["printer"], ...sc("file.print"), run: () => ctl.w5.print.print() },
    { id: "file.exportPdf", title: t("cmd.file.exportPdf", "Export as PDF…"), kind: "File", keywords: ["share", "pdf"], run: () => ctl.w5.print.exportPdf() },
    { id: "file.saveAsTemplate", title: t("cmd.file.saveAsTemplate", "Save as template…"), kind: "File", keywords: ["ots"], run: () => ctl.w56.openSaveAsTemplate() },
    { id: "file.templates", title: t("cmd.file.templates", "New from template…"), kind: "File", keywords: ["gallery"], run: () => useUiStore.getState().setSidebar(true) },
    { id: "file.clearRecent", title: t("cmd.file.clearRecent", "Clear recent files"), kind: "File", keywords: ["open recent"], run: () => void ctl.w56.clearRecent() },
    { id: "sheet.duplicate", title: t("cmd.sheet.duplicate", "Copy sheet…"), kind: "Sheet", keywords: ["duplicate", "clone"], run: () => ctl.w56.openCopySheet() },
    { id: "sheet.hide", title: t("cmd.sheet.hide", "Hide sheet"), kind: "Sheet", keywords: ["tab"], run: () => void ctl.w56.hideSheet() },
    { id: "sheet.show", title: t("cmd.sheet.show", "Show sheet…"), kind: "Sheet", keywords: ["unhide", "tab"], run: () => ctl.w56.openShowSheet() },
    { id: "sheet.tabColor", title: t("cmd.sheet.tabColor", "Tab colour…"), kind: "Sheet", keywords: ["tab color"], run: () => ctl.w56.openTabColor() },
    // Wave 5 (W5-6): shortcut parity and the dialogs reached by keys only.
    { id: "edit.repeat", title: t("cmd.edit.repeat", "Repeat last command"), kind: "Edit", keywords: ["again", "redo action"], ...sc("edit.repeat"), run: () => ctl.w56.repeatLast() },
    { id: "edit.deleteContents", title: t("cmd.edit.deleteContents", "Delete contents…"), kind: "Edit", keywords: ["clear", "delete key", "formats", "notes"], ...notEditing, run: () => ctl.w56.openDeleteContents() },
    { id: "edit.selectData", title: t("cmd.edit.selectData", "Select data area"), kind: "Edit", keywords: ["current region", "table"], ...notEditing, run: () => void ctl.w56.selectDataArea() },
    { id: "edit.selectArray", title: t("cmd.edit.selectArray", "Select array formula"), kind: "Edit", keywords: ["matrix", "block"], ...notEditing, run: () => ctl.w56.selectArrayFormula() },
    { id: "edit.fillSelection", title: t("cmd.edit.fillSelection", "Fill selection with the active cell"), kind: "Edit", keywords: ["alt enter", "repeat value"], ...notEditing, run: () => void ctl.w56.fillSelection() },
    { id: "edit.focusNameBox", title: t("cmd.edit.focusNameBox", "Go to the Name Box"), kind: "Edit", hidden: true, ...sc("edit.focusNameBox"), run: () => ctl.w56.focusNameBox() },
    { id: "sheet.insertCells", title: t("cmd.sheet.insertCells", "Insert cells…"), kind: "Sheet", keywords: ["shift down", "shift right"], ...notEditing, run: () => ctl.w56.openInsertCells() },
    { id: "sheet.deleteCells", title: t("cmd.sheet.deleteCells", "Delete cells…"), kind: "Sheet", keywords: ["shift up", "shift left"], ...notEditing, run: () => ctl.w56.openDeleteCells() },
    { id: "data.sortOptions", title: t("cmd.data.sortOptions", "Sort with options…"), kind: "Sort", keywords: ["left to right", "custom list", "copy results"], run: () => void ctl.w56.openSortOptions() },
    { id: "data.clearOutline", title: t("cmd.data.clearOutline", "Remove outline"), kind: "Data", keywords: ["ungroup all", "outline"], run: () => void ctl.w56.clearOutline() },
    { id: "file.documentStats", title: t("cmd.file.documentStats", "Document statistics…"), kind: "File", keywords: ["cells", "formulas", "count"], run: () => void ctl.w56.openDocumentStats() },
    // Help
    { id: "help.shortcuts", title: "Keyboard shortcuts", kind: "Help", keywords: ["keys", "hotkeys", "cheat sheet"], ...sc("help.shortcuts"), run: () => useUiStore.getState().setShortcuts(!useUiStore.getState().shortcutsOpen) },
    { id: "help.about", title: "About Tally", kind: "Help", run: () => ui.showAbout() },
  ]));
}

/** The document-tab commands (enabled only with the desktop tab shell). */
function windowCommands(ctl: AppController, sc: (id: string) => { shortcut?: string }): Command[] {
  const tabs = { enabled: () => ctl.tabs.available, reason: t("msg.reason.tabsDesktopOnly", "Tabs are available in the desktop app") };
  const select = Array.from({ length: 9 }, (_, i): Command => ({
    id: `window.selectTab${i + 1}`,
    title: i === 8 ? "Show last tab" : `Show tab ${i + 1}`,
    kind: "Window",
    hidden: true,
    ...sc(`window.selectTab${i + 1}`),
    ...tabs,
    run: () => ctl.tabs.select(i + 1),
  }));
  return [
    { id: "window.nextTab", title: t("menu.window.showNextTab", "Show Next Tab"), kind: "Window", keywords: ["tab", "document", "switch"], ...sc("window.nextTab"), ...tabs, run: () => ctl.tabs.next() },
    { id: "window.prevTab", title: t("menu.window.showPrevTab", "Show Previous Tab"), kind: "Window", keywords: ["tab", "document", "switch"], ...sc("window.prevTab"), ...tabs, run: () => ctl.tabs.prev() },
    { id: "window.moveTabToNewWindow", title: t("menu.window.moveTabToNewWindow", "Move Tab to New Window"), kind: "Window", keywords: ["tab", "detach", "split"], ...tabs, run: () => ctl.tabs.detachCurrent() },
    { id: "window.mergeAllWindows", title: t("menu.window.mergeAllWindows", "Merge All Windows"), kind: "Window", keywords: ["tabs", "combine", "join"], ...tabs, run: () => ctl.tabs.merge() },
    ...select,
  ];
}

/** Command titles in the UI language: `cmd.<id>` from the installed table, else the literal. */
export function localizeTitles(cmds: Command[]): Command[] {
  return cmds.map((c) => {
    const title = t(`cmd.${c.id}`, c.title);
    return title === c.title ? c : copyCommand(c, { title });
  });
}
