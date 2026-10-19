/** Right-click menus for cells, row headers and column headers. */
import { useController } from "../context";
import { HIDE_SOON } from "../commands";
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
  let items: PopoverItem[];

  if (menu.area === "rowHeader") {
    items = [
      { id: "insertAbove", label: `Insert ${rows} above`, onSelect: run(() => ctl.insertRows("before")) },
      { id: "insertBelow", label: `Insert ${rows} below`, onSelect: run(() => ctl.insertRows("after")) },
      { id: "insertN", label: "Insert rows…", onSelect: () => dialog({ kind: "insert", axis: "row", where: "before" }) },
      { id: "delete", label: `Delete ${rows}`, danger: true, separator: true, onSelect: run(ctl.deleteRows) },
      { id: "hide", label: `Hide ${rows}`, disabled: true, title: HIDE_SOON, onSelect: () => undefined },
      { id: "resize", label: "Row height…", separator: true, onSelect: () => openResize("row") },
      { id: "clear", label: "Clear contents", onSelect: run(ctl.clearSelection) },
    ];
  } else if (menu.area === "colHeader") {
    items = [
      { id: "insertLeft", label: `Insert ${cols} left`, onSelect: run(() => ctl.insertCols("before")) },
      { id: "insertRight", label: `Insert ${cols} right`, onSelect: run(() => ctl.insertCols("after")) },
      { id: "insertN", label: "Insert columns…", onSelect: () => dialog({ kind: "insert", axis: "col", where: "before" }) },
      { id: "delete", label: `Delete ${cols}`, danger: true, separator: true, onSelect: run(ctl.deleteCols) },
      { id: "hide", label: `Hide ${cols}`, disabled: true, title: HIDE_SOON, onSelect: () => undefined },
      { id: "resize", label: "Column width…", separator: true, onSelect: () => openResize("col") },
      { id: "fit", label: "Fit width to contents", onSelect: run(() => ctl.autoFit("col", range.c0, range.c1)) },
      { id: "clear", label: "Clear contents", onSelect: run(ctl.clearSelection) },
    ];
  } else {
    items = [
      { id: "cut", label: "Cut", hint: "⌘X", onSelect: run(ctl.cut) },
      { id: "copy", label: "Copy", hint: "⌘C", onSelect: run(ctl.copy) },
      { id: "paste", label: "Paste", hint: "⌘V", onSelect: () => void ctl.pasteFromClipboard(false) },
      { id: "pasteValues", label: "Paste values only", hint: "⇧⌘V", onSelect: () => void ctl.pasteFromClipboard(true) },
      { id: "clear", label: "Clear contents", hint: "⌫", separator: true, onSelect: run(ctl.clearSelection) },
      { id: "clearFormats", label: "Clear formats", onSelect: run(ctl.clearFormats) },
      { id: "insertRows", label: `Insert ${rows} above`, separator: true, onSelect: run(() => ctl.insertRows("before")) },
      { id: "insertCols", label: `Insert ${cols} left`, onSelect: run(() => ctl.insertCols("before")) },
      { id: "deleteRows", label: `Delete ${rows}`, danger: true, onSelect: run(ctl.deleteRows) },
      { id: "deleteCols", label: `Delete ${cols}`, danger: true, onSelect: run(ctl.deleteCols) },
    ];
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
