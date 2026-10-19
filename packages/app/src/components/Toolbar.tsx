/** Top row: (traffic-light inset), document title, action pills, Share. */
import { useState, type ReactNode } from "react";
import { COMING_SOON } from "../commands";
import { cx, useController } from "../context";
import { useActiveStyle } from "../hooks";
import { useDocStore } from "../stores/docStore";
import { useUiStore } from "../stores/uiStore";
import { ChartIcon, FilterIcon, RedoIcon, SidebarIcon, SortIcon, UndoIcon } from "./icons";
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
  // Re-render on undo state / sheet changes so `enabled` is fresh.
  useDocStore((s) => s.undo);
  useDocStore((s) => s.info?.sheets.length);
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
  const style = useActiveStyle();
  const [share, setShare] = useState<{ x: number; y: number } | null>(null);

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
        <Pill command="format.bold" label="Bold" iconOnly={false} active={style?.bold ?? false}>
          <span className={styles.glyphBold}>B</span>
        </Pill>
        <Pill command="format.italic" label="Italic" active={style?.italic ?? false}>
          <span className={styles.glyphItalic}>I</span>
        </Pill>
        <Pill command="format.currency" label="Currency" tooltip="Format as currency">
          <span className={styles.glyph}>$</span>
          <span>Currency</span>
        </Pill>
        <Pill command="format.percent" label="Percent" tooltip="Format as percent">
          <span className={styles.glyph}>%</span>
          <span>Percent</span>
        </Pill>
        <span className={styles.sep} />
        <Pill command="data.sort" label="Sort" icon={<SortIcon />} tooltip={COMING_SOON} />
        <Pill command="data.filter" label="Filter" icon={<FilterIcon />} tooltip={COMING_SOON} />
        <Pill command="insert.addUp" label="Add up" tooltip="Add up the numbers above (Σ)">
          <span className={styles.glyph}>Σ</span>
          <span>Add up</span>
        </Pill>
        <Pill command="insert.chart" label="Chart" icon={<ChartIcon />} tooltip={COMING_SOON} />
      </nav>
      <div className={styles.spacer} data-tauri-drag-region />
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
      {share ? (
        <Popover
          label="Share"
          x={share.x}
          y={share.y}
          alignRight
          onClose={() => setShare(null)}
          items={[
            { id: "csv", label: "Export as CSV…", onSelect: () => ctl.registry.run("file.exportCsv") },
            { id: "saveAs", label: "Save a copy…", hint: "⇧⌘S", onSelect: () => ctl.registry.run("file.saveAs") },
          ]}
        />
      ) : null}
    </header>
  );
}
