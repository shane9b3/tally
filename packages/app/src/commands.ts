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

export const COMING_SOON = "Coming soon";
export const FILTER_SOON = "Filtering is coming in v0.2";
export const CHART_SOON = "Charts are coming in v0.2";

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

/** Shown on "Hide rows/columns" until the engine has the op. */
export const HIDE_SOON = "Hiding rows and columns needs an engine op that is coming soon";

export function buildCommands(ctl: AppController, ui: CommandUi): Command[] {
  const sc = (id: string): { shortcut?: string } => {
    const s = shortcutFor(id);
    return s === undefined ? {} : { shortcut: s };
  };
  const undo = useDocStore.getState;
  const editing = (): boolean => useEditStore.getState().edit !== null;
  const soon = { enabled: () => false, reason: COMING_SOON };
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
    // Format
    { id: "format.bold", title: "Bold", kind: "Format", keywords: ["strong", "weight"], ...sc("format.bold"), run: () => ctl.toggleStyle("bold") },
    { id: "format.italic", title: "Italic", kind: "Format", keywords: ["slant", "emphasis"], ...sc("format.italic"), run: () => ctl.toggleStyle("italic") },
    { id: "format.underline", title: "Underline", kind: "Format", ...sc("format.underline"), run: () => ctl.toggleStyle("underline") },
    { id: "format.strike", title: "Strikethrough", kind: "Format", keywords: ["cross out", "strike"], run: () => ctl.toggleStyle("strike") },
    { id: "format.wrap", title: "Wrap text", kind: "Format", keywords: ["lines", "multi-line"], run: () => ctl.toggleStyle("wrap") },
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
    { id: "data.sort", title: "Sort", kind: "Sort", keywords: ["order"], ...soon, run: () => undefined },
    { id: "data.filter", title: "Filter", kind: "Filter", keywords: ["where", "only"], ...soon, run: () => undefined },
    { id: "insert.chart", title: "Chart", kind: "Chart", keywords: ["graph", "plot"], ...soon, run: () => undefined },
    // Assist actions (run from ⌘K suggestions with args; hidden from the list).
    { id: "filter.byColumn", title: "Filter by column", kind: "Filter", hidden: true, run: () => void useUiStore.getState().pushToast("info", FILTER_SOON) },
    { id: "chart.byColumn", title: "Chart by column", kind: "Chart", hidden: true, run: () => void useUiStore.getState().pushToast("info", CHART_SOON) },
    // Sheet structure (the Sheet menu in the palette)
    { id: "sheet.insertRowAbove", title: "Insert rows above", kind: "Sheet", keywords: ["add row"], run: async () => void (await ctl.insertRows("before")) },
    { id: "sheet.insertRowBelow", title: "Insert rows below", kind: "Sheet", keywords: ["add row"], run: async () => void (await ctl.insertRows("after")) },
    { id: "sheet.insertColLeft", title: "Insert columns left", kind: "Sheet", keywords: ["add column"], run: async () => void (await ctl.insertCols("before")) },
    { id: "sheet.insertColRight", title: "Insert columns right", kind: "Sheet", keywords: ["add column"], run: async () => void (await ctl.insertCols("after")) },
    { id: "sheet.deleteRows", title: "Delete rows", kind: "Sheet", keywords: ["remove row"], run: async () => void (await ctl.deleteRows()) },
    { id: "sheet.deleteCols", title: "Delete columns", kind: "Sheet", keywords: ["remove column"], run: async () => void (await ctl.deleteCols()) },
    { id: "sheet.hideRows", title: "Hide rows", kind: "Sheet", enabled: () => false, reason: HIDE_SOON, run: () => undefined },
    { id: "sheet.hideCols", title: "Hide columns", kind: "Sheet", enabled: () => false, reason: HIDE_SOON, run: () => undefined },
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
    { id: "sheet.next", title: "Next sheet", kind: "Sheet", ...sc("sheet.next"), run: () => ctl.stepSheet(1) },
    { id: "sheet.prev", title: "Previous sheet", kind: "Sheet", ...sc("sheet.prev"), run: () => ctl.stepSheet(-1) },
    // File
    { id: "file.new", title: "New workbook", kind: "File", keywords: ["create", "blank"], ...sc("file.new"), run: ctl.newWorkbook },
    { id: "file.open", title: "Open…", kind: "File", keywords: ["load", "file"], ...sc("file.open"), run: ctl.openDialog },
    { id: "file.save", title: "Save", kind: "File", ...sc("file.save"), run: () => ctl.save(false) },
    { id: "file.saveAs", title: "Save as…", kind: "File", ...sc("file.saveAs"), run: () => ctl.save(true) },
    { id: "file.exportCsv", title: "Export as CSV…", kind: "File", keywords: ["share", "download", "csv"], run: () => ctl.save(true, "csv") },
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
    { id: "view.zoomIn", title: "Zoom in", kind: "View", ...sc("view.zoomIn"), run: () => useUiStore.getState().zoomBy(1) },
    { id: "view.zoomOut", title: "Zoom out", kind: "View", ...sc("view.zoomOut"), run: () => useUiStore.getState().zoomBy(-1) },
    { id: "view.zoomReset", title: "Actual size", kind: "View", keywords: ["100%", "zoom"], ...sc("view.zoomReset"), run: () => useUiStore.getState().setZoom(1) },
    { id: "palette.toggle", title: "Command palette", kind: "View", hidden: true, ...sc("palette.toggle"), run: () => useUiStore.getState().setPalette(!useUiStore.getState().paletteOpen) },
    // Help
    { id: "help.shortcuts", title: "Keyboard shortcuts", kind: "Help", keywords: ["keys", "hotkeys", "cheat sheet"], ...sc("help.shortcuts"), run: () => useUiStore.getState().setShortcuts(!useUiStore.getState().shortcutsOpen) },
    { id: "help.about", title: "About Tally", kind: "Help", run: () => ui.showAbout() },
  ];
}
