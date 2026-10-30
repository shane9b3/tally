/**
 * "+" and the sheet tabs: click to switch, double-click to rename,
 * right-click menu, drag to reorder. Several sheets can be selected (AOO
 * ScTabControl): ⌘/Ctrl-click toggles a tab, Shift-click selects a run of
 * tabs, Shift+←/→ extends the selection from the keyboard.
 */
import type { SheetInfo } from "@tally/bridge";
import { useRef, useState } from "react";
import { cx, useController } from "../context";
import { t } from "../i18n";
import { useW56Store } from "../data/store";
import { useDocStore } from "../stores/docStore";
import { PlusIcon } from "./icons";
import { Popover } from "./Popover";
import styles from "./SheetTabs.module.css";

const NO_SHEETS: readonly SheetInfo[] = [];
const NO_GROUP: readonly number[] = [];

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
      aria-label={t("sheet.renamePrompt", "Sheet name")}
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
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const sheets = useDocStore((s) => s.info?.sheets ?? NO_SHEETS);
  const active = useDocStore((s) => s.activeSheet);
  const group = useDocStore((s) => (s.groupSheets.length > 1 ? s.groupSheets : NO_GROUP));
  const selected = (id: number) => id === active || group.includes(id);
  const nSelected = Math.max(1, group.length);
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
    <div className={styles.tabs}>
      <button
        type="button"
        className={styles.add}
        aria-label={t("sheetTabs.add", "Add sheet")}
        aria-disabled={nSelected > 1}
        title={nSelected > 1 ? t("msg.reason.sheetsGrouped", "Not available with several sheets selected") : t("sheetTabs.add", "Add sheet")}
        onClick={() => {
          if (nSelected <= 1) void ctl.addSheet();
        }}
      >
        <PlusIcon />
      </button>
      {/* The tablist holds only the tabs (ARIA); ←/→/Home/End move between sheets. */}
      <div
        className={styles.tabList}
        role="tablist"
        aria-label={t("navigator.sheets", "Sheets")}
        aria-multiselectable="true"
        onKeyDown={(e) => {
          const shown = sheets.filter((x) => !x.hidden);
          const i = shown.findIndex((x) => x.id === active);
          const j = e.key === "ArrowLeft" ? i - 1 : e.key === "ArrowRight" ? i + 1 : e.key === "Home" ? 0 : e.key === "End" ? shown.length - 1 : null;
          if (j === null || !(e.target instanceof HTMLElement) || e.target.getAttribute("role") !== "tab") return;
          e.preventDefault();
          const list = e.currentTarget;
          const refocus = () => requestAnimationFrame(() => list.querySelector<HTMLElement>('[role="tab"][tabindex="0"]')?.focus());
          // Shift+←/→ adds the next tab to the selection; plain keys move and select only that tab.
          if (e.shiftKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
            ctl.selectStep(e.key === "ArrowLeft" ? -1 : 1, false);
            refocus();
            return;
          }
          const to = shown[Math.max(0, Math.min(shown.length - 1, j))];
          if (!to) return;
          useDocStore.getState().selectOnly(to.id);
          ctl.showSheet(to.id, false);
          refocus();
        }}
      >
      {sheets.filter((s) => !s.hidden).map((s, i) =>
        renaming === s.id ? (
          <RenameInput key={s.id} initial={s.name} onDone={(name) => finishRename(s.id, name)} />
        ) : (
          <button
            key={s.id}
            type="button"
            role="tab"
            aria-selected={selected(s.id)}
            aria-current={s.id === active && group.length > 1 ? "page" : undefined}
            tabIndex={s.id === active ? 0 : -1}
            data-sheet={s.id}
            className={cx(
              styles.tab,
              s.id === active && styles.active,
              s.id !== active && group.includes(s.id) && styles.grouped,
              dragging === s.id && styles.dragging,
              dropAt === i && dragging !== null && styles.dropBefore,
            )}
            draggable
            onMouseDown={(e) => {
              // Picking references for a formula: the editor (cell or formula bar) keeps the focus.
              if (ctl.keepsEditAcrossSheets()) e.preventDefault();
            }}
            onClick={(e) => ctl.clickTab(s.id, { toggle: e.metaKey || e.ctrlKey, range: e.shiftKey })}
            onDoubleClick={(e) => {
              // Picking references: the clicks only showed the sheet; no rename over an open formula.
              if (ctl.keepsEditAcrossSheets()) return;
              if (!e.metaKey && !e.ctrlKey && !e.shiftKey && group.length <= 1) setRenaming(s.id);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              // No tab menu while a formula picks references (AOO ignores it in reference input).
              if (ctl.keepsEditAcrossSheets()) return;
              // Inside the selection the tab becomes active and the selection stays (AOO).
              ctl.showSheet(s.id, false, group.includes(s.id));
              setMenu({ sheet: s.id, x: e.clientX, y: e.clientY - 4 });
            }}
            onDragStart={(e) => {
              if (ctl.keepsEditAcrossSheets()) {
                e.preventDefault();
                return;
              }
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
              if (dragging !== null && dropAt !== null) void ctl.dropSheets(dragging, dropAt);
              setDragging(null);
              setDropAt(null);
            }}
          >
            {s.name}
          </button>
        ),
      )}
      </div>
      {menu ? (
        <Popover
          label={t("dialog.sheet", "Sheet")}
          x={menu.x}
          y={menu.y}
          above
          onClose={() => setMenu(null)}
          items={[
            // With several sheets selected Insert and Rename are off and Delete removes them all (AOO).
            { id: "rename", label: t("sheetTabs.rename", "Rename…"), disabled: nSelected > 1, onSelect: () => setRenaming(menu.sheet) },
            { id: "insert", label: t("cmd.sheet.insert", "Insert sheet"), disabled: nSelected > 1, onSelect: () => void ctl.addSheet() },
            nSelected > 1
              ? {
                  id: "delete",
                  label: t("sheetTabs.deleteN", "Delete {n} sheets", { n: nSelected }),
                  danger: true,
                  // At least one visible sheet stays.
                  disabled: nSelected >= sheets.filter((x) => !x.hidden).length,
                  onSelect: () => void ctl.deleteSelectedSheets(),
                }
              : {
                  id: "delete",
                  label: t("dialog.delete", "Delete"),
                  danger: true,
                  disabled: sheets.length <= 1,
                  onSelect: () => void ctl.deleteSheet(menu.sheet),
                },
            ...ctl.w56.sheetTabItems(menu.sheet),
          ]}
        />
      ) : null}
    </div>
  );
}
