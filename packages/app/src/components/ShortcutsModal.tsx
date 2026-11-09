/** ⌘/: the keyboard shortcut reference (the keymap table plus the grid's own keys). */
import { useRef } from "react";
import { useController } from "../context";
import { t } from "../i18n";
import { KIND_KEYS } from "../i18n/locales";
import { useReturnFocus, useTitleId } from "./dialogA11y";
import { trapTab } from "./trapTab";
import { GRID_KEYS, isMacPlatform, platformBindings } from "../keymap";
import { useUiStore } from "../stores/uiStore";
import styles from "./ShortcutsModal.module.css";

const GRID_ACTION_KEYS: Readonly<Record<string, string>> = {
  "Clear contents": "cmd.edit.delete",
  "Paste values only": "cmd.edit.pasteValues",
  "Paste special…": "cmd.edit.pasteSpecial",
  "Move (⇧ extends the selection)": "shell.gk.move",
  "Jump to the edge of the data": "shell.gk.jumpEdge",
  "Start / end of the row": "shell.gk.rowStartEnd",
  "A1 / last used cell": "shell.gk.a1Last",
  "One screen up / down": "shell.gk.screen",
  "Next / previous cell": "shell.gk.nextCell",
  "Select the data, then everything": "shell.gk.selectData",
  "Add a range to the selection": "shell.gk.addRange",
  "Edit the active cell": "shell.gk.editActive",
  "Commit and move down / up": "shell.gk.commitMove",
  "While typing in a cell: new line in the cell": "shell.gk.newLine",
  "Cancel editing": "shell.gk.cancelEdit",
  "Copy / cut / paste": "shell.gk.copyCutPaste",
  "Fill down / right": "shell.gk.fill",
  "Copy instead of continuing a series": "shell.gk.fillCopy",
  "Move cells (references follow)": "shell.gk.moveCells",
  "Enter as an array formula over the selection": "shell.gk.arrayEnter",
  "Find / Find & replace": "shell.gk.find",
  "Next / previous match (in the find bar)": "shell.gk.nextMatch",
  "Point at a reference": "shell.gk.pointRef",
  "Accept a function suggestion": "shell.gk.acceptFn",
  "Open the link (a plain click selects)": "shell.gk.openLink",
  "Choose from the validation list": "shell.gk.validationList",
};

const GROUP_KEYS: Readonly<Record<string, string>> = { Move: "shell.group.move", Select: "wizard.select", Formulas: "shell.group.formulas", Find: "shell.group.find", Cells: "shell.group.cells", Other: "textToColumns.other" };

/** The shortcut reference rows and group names are written in English; they are translated when drawn. */
const gridText = (en: string): string => t(GRID_ACTION_KEYS[en] ?? "", en);
const groupText = (g: string): string => t(KIND_KEYS[g] ?? GROUP_KEYS[g] ?? "", g);

export function ShortcutsModal() {
  const open = useUiStore((s) => s.shortcutsOpen);
  return open ? <Shortcuts /> : null;
}

function Shortcuts() {
  const ctl = useController();
  const cardRef = useRef<HTMLDivElement>(null);
  const titleId = useTitleId();
  useReturnFocus();
  const close = () => {
    useUiStore.getState().setShortcuts(false);
    ctl.focusGrid();
  };
  // One row per command (first binding's label wins; alternates are appended).
  const commands = new Map<string, { title: string; kind: string; keys: string[] }>();
  for (const b of platformBindings(isMacPlatform())) {
    const cmd = ctl.registry.get(b.command);
    const entry = commands.get(b.command) ?? { title: cmd?.title ?? b.command, kind: cmd?.kind ?? "Other", keys: [] };
    entry.keys.push(b.label);
    commands.set(b.command, entry);
  }
  const groups = new Map<string, { keys: string; action: string }[]>();
  for (const g of GRID_KEYS) groups.set(g.group, [...(groups.get(g.group) ?? []), { keys: g.keys, action: gridText(g.action) }]);
  for (const c of commands.values()) groups.set(c.kind, [...(groups.get(c.kind) ?? []), { keys: c.keys.join("  "), action: c.title }]);
  return (
    <div className={styles.overlay} onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div
        className={styles.card}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        data-testid="shortcuts"
        tabIndex={-1}
        ref={(el) => {
          cardRef.current = el;
          if (el && !el.contains(document.activeElement)) el.focus();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            close();
            return;
          }
          if (cardRef.current) trapTab(cardRef.current, e, document.activeElement);
        }}
      >
        <header className={styles.head}>
          <h2 id={titleId} className={styles.title}>
            {t("cmd.help.shortcuts", "Keyboard shortcuts")}
          </h2>
          <button type="button" className={styles.close} aria-label={t("dialog.close", "Close")} onClick={close} data-testid="shortcuts-close">
            ×
          </button>
        </header>
        <div className={styles.columns}>
          {[...groups.entries()].map(([group, rows]) => (
            <section key={group} className={styles.group}>
              <h3 className={styles.h}>{groupText(group)}</h3>
              {rows.map((r) => (
                <div key={`${group}-${r.action}`} className={styles.row}>
                  <span>{r.action}</span>
                  <kbd className={styles.kbd}>{r.keys}</kbd>
                </div>
              ))}
            </section>
          ))}
        </div>
        <p className={styles.foot}>{t("shell.shortcutsFoot", "On Windows and Linux, ⌘ is Ctrl.")}</p>
      </div>
    </div>
  );
}
