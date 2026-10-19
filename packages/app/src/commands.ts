/**
 * Every toolbar, menu and keymap action as a CommandRegistry command.
 * Menu ids (apps/desktop/src-tauri/src/menu.rs) and assist action ids map
 * here one to one.
 */
import { columnIndex, parseA1Range } from "./a1";
import type { AppController } from "./controller";
import { CURRENCY_FORMAT, PERCENT_FORMAT } from "./formats";
import { shortcutFor } from "./keymap";
import type { Command, CommandArgs } from "./registry";
import type { RangeDto } from "@tally/bridge";
import { useDocStore } from "./stores/docStore";
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
}

export function buildCommands(ctl: AppController, ui: CommandUi): Command[] {
  const sc = (id: string): { shortcut?: string } => {
    const s = shortcutFor(id);
    return s === undefined ? {} : { shortcut: s };
  };
  const undo = useDocStore.getState;
  const soon = { enabled: () => false, reason: COMING_SOON };
  return [
    // Edit
    { id: "edit.undo", title: "Undo", kind: "Edit", keywords: ["revert", "back"], ...sc("edit.undo"), enabled: () => undo().undo.canUndo, run: ctl.undo },
    { id: "edit.redo", title: "Redo", kind: "Edit", keywords: ["again"], ...sc("edit.redo"), enabled: () => undo().undo.canRedo, run: ctl.redo },
    { id: "edit.copy", title: "Copy", kind: "Edit", shortcut: "⌘C", run: () => void ctl.copy() },
    { id: "edit.cut", title: "Cut", kind: "Edit", shortcut: "⌘X", run: ctl.cut },
    { id: "edit.delete", title: "Clear contents", kind: "Edit", keywords: ["delete", "erase", "remove"], shortcut: "⌫", run: ctl.clearSelection },
    // Format
    { id: "format.bold", title: "Bold", kind: "Format", keywords: ["strong", "weight"], ...sc("format.bold"), run: () => ctl.toggleStyle("bold") },
    { id: "format.italic", title: "Italic", kind: "Format", keywords: ["slant", "emphasis"], ...sc("format.italic"), run: () => ctl.toggleStyle("italic") },
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
    { id: "file.close", title: "Close workbook", kind: "File", run: ctl.closeDocument },
    // View
    { id: "view.toggleSidebar", title: "Toggle sidebar", kind: "View", keywords: ["panel", "workbooks"], ...sc("view.toggleSidebar"), run: () => useUiStore.getState().toggleSidebar() },
    { id: "view.zoomIn", title: "Zoom in", kind: "View", ...sc("view.zoomIn"), run: () => useUiStore.getState().zoomBy(1) },
    { id: "view.zoomOut", title: "Zoom out", kind: "View", ...sc("view.zoomOut"), run: () => useUiStore.getState().zoomBy(-1) },
    { id: "view.zoomReset", title: "Actual size", kind: "View", keywords: ["100%", "zoom"], ...sc("view.zoomReset"), run: () => useUiStore.getState().setZoom(1) },
    { id: "palette.toggle", title: "Command palette", kind: "View", hidden: true, ...sc("palette.toggle"), run: () => useUiStore.getState().setPalette(!useUiStore.getState().paletteOpen) },
    // Help
    { id: "help.about", title: "About Tally", kind: "Help", run: () => ui.showAbout() },
  ];
}
