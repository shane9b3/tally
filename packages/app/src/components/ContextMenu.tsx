/** Right-click menus for cells, row headers and column headers. */
import { useController } from "../context";
import { t } from "../i18n";
import { useW56Store } from "../data/store";
import { useDataStore } from "../stores/dataStore";
import { useDocStore } from "../stores/docStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore, type ContextMenuState } from "../stores/uiStore";
import { Popover, type PopoverItem } from "./Popover";

export function ContextMenu() {
  const menu = useUiStore((s) => s.contextMenu);
  useW56Store((s) => s.i18nRev);
  if (!menu) return null;
  return <Menu menu={menu} />;
}

function Menu({ menu }: { menu: ContextMenuState }) {
  const ctl = useController();
  const range = useSelectionStore((s) => s.selection.range);
  const close = () => useUiStore.getState().openContextMenu(null);
  const run = (f: () => unknown) => () => {
    void f();
    ctl.focusGrid();
  };
  const dialog = useUiStore.getState().openDialog;
  const nRows = range.r1 - range.r0 + 1;
  const nCols = range.c1 - range.c0 + 1;
  const rows = nRows === 1 ? t("shell.ctxOneRow", "1 row") : t("shell.ctxNRows", "{n} rows", { n: nRows });
  const cols = nCols === 1 ? t("shell.ctxOneCol", "1 column") : t("shell.ctxNCols", "{n} columns", { n: nCols });
  const hasFilter = useDataStore.getState().filter !== null;
  const merged = ctl.selectionIsMerged();
  const single = nRows === 1 && nCols === 1;
  // Off with several sheets selected (commands.ts GROUP_DISABLED_COMMANDS).
  const off = (command: string) => ctl.registry.has(command) && !ctl.registry.isEnabled(command);
  let items: PopoverItem[];

  if (menu.area === "rowHeader") {
    items = [
      { id: "insertAbove", label: t("shell.ctxInsertAbove", "Insert {what} above", { what: rows }), onSelect: run(() => ctl.insertRows("before")) },
      { id: "insertBelow", label: t("shell.ctxInsertBelow", "Insert {what} below", { what: rows }), onSelect: run(() => ctl.insertRows("after")) },
      { id: "insertN", label: t("shell.ctxInsertRowsN", "Insert rows…"), onSelect: () => dialog({ kind: "insert", axis: "row", where: "before" }) },
      { id: "delete", label: t("shell.ctxDelete", "Delete {what}", { what: rows }), danger: true, separator: true, onSelect: run(ctl.deleteRows) },
      { id: "hide", label: t("shell.ctxHide", "Hide {what}", { what: rows }), onSelect: run(() => ctl.setHidden("row", true)) },
      { id: "show", label: t("shell.ctxShowRows", "Show hidden rows"), onSelect: run(() => ctl.setHidden("row", false)) },
      { id: "freezeRows", label: t("shell.ctxFreezeRows", "Freeze rows above"), onSelect: run(() => ctl.freeze("rows")) },
      { id: "fitRows", label: t("cmd.format.fitRows", "Fit row height to contents"), onSelect: run(() => ctl.v3.fitRows()) },
      { id: "pageBreak", label: t("shell.ctxPageBreakAbove", "Insert page break above"), onSelect: run(() => ctl.v3.insertPageBreak("row")) },
      { id: "resize", label: t("cmd.sheet.rowHeight", "Row height…"), separator: true, onSelect: () => openResize("row") },
      { id: "clear", label: t("cmd.edit.delete", "Clear contents"), onSelect: run(ctl.clearSelection) },
    ];
  } else if (menu.area === "colHeader") {
    items = [
      { id: "insertLeft", label: t("shell.ctxInsertLeft", "Insert {what} left", { what: cols }), onSelect: run(() => ctl.insertCols("before")) },
      { id: "insertRight", label: t("shell.ctxInsertRight", "Insert {what} right", { what: cols }), onSelect: run(() => ctl.insertCols("after")) },
      { id: "insertN", label: t("shell.ctxInsertColsN", "Insert columns…"), onSelect: () => dialog({ kind: "insert", axis: "col", where: "before" }) },
      { id: "delete", label: t("shell.ctxDelete", "Delete {what}", { what: cols }), danger: true, separator: true, onSelect: run(ctl.deleteCols) },
      { id: "hide", label: t("shell.ctxHide", "Hide {what}", { what: cols }), onSelect: run(() => ctl.setHidden("col", true)) },
      { id: "show", label: t("shell.ctxShowCols", "Show hidden columns"), onSelect: run(() => ctl.setHidden("col", false)) },
      { id: "sortAsc", label: t("shell.ctxSortAsc", "Sort A → Z"), separator: true, disabled: off("data.sortAsc"), onSelect: run(() => ctl.sortByActive(true)) },
      { id: "sortDesc", label: t("shell.ctxSortDesc", "Sort Z → A"), disabled: off("data.sortDesc"), onSelect: run(() => ctl.sortByActive(false)) },
      { id: "resize", label: t("cmd.sheet.colWidth", "Column width…"), separator: true, onSelect: () => openResize("col") },
      { id: "fit", label: t("shell.ctxFitWidth", "Fit width to contents"), onSelect: run(() => ctl.autoFit("col", range.c0, range.c1)) },
      { id: "pageBreak", label: t("shell.ctxPageBreakLeft", "Insert page break left"), onSelect: run(() => ctl.v3.insertPageBreak("col")) },
      { id: "clear", label: t("cmd.edit.delete", "Clear contents"), onSelect: run(ctl.clearSelection) },
    ];
  } else {
    items = [
      { id: "cut", label: t("cmd.edit.cut", "Cut"), hint: "⌘X", onSelect: run(ctl.cut) },
      { id: "copy", label: t("cmd.edit.copy", "Copy"), hint: "⌘C", onSelect: run(ctl.copy) },
      { id: "paste", label: t("cmd.edit.paste", "Paste"), hint: "⌘V", onSelect: () => void ctl.pasteFromClipboard(false) },
      { id: "pasteValues", label: t("cmd.edit.pasteValues", "Paste values only"), hint: "⇧⌘V", onSelect: () => void ctl.pasteFromClipboard(true) },
      { id: "pasteSpecial", label: t("cmd.edit.pasteSpecial", "Paste special…"), hint: "⌥⌘V", onSelect: () => dialog({ kind: "pasteSpecial" }) },
      { id: "clear", label: t("cmd.edit.delete", "Clear contents"), hint: "⌫", separator: true, onSelect: run(ctl.clearSelection) },
      { id: "clearFormats", label: t("cmd.edit.clearFormats", "Clear formats"), onSelect: run(ctl.clearFormats) },
      { id: "sortAsc", label: t("shell.ctxSortAsc", "Sort A → Z"), separator: true, disabled: off("data.sortAsc"), onSelect: run(() => ctl.sortByActive(true)) },
      { id: "sortDesc", label: t("shell.ctxSortDesc", "Sort Z → A"), disabled: off("data.sortDesc"), onSelect: run(() => ctl.sortByActive(false)) },
      { id: "filter", label: hasFilter ? t("shell.ctxRemoveAutoFilter", "Remove AutoFilter") : t("shell.ctxAutoFilter", "AutoFilter"), disabled: off("data.filter"), onSelect: run(ctl.toggleAutoFilter) },
      merged
        ? { id: "unmerge", label: t("cmd.format.unmerge", "Unmerge cells"), onSelect: run(ctl.unmergeCells) }
        : { id: "merge", label: t("cmd.format.merge", "Merge cells"), disabled: single, title: single ? t("shell.ctxSelectTwo", "Select two or more cells") : undefined, onSelect: run(ctl.mergeCells) },
      { id: "defineName", label: t("cmd.data.defineName", "Define name…"), disabled: off("data.defineName"), onSelect: () => ctl.openDefineName() },
      ...cellExtras(),
      ...ctl.w56.cellMenuItems(menu.cell),
      { id: "insertRows", label: t("shell.ctxInsertAbove", "Insert {what} above", { what: rows }), separator: true, onSelect: run(() => ctl.insertRows("before")) },
      { id: "insertCols", label: t("shell.ctxInsertLeft", "Insert {what} left", { what: cols }), onSelect: run(() => ctl.insertCols("before")) },
      { id: "deleteRows", label: t("shell.ctxDelete", "Delete {what}", { what: rows }), danger: true, onSelect: run(ctl.deleteRows) },
      { id: "deleteCols", label: t("shell.ctxDelete", "Delete {what}", { what: cols }), danger: true, onSelect: run(ctl.deleteCols) },
    ];
  }

  /** v0.3 items of the cell menu: note, hyperlink, validation, conditional formatting, page break. */
  function cellExtras(): PopoverItem[] {
    const view = useDocStore.getState().source?.getCell(menu.cell.r, menu.cell.c);
    const hasNote = ((view?.flags ?? 0) & 8) !== 0;
    const hasLink = ((view?.flags ?? 0) & 32) !== 0;
    const out: PopoverItem[] = [
      { id: "note", label: hasNote ? t("shell.ctxEditNote", "Edit note…") : t("shell.ctxInsertNote", "Insert note…"), hint: "⇧F2", separator: true, onSelect: () => void ctl.v3.editNote(menu.cell) },
    ];
    if (hasNote) out.push({ id: "deleteNote", label: t("cmd.insert.deleteNote", "Delete note"), onSelect: run(() => ctl.v3.deleteNote(menu.cell)) });
    out.push({ id: "hyperlink", label: hasLink ? t("shell.ctxEditLink", "Edit hyperlink…") : t("cmd.insert.hyperlink", "Hyperlink…"), hint: "⌥⌘K", disabled: off("insert.hyperlink"), onSelect: () => void ctl.v3.openHyperlink() });
    if (hasLink) {
      out.push({ id: "openLink", label: t("shell.ctxOpenLink", "Open hyperlink"), onSelect: () => view?.link && void ctl.v3.openLink(view.link) });
      out.push({ id: "removeLink", label: t("cmd.insert.removeHyperlink", "Remove hyperlink"), disabled: off("insert.removeHyperlink"), onSelect: run(() => ctl.v3.removeLink()) });
    }
    out.push(
      { id: "validation", label: t("cmd.data.validation", "Data validation…"), onSelect: () => void ctl.v3.openValidation() },
      { id: "conditional", label: t("cmd.format.conditional", "Conditional formatting…"), onSelect: () => useUiStore.getState().openDialog({ kind: "cfManager" }) },
    );
    return out;
  }

  function openResize(axis: "row" | "col") {
    openResizeDialog(ctl, axis);
  }

  return <Popover label={t("shell.contextMenu", "Context menu")} testId="context-menu" x={menu.x} y={menu.y} items={items} onClose={close} />;
}

/** Opens Row height / Column width for the selection, seeded with the first index's size. */
export function openResizeDialog(ctl: ReturnType<typeof useController>, axis: "row" | "col"): void {
  const range = ctl.selection.range;
  const start = axis === "row" ? range.r0 : range.c0;
  const end = axis === "row" ? range.r1 : range.c1;
  useUiStore.getState().openDialog({ kind: "resize", axis, start, end, px: ctl.layoutSize(axis, start) });
}
