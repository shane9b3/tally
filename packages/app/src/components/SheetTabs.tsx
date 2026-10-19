/** "+" and the sheet tabs: click to switch, double-click to rename, right-click menu, drag to reorder. */
import type { SheetInfo } from "@tally/bridge";
import { useRef, useState } from "react";
import { cx, useController } from "../context";
import { useDocStore } from "../stores/docStore";
import { PlusIcon } from "./icons";
import { Popover } from "./Popover";
import styles from "./SheetTabs.module.css";

const NO_SHEETS: readonly SheetInfo[] = [];

export interface SheetTabsProps {
  renaming: number | null;
  setRenaming: (sheet: number | null) => void;
}

function RenameInput({ initial, onDone }: { initial: string; onDone: (name: string | null) => void }) {
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const finish = (name: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(name);
  };
  return (
    <input
      className={styles.rename}
      aria-label="Sheet name"
      data-testid="sheet-rename"
      autoFocus
      value={value}
      size={Math.max(4, value.length + 1)}
      onFocus={(e) => e.currentTarget.select()}
      onChange={(e) => setValue(e.target.value)}
      onBlur={() => finish(value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          finish(value);
        } else if (e.key === "Escape") {
          e.preventDefault();
          finish(null);
        }
      }}
    />
  );
}

export function SheetTabs({ renaming, setRenaming }: SheetTabsProps) {
  const ctl = useController();
  const sheets = useDocStore((s) => s.info?.sheets ?? NO_SHEETS);
  const active = useDocStore((s) => s.activeSheet);
  const [menu, setMenu] = useState<{ sheet: number; x: number; y: number } | null>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const [dropAt, setDropAt] = useState<number | null>(null);

  const finishRename = (sheet: number, name: string | null) => {
    setRenaming(null);
    const old = sheets.find((s) => s.id === sheet)?.name;
    const next = name?.trim();
    if (next && next !== old) void ctl.renameSheet(sheet, next);
    ctl.focusGrid();
  };

  return (
    <div className={styles.tabs} role="tablist" aria-label="Sheets">
      <button type="button" className={styles.add} aria-label="Add sheet" title="Add sheet" onClick={() => void ctl.addSheet()}>
        <PlusIcon />
      </button>
      {sheets.map((s, i) =>
        renaming === s.id ? (
          <RenameInput key={s.id} initial={s.name} onDone={(name) => finishRename(s.id, name)} />
        ) : (
          <button
            key={s.id}
            type="button"
            role="tab"
            aria-selected={s.id === active}
            className={cx(
              styles.tab,
              s.id === active && styles.active,
              dragging === s.id && styles.dragging,
              dropAt === i && dragging !== null && styles.dropBefore,
            )}
            draggable
            onClick={() => ctl.showSheet(s.id)}
            onDoubleClick={() => setRenaming(s.id)}
            onContextMenu={(e) => {
              e.preventDefault();
              ctl.showSheet(s.id, false);
              setMenu({ sheet: s.id, x: e.clientX, y: e.clientY - 4 });
            }}
            onDragStart={(e) => {
              setDragging(s.id);
              e.dataTransfer.effectAllowed = "move";
              e.dataTransfer.setData("text/x-tally-sheet", String(s.id));
            }}
            onDragOver={(e) => {
              if (dragging === null) return;
              e.preventDefault();
              const r = e.currentTarget.getBoundingClientRect();
              setDropAt(e.clientX < r.left + r.width / 2 ? i : i + 1);
            }}
            onDragEnd={() => {
              setDragging(null);
              setDropAt(null);
            }}
            onDrop={(e) => {
              e.preventDefault();
              const from = sheets.findIndex((x) => x.id === dragging);
              if (dragging !== null && dropAt !== null && from >= 0) {
                const to = dropAt > from ? dropAt - 1 : dropAt;
                if (to !== from) void ctl.moveSheet(dragging, to);
              }
              setDragging(null);
              setDropAt(null);
            }}
          >
            {s.name}
          </button>
        ),
      )}
      {menu ? (
        <Popover
          label="Sheet"
          x={menu.x}
          y={menu.y}
          above
          onClose={() => setMenu(null)}
          items={[
            { id: "rename", label: "Rename…", onSelect: () => setRenaming(menu.sheet) },
            { id: "insert", label: "Insert sheet", onSelect: () => void ctl.addSheet() },
            {
              id: "delete",
              label: "Delete",
              danger: true,
              disabled: sheets.length <= 1,
              onSelect: () => void ctl.deleteSheet(menu.sheet),
            },
          ]}
        />
      ) : null}
    </div>
  );
}
