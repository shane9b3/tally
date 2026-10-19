/** Right-click menus for cells, row headers and column headers. */
import { useController } from "../context";
import { useDataStore } from "../stores/dataStore";
import { useDocStore } from "../stores/docStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore, type ContextMenuState } from "../stores/uiStore";
import { Popover, type PopoverItem } from "./Popover";

const plural = (n: number, one: string, many: string): string => (n === 1 ? `1 ${one}` : `${n} ${many}`);

export function ContextMenu() {
  const menu = useUiStore((s) => s.contextMenu);
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
  const rows = plural(nRows, "row", "rows");
  const cols = plural(nCols, "column", "columns");
  const hasFilter = useDataStore.getState().filter !== null;
  const merged = ctl.selectionIsMerged();
  const single = nRows === 1 && nCols === 1;
  let items: PopoverItem[];

  if (menu.area === "rowHeader") {
    items = [
      { id: "insertAbove", label: `Insert ${rows} above`, onSelect: run(() => ctl.insertRows("before")) },
      { id: "insertBelow", label: `Insert ${rows} below`, onSelect: run(() => ctl.insertRows("after")) },
      { id: "insertN", label: "Insert rows…", onSelect: () => dialog({ kind: "insert", axis: "row", where: "before" }) },
      { id: "delete", label: `Delete ${rows}`, danger: true, separator: true, onSelect: run(ctl.deleteRows) },
      { id: "hide", label: `Hide ${rows}`, onSelect: run(() => ctl.setHidden("row", true)) },
      { id: "show", label: "Show hidden rows", onSelect: run(() => ctl.setHidden("row", false)) },
      { id: "freezeRows", label: "Freeze rows above", onSelect: run(() => ctl.freeze("rows")) },
      { id: "fitRows", label: "Fit row height to contents", onSelect: run(() => ctl.v3.fitRows()) },
      { id: "pageBreak", label: "Insert page break above", onSelect: run(() => ctl.v3.insertPageBreak("row")) },
      { id: "resize", label: "Row height…", separator: true, onSelect: () => openResize("row") },
      { id: "clear", label: "Clear contents", onSelect: run(ctl.clearSelection) },
    ];
  } else if (menu.area === "colHeader") {
    items = [
      { id: "insertLeft", label: `Insert ${cols} left`, onSelect: run(() => ctl.insertCols("before")) },
      { id: "insertRight", label: `Insert ${cols} right`, onSelect: run(() => ctl.insertCols("after")) },
      { id: "insertN", label: "Insert columns…", onSelect: () => dialog({ kind: "insert", axis: "col", where: "before" }) },
      { id: "delete", label: `Delete ${cols}`, danger: true, separator: true, onSelect: run(ctl.deleteCols) },
      { id: "hide", label: `Hide ${cols}`, onSelect: run(() => ctl.setHidden("col", true)) },
      { id: "show", label: "Show hidden columns", onSelect: run(() => ctl.setHidden("col", false)) },
      { id: "sortAsc", label: "Sort A → Z", separator: true, onSelect: run(() => ctl.sortByActive(true)) },
      { id: "sortDesc", label: "Sort Z → A", onSelect: run(() => ctl.sortByActive(false)) },
      { id: "resize", label: "Column width…", separator: true, onSelect: () => openResize("col") },
      { id: "fit", label: "Fit width to contents", onSelect: run(() => ctl.autoFit("col", range.c0, range.c1)) },
      { id: "pageBreak", label: "Insert page break left", onSelect: run(() => ctl.v3.insertPageBreak("col")) },
      { id: "clear", label: "Clear contents", onSelect: run(ctl.clearSelection) },
    ];
  } else {
    items = [
      { id: "cut", label: "Cut", hint: "⌘X", onSelect: run(ctl.cut) },
      { id: "copy", label: "Copy", hint: "⌘C", onSelect: run(ctl.copy) },
      { id: "paste", label: "Paste", hint: "⌘V", onSelect: () => void ctl.pasteFromClipboard(false) },
      { id: "pasteValues", label: "Paste values only", hint: "⇧⌘V", onSelect: () => void ctl.pasteFromClipboard(true) },
      { id: "pasteSpecial", label: "Paste special…", hint: "⌥⌘V", onSelect: () => dialog({ kind: "pasteSpecial" }) },
      { id: "clear", label: "Clear contents", hint: "⌫", separator: true, onSelect: run(ctl.clearSelection) },
      { id: "clearFormats", label: "Clear formats", onSelect: run(ctl.clearFormats) },
      { id: "sortAsc", label: "Sort A → Z", separator: true, onSelect: run(() => ctl.sortByActive(true)) },
      { id: "sortDesc", label: "Sort Z → A", onSelect: run(() => ctl.sortByActive(false)) },
      { id: "filter", label: hasFilter ? "Remove AutoFilter" : "AutoFilter", onSelect: run(ctl.toggleAutoFilter) },
      merged
        ? { id: "unmerge", label: "Unmerge cells", onSelect: run(ctl.unmergeCells) }
        : { id: "merge", label: "Merge cells", disabled: single, title: single ? "Select two or more cells" : undefined, onSelect: run(ctl.mergeCells) },
      { id: "defineName", label: "Define name…", onSelect: () => ctl.openDefineName() },
      ...cellExtras(),
      { id: "insertRows", label: `Insert ${rows} above`, separator: true, onSelect: run(() => ctl.insertRows("before")) },
      { id: "insertCols", label: `Insert ${cols} left`, onSelect: run(() => ctl.insertCols("before")) },
      { id: "deleteRows", label: `Delete ${rows}`, danger: true, onSelect: run(ctl.deleteRows) },
      { id: "deleteCols", label: `Delete ${cols}`, danger: true, onSelect: run(ctl.deleteCols) },
    ];
  }

  /** v0.3 items of the cell menu: note, hyperlink, validation, conditional formatting, page break. */
  function cellExtras(): PopoverItem[] {
    const view = useDocStore.getState().source?.getCell(menu.cell.r, menu.cell.c);
    const hasNote = ((view?.flags ?? 0) & 8) !== 0;
    const hasLink = ((view?.flags ?? 0) & 32) !== 0;
    const out: PopoverItem[] = [
      { id: "note", label: hasNote ? "Edit note…" : "Insert note…", hint: "⇧F2", separator: true, onSelect: () => void ctl.v3.editNote(menu.cell) },
    ];
    if (hasNote) out.push({ id: "deleteNote", label: "Delete note", onSelect: run(() => ctl.v3.deleteNote(menu.cell)) });
    out.push({ id: "hyperlink", label: hasLink ? "Edit hyperlink…" : "Hyperlink…", hint: "⌥⌘K", onSelect: () => void ctl.v3.openHyperlink() });
    if (hasLink) {
      out.push({ id: "openLink", label: "Open hyperlink", onSelect: () => view?.link && void ctl.v3.openLink(view.link) });
      out.push({ id: "removeLink", label: "Remove hyperlink", onSelect: run(() => ctl.v3.removeLink()) });
    }
    out.push(
      { id: "validation", label: "Data validation…", onSelect: () => void ctl.v3.openValidation() },
      { id: "conditional", label: "Conditional formatting…", onSelect: () => useUiStore.getState().openDialog({ kind: "cfManager" }) },
    );
    return out;
  }

  function openResize(axis: "row" | "col") {
    openResizeDialog(ctl, axis);
  }

  return <Popover label="Context menu" testId="context-menu" x={menu.x} y={menu.y} items={items} onClose={close} />;
}

/** Opens Row height / Column width for the selection, seeded with the first index's size. */
export function openResizeDialog(ctl: ReturnType<typeof useController>, axis: "row" | "col"): void {
  const range = ctl.selection.range;
  const start = axis === "row" ? range.r0 : range.c0;
  const end = axis === "row" ? range.r1 : range.c1;
  useUiStore.getState().openDialog({ kind: "resize", axis, start, end, px: ctl.layoutSize(axis, start) });
}
