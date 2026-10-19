/** ⌘/: the keyboard shortcut reference (the keymap table plus the grid's own keys). */
import { useController } from "../context";
import { BINDINGS, GRID_KEYS } from "../keymap";
import { useUiStore } from "../stores/uiStore";
import styles from "./ShortcutsModal.module.css";

export function ShortcutsModal() {
  const open = useUiStore((s) => s.shortcutsOpen);
  const ctl = useController();
  if (!open) return null;
  const close = () => {
    useUiStore.getState().setShortcuts(false);
    ctl.focusGrid();
  };
  // One row per command (first binding's label wins; alternates are appended).
  const commands = new Map<string, { title: string; kind: string; keys: string[] }>();
  for (const b of BINDINGS) {
    const cmd = ctl.registry.get(b.command);
    const entry = commands.get(b.command) ?? { title: cmd?.title ?? b.command, kind: cmd?.kind ?? "Other", keys: [] };
    entry.keys.push(b.label);
    commands.set(b.command, entry);
  }
  const groups = new Map<string, { keys: string; action: string }[]>();
  for (const g of GRID_KEYS) groups.set(g.group, [...(groups.get(g.group) ?? []), { keys: g.keys, action: g.action }]);
  for (const c of commands.values()) groups.set(c.kind, [...(groups.get(c.kind) ?? []), { keys: c.keys.join("  "), action: c.title }]);
  return (
    <div className={styles.overlay} onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div
        className={styles.card}
        role="dialog"
        aria-label="Keyboard shortcuts"
        data-testid="shortcuts"
        tabIndex={-1}
        ref={(el) => el?.focus()}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            close();
          }
        }}
      >
        <header className={styles.head}>
          <h2 className={styles.title}>Keyboard shortcuts</h2>
          <button type="button" className={styles.close} aria-label="Close" onClick={close}>
            ×
          </button>
        </header>
        <div className={styles.columns}>
          {[...groups.entries()].map(([group, rows]) => (
            <section key={group} className={styles.group}>
              <h3 className={styles.h}>{group}</h3>
              {rows.map((r) => (
                <div key={`${group}-${r.action}`} className={styles.row}>
                  <span>{r.action}</span>
                  <kbd className={styles.kbd}>{r.keys}</kbd>
                </div>
              ))}
            </section>
          ))}
        </div>
        <p className={styles.foot}>On Windows and Linux, ⌘ is Ctrl.</p>
      </div>
    </div>
  );
}
