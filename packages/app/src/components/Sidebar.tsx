/**
 * Left sidebar: new workbook, the open workbook, "Open recent" (paths from
 * `recentFiles`, with when each was last open here, kept per viewer in
 * localStorage by path), templates, tip card.
 */
import { useEffect, useMemo, useState } from "react";
import { cx, useController } from "../context";
import { relativeTime } from "../hooks";
import type { TemplateInfo } from "@tally/bridge";
import { useDocStore } from "../stores/docStore";
import { PlusIcon } from "./icons";
import styles from "./Sidebar.module.css";
import { t } from "../i18n";
import { useW56Store } from "../data/store";

const DOTS = ["#2563eb", "#16a34a", "#f59e0b", "#db2777", "#7c3aed", "#0891b2"];
const TEMPLATE_DOTS: Record<string, string> = { invoices: "#16a34a", budget: "#f59e0b", stock: "#7c3aed" };
const SEEN_KEY = "tally.recentSeen";

function dotFor(name: string): string {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return DOTS[h % DOTS.length] ?? "#2563eb";
}

function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/** Display name of a recent file: its base name without the spreadsheet extension. */
export function displayName(path: string): string {
  return baseName(path).replace(/\.(ods|xlsx|csv)$/i, "");
}

/** The file format tag of a recent file (`XLSX`, `ODS`, `CSV`). */
export function formatTag(path: string): string {
  return /\.(ods|xlsx|csv)$/i.exec(path)?.[1]?.toUpperCase() ?? "";
}

/** The folder a recent file lives in (tooltip/secondary text). */
function folderOf(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts.length > 1 ? (parts[parts.length - 2] ?? "") : "";
}

/** Per-viewer convenience: when each recent file was last open here. */
function readSeen(): Record<string, number> {
  try {
    const raw = localStorage.getItem(SEEN_KEY);
    const v: unknown = raw ? JSON.parse(raw) : {};
    return typeof v === "object" && v !== null ? (v as Record<string, number>) : {};
  } catch {
    return {};
  }
}

function writeSeen(seen: Record<string, number>): void {
  try {
    localStorage.setItem(SEEN_KEY, JSON.stringify(seen));
  } catch {
    // Storage unavailable: relative times are optional.
  }
}

export function Sidebar({ inset }: { inset: boolean }) {
  const ctl = useController();
  const info = useDocStore((s) => s.info);
  const rev = useDocStore((s) => s.rev);
  const [recent, setRecent] = useState<string[]>([]);
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);
  const [userTemplates, setUserTemplates] = useState<TemplateInfo[]>([]);
  useW56Store((s) => s.i18nRev);
  const [editedAt, setEditedAt] = useState(() => Date.now());
  const [, setNow] = useState(0);

  useEffect(() => {
    ctl.shell.listTemplates().then(setTemplates, () => setTemplates([]));
  }, [ctl]);

  useEffect(() => {
    ctl.shell.recentFiles().then(setRecent, () => setRecent([]));
    if (info?.path) writeSeen({ ...readSeen(), [info.path]: Date.now() });
  }, [ctl, info?.path, rev]);

  useEffect(() => {
    void ctl.w56.listUserTemplates().then(setUserTemplates);
  }, [ctl, rev]);

  useEffect(() => {
    if (info?.dirty) setEditedAt(Date.now());
  }, [rev, info?.dirty]);

  useEffect(() => {
    const t = setInterval(() => setNow((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);

  const seen = useMemo(readSeen, [recent]);
  const others = recent.filter((p) => p !== info?.path).slice(0, 6);

  return (
    <aside className={styles.sidebar} aria-label="Sidebar">
      <div className={cx(styles.top, inset && styles.insetTop)} data-tauri-drag-region />
      <button type="button" className={styles.newButton} onClick={() => void ctl.newWorkbook()}>
        <PlusIcon />
        {t("cmd.file.new", "New sheet")}
      </button>

      <section className={styles.section}>
        <h3 className={styles.heading}>{t("app.name", "Tally")}</h3>
        <ul className={styles.list}>
          {info ? (
            <li>
              <button type="button" className={cx(styles.row, styles.current)} onClick={() => ctl.focusGrid()}>
                <span className={styles.dot} style={{ background: dotFor(info.title) }} />
                <span className={styles.rowTitle}>{info.title}</span>
                <span className={styles.meta}>{info.dirty ? relativeTime(editedAt) : "Open"}</span>
              </button>
            </li>
          ) : null}
        </ul>
      </section>

      {others.length > 0 ? (
        <section className={styles.section} data-testid="open-recent">
          <h3 className={styles.heading}>{t("menu.file.recent", "Open recent")}</h3>
          <ul className={styles.list}>
            {others.map((p) => {
              const t = seen[p];
              return (
                <li key={p}>
                  <button
                    type="button"
                    className={styles.row}
                    title={folderOf(p) ? `${p}\n(in ${folderOf(p)})` : p}
                    onClick={() => void ctl.openRecent(p)}
                  >
                    <span className={styles.dot} style={{ background: dotFor(baseName(p)) }} />
                    <span className={styles.rowTitle}>{displayName(p)}</span>
                    <span className={styles.meta} data-format={formatTag(p)}>
                      {[formatTag(p), t ? relativeTime(t) : ""].filter(Boolean).join(" · ")}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      ) : null}

      {userTemplates.length > 0 ? (
        <section className={styles.section} data-testid="user-templates">
          <h3 className={styles.heading}>{t("templates.title", "Templates")}</h3>
          <ul className={styles.list}>
            {userTemplates.map((u) => (
              <li key={u.id}>
                <button type="button" className={styles.row} title={u.description} onClick={() => void ctl.w56.newFromUserTemplate(u.id)}>
                  <span className={styles.square} style={{ borderColor: "#2563eb" }} />
                  <span className={styles.rowTitle}>{u.title}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {templates.length > 0 ? (
        <section className={styles.section}>
          <h3 className={styles.heading}>{t("templates.title", "Start from a template")}</h3>
          <ul className={styles.list}>
            {templates.map((t) => (
              <li key={t.id}>
                <button type="button" className={styles.row} title={t.description} onClick={() => void ctl.newFromTemplate(t.id)}>
                  <span className={styles.square} style={{ borderColor: TEMPLATE_DOTS[t.id] ?? "#9ca3af" }} />
                  <span className={styles.rowTitle}>{t.title}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <div className={styles.grow} />
      <div className={styles.tip}>
        <div className={styles.tipTitle}>Tip</div>
        Type <kbd className={styles.kbd}>=</kbd> in any cell and describe what you want, or press{" "}
        <kbd className={styles.kbd}>⌘K</kbd> to ask in plain English.
      </div>
    </aside>
  );
}
