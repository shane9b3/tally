/**
 * Top row: (traffic-light inset), document title, action pills, panel/theme
 * toggles, Share. Buttons can be hidden (View ▸ Toolbars, right-click ▸
 * Visible Buttons); those that do not fit go into "»" (chrome/ChromeUi.tsx).
 */
import { Fragment, useState, type ReactNode } from "react";
import { OverflowRow, useBarMenu } from "../chrome/ChromeUi";
import { TOOLBAR_RIGHT, TOOLBAR_SECTIONS, slotsOf, visibleSections } from "../chrome/chromeModel";
import { useDataStore } from "../stores/dataStore";
import { cx, useController } from "../context";
import { useDocStore } from "../stores/docStore";
import { useEditStore } from "../stores/editStore";
import { useUiStore } from "../stores/uiStore";
import { ChartIcon, FilterIcon, GlanceIcon, MoonIcon, RedoIcon, SaveIcon, SidebarIcon, SortIcon, UndoIcon } from "./icons";
import { Popover } from "./Popover";
import { t } from "../i18n";
import { useW56Store } from "../data/store";
import styles from "./Toolbar.module.css";

interface PillProps {
  command: string;
  label: string;
  icon?: ReactNode;
  children?: ReactNode;
  active?: boolean;
  /** Show only the icon (label becomes the tooltip). */
  iconOnly?: boolean;
  tooltip?: string;
}

function Pill({ command, label, icon, children, active, iconOnly, tooltip }: PillProps) {
  const ctl = useController();
  // Re-render on undo state / sheet / edit changes so `enabled` is fresh.
  useDocStore((s) => s.undo);
  useDocStore((s) => s.info?.sheets.length);
  useEditStore((s) => s.edit !== null);
  const cmd = ctl.registry.get(command);
  const enabled = ctl.registry.isEnabled(command);
  const tip = !enabled && cmd?.reason ? cmd.reason : [tooltip ?? label, cmd?.shortcut].filter(Boolean).join("  ");
  return (
    <button
      type="button"
      className={cx(styles.pill, iconOnly && styles.iconOnly, active && styles.active)}
      // With visible text of its own (Σ Add up) the button is named by that text (WCAG 2.5.3).
      aria-label={iconOnly || !children ? label : undefined}
      aria-pressed={active === undefined ? undefined : active}
      aria-disabled={!enabled}
      data-command={command}
      title={tip}
      onMouseDown={(e) => e.preventDefault() /* keep focus in the grid */}
      onClick={() => {
        if (ctl.registry.run(command)) ctl.focusGrid();
      }}
    >
      {icon}
      {iconOnly ? null : (children ?? <span>{label}</span>)}
    </button>
  );
}

export function Toolbar({ inset }: { inset: boolean }) {
  const ctl = useController();
  const info = useDocStore((s) => s.info);
  const sidebarOpen = useUiStore((s) => s.sidebarOpen);
  const glanceOpen = useUiStore((s) => s.glanceOpen);
  const themePref = useUiStore((s) => s.themePref);
  const [share, setShare] = useState<{ x: number; y: number } | null>(null);
  const [sortMenu, setSortMenu] = useState<{ x: number; y: number } | null>(null);
  const filtering = useDataStore((s) => s.filter !== null);
  useW56Store((s) => s.i18nRev);
  const hidden = useW56Store((s) => s.prefs.chrome.hidden.toolbar);
  const barMenu = useBarMenu("toolbar");

  const title = info?.title ?? t("app.untitled", "Untitled");
  const folder = info?.path ? info.path.split(/[\\/]/).slice(-2, -1)[0] : undefined;
  const subtitle = `${folder ?? t("shell.draft", "Draft")} · ${info?.dirty ? t("sidebar.edited", "Edited") : t("status.saved", "Saved")}`;

  const items: Record<string, ReactNode> = {
    save: <Pill command="file.save" label={t("cmd.file.save", "Save")} icon={<SaveIcon />} />,
    undo: <Pill command="edit.undo" label={t("cmd.edit.undo", "Undo")} icon={<UndoIcon />} />,
    redo: <Pill command="edit.redo" label={t("cmd.edit.redo", "Redo")} icon={<RedoIcon />} iconOnly />,
    sort: (
      <button
        type="button"
        className={styles.pill}
        aria-label={t("sort.title", "Sort")}
        aria-haspopup="menu"
        aria-expanded={sortMenu !== null}
        data-command="data.sort"
        title={t("shell.sortTip", "Sort the table by the active column")}
        onMouseDown={(e) => e.preventDefault()}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setSortMenu(sortMenu ? null : { x: r.left, y: r.bottom + 6 });
        }}
      >
        <SortIcon />
        <span>{t("sort.title", "Sort")}</span>
      </button>
    ),
    filter: <Pill command="data.filter" label={t("menu.data.filter", "Filter")} icon={<FilterIcon />} active={filtering} tooltip={filtering ? t("shell.autoFilterOff", "Remove the AutoFilter") : t("shell.autoFilterOn", "AutoFilter: a ▾ in each header")} />,
    addUp: (
      <Pill command="insert.addUp" label={t("chrome.item.addUp", "Add up")} tooltip={t("shell.addUpTip", "Add up the numbers above (Σ)")}>
        <span className={styles.glyph} aria-hidden="true">
          Σ
        </span>
        <span>{t("chrome.item.addUp", "Add up")}</span>
      </Pill>
    ),
    chart: <Pill command="insert.chart" label={t("chart.title", "Chart")} icon={<ChartIcon />} tooltip={t("shell.chartTip", "Insert a chart of the data around the selection")} />,
    theme: (
      <button
        type="button"
        className={cx(styles.pill, styles.iconOnly, styles.ghost)}
        aria-label={t("cmd.view.theme", "Theme")}
        title={themePref === "system" ? t("shell.themeTipSystem", "Theme: match system (click to change)") : themePref === "light" ? t("shell.themeTipLight", "Theme: light (click to change)") : t("shell.themeTipDark", "Theme: dark (click to change)")}
        data-testid="theme-toggle"
        data-theme-pref={themePref}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => ctl.registry.run("view.theme")}
      >
        <MoonIcon />
      </button>
    ),
    glance: <Pill command="view.glance" label={t("glance.title", "At a glance")} icon={<GlanceIcon />} iconOnly active={glanceOpen} tooltip={t("glance.title", "At a glance")} />,
    share: (
      <button
        type="button"
        className={styles.share}
        aria-haspopup="menu"
        aria-expanded={share !== null}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setShare(share ? null : { x: r.right, y: r.bottom + 6 });
        }}
      >
        {t("menu.file.share", "Share")}
      </button>
    ),
  };
  const slots = slotsOf(visibleSections(TOOLBAR_SECTIONS, hidden));

  return (
    <header className={cx(styles.toolbar, inset && styles.inset)} data-tauri-drag-region onContextMenu={barMenu.onContextMenu}>
      {!sidebarOpen ? (
        <button
          type="button"
          className={cx(styles.pill, styles.iconOnly, styles.ghost)}
          aria-label={t("shell.showSidebar", "Show sidebar")}
          title={`${t("shell.showSidebar", "Show sidebar")}  ⌘\\`}
          onClick={() => useUiStore.getState().setSidebar(true)}
        >
          <SidebarIcon />
        </button>
      ) : null}
      <div className={styles.title} data-tauri-drag-region>
        <div className={styles.name} data-tauri-drag-region data-testid="doc-title">
          {title}
        </div>
        <div className={styles.subtitle} data-tauri-drag-region data-testid="doc-subtitle">
          {subtitle}
        </div>
      </div>
      <nav className={styles.pills} aria-label={t("chrome.bar.toolbar", "Toolbar")} data-tauri-drag-region>
        <OverflowRow
          slots={slots}
          testId="toolbar-row"
          className={styles.row}
          dragRegion
          renderSlot={(s) => (s.kind === "sep" ? <span className={styles.sep} /> : typeof s.el === "string" ? items[s.el] : null)}
        />
      </nav>
      <div className={styles.spacer} data-tauri-drag-region />
      {TOOLBAR_RIGHT.filter((id) => !hidden.includes(id)).map((id) => (
        <Fragment key={id}>{items[id]}</Fragment>
      ))}
      {sortMenu ? (
        <Popover
          label={t("sort.title", "Sort")}
          testId="sort-menu"
          x={sortMenu.x}
          y={sortMenu.y}
          onClose={() => setSortMenu(null)}
          items={[
            { id: "asc", label: t("shell.ctxSortAsc", "Sort A → Z"), hint: t("shell.sortByActive", "by active column"), onSelect: () => ctl.registry.run("data.sortAsc") },
            { id: "desc", label: t("shell.ctxSortDesc", "Sort Z → A"), onSelect: () => ctl.registry.run("data.sortDesc") },
            { id: "custom", label: t("cmd.data.sort", "Sort…"), separator: true, onSelect: () => ctl.registry.run("data.sort") },
          ]}
        />
      ) : null}
      {share ? (
        <Popover
          label={t("menu.file.share", "Share")}
          x={share.x}
          y={share.y}
          alignRight
          onClose={() => setShare(null)}
          items={[
            { id: "xlsx", label: t("shell.exportXlsx", "Export XLSX…"), hint: ".xlsx", onSelect: () => ctl.registry.run("file.exportXlsx") },
            { id: "ods", label: t("cmd.file.exportOds", "Export ODS…"), hint: ".ods", onSelect: () => ctl.registry.run("file.exportOds") },
            { id: "csv", label: t("cmd.file.exportCsv", "Export as CSV…"), hint: ".csv", onSelect: () => ctl.registry.run("file.exportCsv") },
            { id: "pdf", label: t("cmd.file.exportPdf", "Export as PDF…"), hint: ".pdf", onSelect: () => ctl.registry.run("file.exportPdf") },
            { id: "saveAs", label: t("shell.saveCopy", "Save a copy…"), hint: "⇧⌘S", onSelect: () => ctl.registry.run("file.saveAs") },
          ]}
        />
      ) : null}
      {barMenu.menu}
    </header>
  );
}
