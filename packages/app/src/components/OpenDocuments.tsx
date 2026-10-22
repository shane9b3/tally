/**
 * The sidebar's "Editing" section: every open document of the app (tabs.ts),
 * this window's tabs first, then the other windows' documents, in the row
 * format of "Open recent". The document this webview shows is highlighted.
 * Click shows a document (another window comes to the front), × closes it
 * (the shell runs that document's own save prompt), drag reorders this
 * window's tabs, right-click offers Close / Close Others / Move to New
 * Window (the last two for this window's rows only). Keyboard: a listbox
 * with roving focus, ↑/↓/Home/End, ↩ shows, ⌫/Delete closes.
 *
 * Without a desktop tab shell (the browser build) it shows the one document
 * as a single row, as before.
 */
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { cx, useController } from "../context";
import { relativeTime } from "../hooks";
import { t } from "../i18n";
import { useDocStore } from "../stores/docStore";
import { canDetach, editedSince, moveIndex, openDocuments, tabTooltip, useTabsStore, type TabInfo } from "../tabs";
import { CloseSmallIcon } from "./icons";
import { Popover } from "./Popover";
import styles from "./Sidebar.module.css";

export interface OpenDocumentsProps {
  /** Row dot colour for a name (shared with "Open recent"). */
  dotFor: (name: string) => string;
  /** File-format tag of a path (`ODS`, `XLSX`, `CSV`). */
  formatTag: (path: string) => string;
}

export function OpenDocuments({ dotFor, formatTag }: OpenDocumentsProps) {
  const ctl = useController();
  const info = useDocStore((s) => s.info);
  const state = useTabsStore((s) => s.state);
  const self = useTabsStore((s) => s.self);
  const [since, setSince] = useState<Record<string, number>>({});
  const [focus, setFocus] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ label: string; x: number; y: number } | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  /** Drop before this tab of this window; `""` = after the last one. */
  const [dropAt, setDropAt] = useState<string | null>(null);
  const refs = useRef(new Map<string, HTMLLIElement>());
  const [, setNow] = useState(0);

  const untitled = t("app.untitled", "Untitled");
  const shell = ctl.tabs.available && state !== null;
  // Without a shell: this document as the only row.
  const docs: TabInfo[] = shell
    ? openDocuments(state)
    : info
      ? [{ label: self ?? "", window: "", title: info.title, edited: info.dirty, path: info.path ?? null }]
      : [];
  const here = new Set(state?.tabs.map((d) => d.label) ?? []);
  const current = shell ? self : (docs[0]?.label ?? null);

  const edits = docs.filter((d) => d.edited).map((d) => d.label).join("|");
  useEffect(() => {
    setSince((prev) => editedSince(prev, docs, Date.now()));
    // `docs` is rebuilt each render; the edited set is what matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [edits]);

  useEffect(() => {
    const id = setInterval(() => setNow((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);

  const titleOf = (d: TabInfo): string => d.title || untitled;
  /** "Edited · 5 min ago" (tooltip, the dot's label); the row's meta shows only the time. */
  const editedText = (d: TabInfo): string => {
    const at = since[d.label];
    return [t("sidebar.edited", "Edited"), at === undefined ? "" : relativeTime(at)].filter(Boolean).join(" · ");
  };
  const metaOf = (d: TabInfo): string => {
    if (d.edited) {
      const at = since[d.label];
      return at === undefined ? "" : relativeTime(at);
    }
    return d.path ? formatTag(d.path) || t("status.saved", "Saved") : "";
  };
  const focused = docs.some((d) => d.label === focus) ? focus : current;

  const moveFocus = (d: TabInfo | undefined) => {
    if (!d) return;
    setFocus(d.label);
    refs.current.get(d.label)?.focus();
  };

  const show = (d: TabInfo) => {
    if (!shell) ctl.focusGrid();
    else if (d.label === self) ctl.focusGrid();
    else void ctl.tabs.activate(d.label);
  };

  const onKey = (e: KeyboardEvent<HTMLLIElement>, d: TabInfo, i: number) => {
    switch (e.key) {
      case "ArrowDown":
        moveFocus(docs[Math.min(i + 1, docs.length - 1)]);
        break;
      case "ArrowUp":
        moveFocus(docs[Math.max(i - 1, 0)]);
        break;
      case "Home":
        moveFocus(docs[0]);
        break;
      case "End":
        moveFocus(docs[docs.length - 1]);
        break;
      case "Enter":
      case " ":
        show(d);
        break;
      case "Delete":
      case "Backspace":
        if (shell) void ctl.tabs.close(d.label);
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  };

  const drop = (target: string | null) => {
    if (dragging !== null && state) {
      const index = moveIndex(state, dragging, target);
      if (index !== null) void ctl.tabs.move(dragging, index);
    }
    setDragging(null);
    setDropAt(null);
  };

  const menuHere = menu !== null && here.has(menu.label);
  return (
    <section className={styles.section} data-testid="open-documents">
      <h3 className={styles.heading}>{t("sidebar.open", "Editing")}</h3>
      <ul className={styles.list} role="listbox" aria-label={t("tabs.label", "Open documents")}>
        {docs.map((d, i) => {
          const inWindow = here.has(d.label);
          const active = d.label === current;
          return (
            <li
              key={d.label || "doc"}
              ref={(el) => {
                if (el) refs.current.set(d.label, el);
                else refs.current.delete(d.label);
              }}
              role="option"
              aria-selected={active}
              tabIndex={d.label === focused ? 0 : -1}
              title={d.edited ? `${tabTooltip(d, untitled)}\n${editedText(d)}` : tabTooltip(d, untitled)}
              data-testid={`open-doc-${d.label || "current"}`}
              data-window={d.window || undefined}
              data-edited={d.edited || undefined}
              className={cx(
                styles.row,
                styles.openRow,
                active && styles.current,
                dragging === d.label && styles.dragging,
                dragging !== null && dropAt === d.label && styles.dropBefore,
                dragging !== null && dropAt === "" && i === docs.length - 1 && styles.dropAfter,
              )}
              draggable={shell && inWindow}
              onClick={() => {
                setFocus(d.label);
                show(d);
              }}
              onKeyDown={(e) => onKey(e, d, i)}
              onContextMenu={(e) => {
                if (!shell) return;
                e.preventDefault();
                setMenu({ label: d.label, x: e.clientX, y: e.clientY + 4 });
              }}
              onDragStart={(e) => {
                setDragging(d.label);
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/x-tally-tab", d.label);
              }}
              onDragOver={(e) => {
                // Reordering stays within this window's tabs.
                if (dragging === null || !state || !inWindow) return;
                e.preventDefault();
                const r = e.currentTarget.getBoundingClientRect();
                const j = state.tabs.findIndex((x) => x.label === d.label);
                setDropAt(e.clientY < r.top + r.height / 2 ? d.label : (state.tabs[j + 1]?.label ?? ""));
              }}
              onDragEnd={() => {
                setDragging(null);
                setDropAt(null);
              }}
              onDrop={(e) => {
                if (!inWindow) return;
                e.preventDefault();
                drop(dropAt === "" ? null : (dropAt ?? d.label));
              }}
            >
              <span className={styles.dot} style={{ background: dotFor(titleOf(d)) }} />
              <span className={styles.rowTitle}>{titleOf(d)}</span>
              {d.edited ? <span className={styles.editedDot} role="img" aria-label={editedText(d)} /> : null}
              <span className={styles.meta}>{metaOf(d)}</span>
              {shell ? (
                <button
                  type="button"
                  className={styles.closeDoc}
                  tabIndex={-1}
                  aria-label={`${t("tabs.close", "Close")}: ${titleOf(d)}`}
                  data-testid={`open-doc-close-${d.label}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    void ctl.tabs.close(d.label);
                  }}
                >
                  <CloseSmallIcon />
                </button>
              ) : null}
            </li>
          );
        })}
      </ul>
      {menu ? (
        <Popover
          label={t("tabs.label", "Open documents")}
          testId="open-doc-menu"
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            { id: "close", label: t("tabs.close", "Close"), onSelect: () => void ctl.tabs.close(menu.label) },
            { id: "closeOthers", label: t("tabs.closeOthers", "Close Others"), disabled: !menuHere || (state?.tabs.length ?? 0) < 2, onSelect: () => void ctl.tabs.closeOthers(menu.label) },
            { id: "detach", label: t("tabs.moveToNewWindow", "Move to New Window"), separator: true, disabled: !menuHere || !canDetach(state), onSelect: () => void ctl.tabs.detach(menu.label) },
          ]}
        />
      ) : null}
    </section>
  );
}
