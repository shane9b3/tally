/** Top row: (traffic-light inset), document title, action pills, panel/theme toggles, Share. */
import { useState, type ReactNode } from "react";
import { COMING_SOON } from "../commands";
import { useDataStore } from "../stores/dataStore";
import { cx, useController } from "../context";
import { useDocStore } from "../stores/docStore";
import { useEditStore } from "../stores/editStore";
import { useUiStore } from "../stores/uiStore";
import { ChartIcon, FilterIcon, GlanceIcon, MoonIcon, RedoIcon, SidebarIcon, SortIcon, UndoIcon } from "./icons";
import { Popover } from "./Popover";
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
      aria-label={label}
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

  const title = info?.title ?? "Untitled";
  const folder = info?.path ? info.path.split(/[\\/]/).slice(-2, -1)[0] : undefined;
  const subtitle = `${folder ?? "Draft"} · ${info?.dirty ? "Edited" : "Saved"}`;

  return (
    <header className={cx(styles.toolbar, inset && styles.inset)} data-tauri-drag-region>
      {!sidebarOpen ? (
        <button
          type="button"
          className={cx(styles.pill, styles.iconOnly, styles.ghost)}
          aria-label="Show sidebar"
          title="Show sidebar  ⌘\"
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
      <nav className={styles.pills} aria-label="Toolbar">
        <Pill command="edit.undo" label="Undo" icon={<UndoIcon />} />
        <Pill command="edit.redo" label="Redo" icon={<RedoIcon />} iconOnly />
        <span className={styles.sep} />
        <button
          type="button"
          className={styles.pill}
          aria-label="Sort"
          aria-haspopup="menu"
          aria-expanded={sortMenu !== null}
          data-command="data.sort"
          title="Sort the table by the active column"
          onMouseDown={(e) => e.preventDefault()}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setSortMenu(sortMenu ? null : { x: r.left, y: r.bottom + 6 });
          }}
        >
          <SortIcon />
          <span>Sort</span>
        </button>
        <Pill command="data.filter" label="Filter" icon={<FilterIcon />} active={filtering} tooltip={filtering ? "Remove the AutoFilter" : "AutoFilter: a ▾ in each header"} />
        <Pill command="insert.addUp" label="Add up" tooltip="Add up the numbers above (Σ)">
          <span className={styles.glyph}>Σ</span>
          <span>Add up</span>
        </Pill>
        <Pill command="insert.chart" label="Chart" icon={<ChartIcon />} tooltip={COMING_SOON} />
      </nav>
      <div className={styles.spacer} data-tauri-drag-region />
      <button
        type="button"
        className={cx(styles.pill, styles.iconOnly, styles.ghost)}
        aria-label="Theme"
        title={`Theme: ${themePref === "system" ? "match system" : themePref} (click to change)`}
        data-testid="theme-toggle"
        data-theme-pref={themePref}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => ctl.registry.run("view.theme")}
      >
        <MoonIcon />
      </button>
      <Pill command="view.glance" label="At a glance" icon={<GlanceIcon />} iconOnly active={glanceOpen} tooltip="At a glance" />
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
        Share
      </button>
      {sortMenu ? (
        <Popover
          label="Sort"
          testId="sort-menu"
          x={sortMenu.x}
          y={sortMenu.y}
          onClose={() => setSortMenu(null)}
          items={[
            { id: "asc", label: "Sort A → Z", hint: "by active column", onSelect: () => ctl.registry.run("data.sortAsc") },
            { id: "desc", label: "Sort Z → A", onSelect: () => ctl.registry.run("data.sortDesc") },
            { id: "custom", label: "Sort…", separator: true, onSelect: () => ctl.registry.run("data.sort") },
          ]}
        />
      ) : null}
      {share ? (
        <Popover
          label="Share"
          x={share.x}
          y={share.y}
          alignRight
          onClose={() => setShare(null)}
          items={[
            { id: "xlsx", label: "Export XLSX…", hint: ".xlsx", onSelect: () => ctl.registry.run("file.exportXlsx") },
            { id: "ods", label: "Export ODS…", hint: ".ods", onSelect: () => ctl.registry.run("file.exportOds") },
            { id: "csv", label: "Export as CSV…", hint: ".csv", onSelect: () => ctl.registry.run("file.exportCsv") },
            { id: "saveAs", label: "Save a copy…", hint: "⇧⌘S", onSelect: () => ctl.registry.run("file.saveAs") },
          ]}
        />
      ) : null}
    </header>
  );
}
