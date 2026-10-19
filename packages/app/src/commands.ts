/**
 * Every toolbar, menu and keymap action as a CommandRegistry command.
 * Menu ids (apps/desktop/src-tauri/src/menu.rs) and assist action ids map
 * here one to one.
 */
import { columnIndex, parseA1Range } from "./a1";
import type { AppController } from "./controller";
import { CURRENCY_FORMAT, NUMBER_FORMATS, PERCENT_FORMAT } from "./formats";
import { shortcutFor } from "./keymap";
import type { Command, CommandArgs } from "./registry";
import type { RangeDto } from "@tally/bridge";
import { useDocStore } from "./stores/docStore";
import { useEditStore } from "./stores/editStore";
import { useUiStore } from "./stores/uiStore";
import { useDataStore } from "./stores/dataStore";
import { BORDER_PRESETS, borderLine } from "./dataOps";

export const COMING_SOON = "Coming soon";
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
  const notEditing = { enabled: () => !editing(), reason: "Finish editing the cell first" };
  const soon = { enabled: () => false, reason: COMING_SOON };
  const hasFilter = (): boolean => useDataStore.getState().filter !== null;
  return [
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
        title: `Borders: ${b.label.toLowerCase()}`,
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
        title: `Number format: ${f.label}`,
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
    { id: "data.clearFilters", title: "Clear filters", kind: "Filter", keywords: ["show all"], enabled: hasFilter, reason: "No AutoFilter on this sheet", run: () => ctl.clearFilters() },
    { id: "data.defineName", title: "Define name…", kind: "Data", keywords: ["named range", "name"], ...sc("data.defineName"), run: ctl.openDefineName },
    { id: "data.manageNames", title: "Manage names…", kind: "Data", keywords: ["named ranges", "names"], run: () => useUiStore.getState().openDialog({ kind: "manageNames" }) },
    { id: "data.recalc", title: "Recalculate all", kind: "Data", keywords: ["hard recalc", "refresh", "F9"], ...sc("data.recalc"), run: ctl.recalcAll },
    { id: "insert.chart", title: "Chart", kind: "Chart", keywords: ["graph", "plot"], ...soon, run: () => undefined },
    // Assist actions (run from ⌘K suggestions with args; hidden from the list).
    { id: "filter.byColumn", title: "Filter by column", kind: "Filter", hidden: true, run: (args) => ctl.filterByColumn(args) },
    { id: "chart.byColumn", title: "Chart by column", kind: "Chart", hidden: true, run: () => void useUiStore.getState().pushToast("info", CHART_SOON) },
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
      enabled: () => (undo().info?.sheets.length ?? 0) > 1,
      run: async () => void (await ctl.deleteSheet(ctl.sheet)),
    },
    { id: "sheet.moveLeft", title: "Move sheet left", kind: "Sheet", keywords: ["reorder", "tab"], run: () => ctl.moveSheetBy(-1) },
    { id: "sheet.moveRight", title: "Move sheet right", kind: "Sheet", keywords: ["reorder", "tab"], run: () => ctl.moveSheetBy(1) },
    { id: "sheet.next", title: "Next sheet", kind: "Sheet", ...sc("sheet.next"), run: () => ctl.stepSheet(1) },
    { id: "sheet.prev", title: "Previous sheet", kind: "Sheet", ...sc("sheet.prev"), run: () => ctl.stepSheet(-1) },
    // File
    { id: "file.new", title: "New workbook", kind: "File", keywords: ["create", "blank"], ...sc("file.new"), run: ctl.newWorkbook },
    { id: "file.open", title: "Open…", kind: "File", keywords: ["load", "file"], ...sc("file.open"), run: ctl.openDialog },
    { id: "file.save", title: "Save", kind: "File", ...sc("file.save"), run: () => ctl.save(false) },
    { id: "file.saveAs", title: "Save as…", kind: "File", ...sc("file.saveAs"), run: () => ctl.save(true) },
    { id: "file.exportCsv", title: "Export as CSV…", kind: "File", keywords: ["share", "download", "csv"], run: () => ctl.save(true, "csv") },
    { id: "file.exportXlsx", title: "Export Excel (XLSX)…", kind: "File", keywords: ["share", "download", "excel", "xlsx", "microsoft"], run: () => ctl.save(true, "xlsx") },
    { id: "file.exportOds", title: "Export ODS…", kind: "File", keywords: ["share", "download", "opendocument", "ods"], run: () => ctl.save(true, "ods") },
    { id: "file.close", title: "Close workbook", kind: "File", run: ctl.closeDocument },
    // View
    { id: "view.toggleSidebar", title: "Toggle sidebar", kind: "View", keywords: ["panel", "workbooks"], ...sc("view.toggleSidebar"), run: () => useUiStore.getState().toggleSidebar() },
    { id: "view.glance", title: "At a glance", kind: "View", keywords: ["summary", "insights", "kpi", "panel", "chart"], ...sc("view.glance"), run: () => useUiStore.getState().toggleGlance() },
    {
      id: "view.theme",
      title: "Theme: system / light / dark",
      kind: "View",
      keywords: ["dark mode", "light mode", "appearance", "colour scheme"],
      run: () => {
        useUiStore.getState().cycleTheme();
        const p = useUiStore.getState().themePref;
        useUiStore.getState().pushToast("info", `Theme: ${p === "system" ? "match system" : p}`);
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
    // Help
    { id: "help.shortcuts", title: "Keyboard shortcuts", kind: "Help", keywords: ["keys", "hotkeys", "cheat sheet"], ...sc("help.shortcuts"), run: () => useUiStore.getState().setShortcuts(!useUiStore.getState().shortcutsOpen) },
    { id: "help.about", title: "About Tally", kind: "Help", run: () => ui.showAbout() },
  ];
}
