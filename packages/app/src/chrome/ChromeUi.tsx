/**
 * Toolbar customisation UI: the overflow row ("»" for buttons that do not
 * fit), the bars' context menu (Visible Buttons, Customize Toolbars…, hide
 * the bar), the Customize Toolbars dialog and the minimal title strip shown
 * when the top toolbar is hidden.
 */
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { cx, useController } from "../context";
import { openCustomize, saveChrome } from "./chromeActions";
import { t } from "../i18n";
import { useW56Store } from "../data/store";
import { Check, W56Shell } from "../data/W56Shell";
import w56 from "../data/W56.module.css";
import { useDismiss, useFloating } from "../components/Popover";
import { useDocStore } from "../stores/docStore";
import { useUiStore } from "../stores/uiStore";
import { SidebarIcon } from "../components/icons";
export { openCustomize, saveChrome, toggleBar } from "./chromeActions";
import { BARS, BAR_LABELS, DEFAULT_CHROME, ITEM_LABELS, TOOLBARS, TOOLBAR_ITEMS, chooseLayout, isItemVisible, withBar, withItem, type BarId, type ChromePrefs, type RowLayout, type Slot, type ToolbarId } from "./chromeModel";
import styles from "./Chrome.module.css";

export const itemLabel = (id: string): string => t(`chrome.item.${id}`, ITEM_LABELS[id] ?? id);
export const barLabel = (id: BarId): string => t(`chrome.bar.${id}`, BAR_LABELS[id]);

/** The chrome prefs (re-renders on change). */
export function useChrome(): ChromePrefs {
  return useW56Store((s) => s.prefs.chrome);
}

// ---------------------------------------------------------------------------
// Overflow row
// ---------------------------------------------------------------------------

/** Width reserved for the "»" button (px). */
const MORE_W = 30;

interface OverflowRowProps {
  slots: readonly Slot[];
  /** Draws a slot; `compact` drops the labels of wide buttons (icon-only). */
  renderSlot(slot: Slot, compact: boolean): ReactNode;
  /** Slots that are narrower when compact; the row goes compact before anything overflows. */
  compactable?: (slot: Slot) => boolean;
  className?: string | undefined;
  testId: string;
  /** The empty part of the row drags the window (macOS overlay title bar). */
  dragRegion?: boolean;
}

/**
 * A row of toolbar slots: when the row is too narrow, wide buttons first
 * lose their labels (compact), then the slots that still do not fit go
 * behind "»", which opens a panel with them. Slot widths are measured
 * while shown and remembered per mode, so the row decides without drawing
 * hidden buttons twice; labels come back when there is room.
 */
export function OverflowRow({ slots, renderSlot, compactable, className, testId, dragRegion }: OverflowRowProps) {
  const ctl = useController();
  const ref = useRef<HTMLDivElement>(null);
  const fullW = useRef(new Map<string, number>());
  const compactW = useRef(new Map<string, number>());
  const [layout, setLayout] = useState<RowLayout>({ compact: false, n: slots.length });
  const [tick, setTick] = useState(0);
  const [panel, setPanel] = useState<{ x: number; y: number } | null>(null);
  const count = Math.min(layout.n, slots.length);
  const canCompact = compactable !== undefined && slots.some((s) => compactable(s));

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const into = layout.compact ? compactW.current : fullW.current;
    for (const c of el.querySelectorAll<HTMLElement>(":scope > [data-slot]")) {
      const w = c.getBoundingClientRect().width;
      if (w > 0) into.set(c.dataset.slot ?? "", w);
    }
    const cs = getComputedStyle(el);
    const gap = parseFloat(cs.columnGap) || 0;
    const avail = el.clientWidth - (parseFloat(cs.paddingLeft) || 0) - (parseFloat(cs.paddingRight) || 0);
    if (avail <= 0) return;
    const next = chooseLayout(
      slots.map((s) => fullW.current.get(s.key) ?? null),
      canCompact ? slots.map((s) => compactW.current.get(s.key) ?? null) : null,
      slots.map((s) => s.kind === "sep"),
      avail,
      gap,
      MORE_W,
    );
    if (next.compact !== layout.compact || next.n !== layout.n) setLayout(next);
  });

  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return undefined;
    const ro = new ResizeObserver(() => setTick((x) => x + 1));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  void tick;

  const rest = slots.slice(count).filter((s, i) => !(i === 0 && s.kind === "sep"));
  useEffect(() => {
    if (rest.length === 0 && panel) setPanel(null);
  }, [rest.length, panel]);
  const toggle = (btn: HTMLElement) => {
    const r = btn.getBoundingClientRect();
    setPanel(panel ? null : { x: r.right, y: r.bottom + 6 });
  };
  return (
    <div ref={ref} className={cx(styles.row, className)} data-testid={testId} data-compact={layout.compact ? "true" : undefined} data-tauri-drag-region={dragRegion ? true : undefined}>
      {slots.slice(0, count).map((s) => (
        <div key={s.key} data-slot={s.key} className={styles.slot}>
          {renderSlot(s, layout.compact)}
        </div>
      ))}
      {rest.length > 0 ? (
        <button
          type="button"
          className={styles.more}
          aria-label={t("chrome.more", "More buttons")}
          aria-haspopup="true"
          aria-expanded={panel !== null}
          title={t("chrome.more", "More buttons")}
          data-testid={`${testId}-more`}
          onMouseDown={(e) => {
            // Mouse: open on press and keep the grid's focus.
            e.preventDefault();
            e.stopPropagation();
            toggle(e.currentTarget);
          }}
          onClick={(e) => {
            // Keyboard (Enter / Space): a click without a mouse press.
            if (e.detail === 0) toggle(e.currentTarget);
          }}
        >
          »
        </button>
      ) : null}
      {panel && rest.length > 0 ? (
        <OverflowPanel
          x={panel.x}
          y={panel.y}
          slots={rest}
          renderSlot={(s) => renderSlot(s, false)}
          testId={`${testId}-overflow`}
          onClose={(viaKey) => {
            setPanel(null);
            if (viaKey) ctl.focusGrid();
          }}
        />
      ) : null}
    </div>
  );
}

/** The "»" panel: closes on an outside press, or on Esc unless a combo list inside is open (Esc closes that first). */
function OverflowPanel({ x, y, slots, renderSlot, testId, onClose }: { x: number; y: number; slots: readonly Slot[]; renderSlot(slot: Slot): ReactNode; testId: string; onClose(viaKey: boolean): void }) {
  const { ref, style } = useFloating(x, y, false, true);
  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose(false);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const a = document.activeElement;
      if (a instanceof HTMLElement && a.getAttribute("role") === "combobox" && a.getAttribute("aria-expanded") === "true" && ref.current?.contains(a)) return;
      e.stopPropagation();
      e.preventDefault();
      onClose(true);
    };
    window.addEventListener("mousedown", down, true);
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("mousedown", down, true);
      window.removeEventListener("keydown", key, true);
    };
  }, [ref, onClose]);
  return (
    <div ref={ref} className={styles.panel} style={style} role="toolbar" aria-label={t("chrome.more", "More buttons")} data-testid={testId}>
      {slots.map((s) =>
        s.kind === "sep" ? (
          <span key={s.key} className={styles.panelSep} />
        ) : (
          <div key={s.key} className={styles.slot}>
            {renderSlot(s)}
          </div>
        ),
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// The bars' context menu
// ---------------------------------------------------------------------------

/** Right-click on a bar: Visible Buttons ▸ (checks), Customize Toolbars…, the bars. */
export function BarMenu({ bar, x, y, onClose }: { bar: ToolbarId; x: number; y: number; onClose(): void }) {
  const ctl = useController();
  const chrome = useChrome();
  const { ref, style } = useFloating(x, y);
  useDismiss(ref, onClose);
  const [sub, setSub] = useState(false);
  const subRef = useRef<HTMLButtonElement>(null);
  const [subAt, setSubAt] = useState<{ x: number; y: number } | null>(null);
  const subMenuRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.querySelector<HTMLButtonElement>("button")?.focus();
  }, [ref]);
  // The flyout opens to the left when it would leave the window.
  useLayoutEffect(() => {
    const el = subMenuRef.current;
    const anchor = subRef.current?.closest("[role=menu]")?.getBoundingClientRect();
    if (!el || !anchor || !subAt) return;
    const r = el.getBoundingClientRect();
    if (r.right > window.innerWidth - 8 && subAt.x > anchor.left) setSubAt({ x: Math.max(8, anchor.left - r.width - 2), y: subAt.y });
  }, [subAt, sub]);
  const openSub = () => {
    const r = subRef.current?.getBoundingClientRect();
    if (r) setSubAt({ x: r.right + 2, y: r.top - 4 });
    setSub(true);
  };
  return (
    <div ref={ref} className={styles.menu} style={style} role="menu" aria-label={barLabel(bar)} data-testid="bar-menu">
      <button
        ref={subRef}
        type="button"
        role="menuitem"
        aria-haspopup="menu"
        aria-expanded={sub}
        className={styles.item}
        data-item="visible"
        onMouseEnter={openSub}
        onClick={openSub}
        onKeyDown={(e) => {
          if (e.key === "ArrowRight") {
            e.preventDefault();
            openSub();
          }
        }}
      >
        <span className={styles.check} />
        <span className={styles.label}>{t("chrome.visibleButtons", "Visible Buttons")}</span>
        <span className={styles.hint}>▸</span>
      </button>
      <button
        type="button"
        role="menuitem"
        className={styles.item}
        data-item="customize"
        onMouseEnter={() => setSub(false)}
        onClick={() => {
          onClose();
          openCustomize();
        }}
      >
        <span className={styles.check} />
        <span className={styles.label}>{t("cmd.view.customizeToolbars", "Customize Toolbars…")}</span>
      </button>
      <div className={styles.separator} role="separator" />
      {BARS.map((b) => (
        <button
          key={b}
          type="button"
          role="menuitemcheckbox"
          aria-checked={chrome.bars[b]}
          className={styles.item}
          data-item={`bar-${b}`}
          onMouseEnter={() => setSub(false)}
          onClick={() => {
            onClose();
            void saveChrome(ctl, withBar(chrome, b, !chrome.bars[b]));
          }}
        >
          <span className={styles.check}>{chrome.bars[b] ? "✓" : ""}</span>
          <span className={styles.label}>{barLabel(b)}</span>
        </button>
      ))}
      {sub && subAt ? (
        <div
          ref={subMenuRef}
          className={styles.menu}
          style={{ left: subAt.x, top: subAt.y, maxHeight: `calc(100vh - ${Math.round(subAt.y) + 8}px)`, overflowY: "auto" }}
          role="menu"
          aria-label={t("chrome.visibleButtons", "Visible Buttons")}
          data-testid="visible-buttons"
          onKeyDown={(e) => {
            if (e.key === "ArrowLeft") {
              e.preventDefault();
              setSub(false);
              subRef.current?.focus();
            }
          }}
        >
          {TOOLBAR_ITEMS[bar].map((id) => {
            const on = isItemVisible(chrome, bar, id);
            return (
              <button
                key={id}
                type="button"
                role="menuitemcheckbox"
                aria-checked={on}
                className={styles.item}
                data-item={id}
                onClick={() => void saveChrome(ctl, withItem(chrome, bar, id, !on))}
              >
                <span className={styles.check}>{on ? "✓" : ""}</span>
                <span className={styles.label}>{itemLabel(id)}</span>
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

/** Context-menu state for a bar. */
export function useBarMenu(bar: ToolbarId) {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  return {
    onContextMenu: (e: React.MouseEvent) => {
      // Inputs keep their own menu (cut / copy / paste).
      if ((e.target as HTMLElement).closest("input, textarea")) return;
      e.preventDefault();
      setAt({ x: e.clientX, y: e.clientY });
    },
    menu: at ? <BarMenu bar={bar} x={at.x} y={at.y} onClose={() => setAt(null)} /> : null,
  };
}

// ---------------------------------------------------------------------------
// Customize Toolbars dialog
// ---------------------------------------------------------------------------

export function CustomizeToolbarsDialog() {
  const ctl = useController();
  const saved = useChrome();
  const [c, setC] = useState<ChromePrefs>(saved);
  useW56Store((s) => s.i18nRev);
  return (
    <W56Shell
      title={t("chrome.customizeTitle", "Customize Toolbars")}
      ok={t("dialog.ok", "OK")}
      size="wide"
      testId="customize-toolbars"
      extra={
        <button type="button" className={w56.chip} onClick={() => setC(DEFAULT_CHROME)} data-testid="customize-reset">
          {t("chrome.reset", "Reset to Default")}
        </button>
      }
      onSubmit={() => saveChrome(ctl, c)}
    >
      <div className={w56.section}>{t("chrome.bars", "Bars")}</div>
      <div className={styles.checks}>
        {BARS.map((b) => (
          <Check key={b} label={barLabel(b)} checked={c.bars[b]} onChange={(v) => setC(withBar(c, b, v))} testId={`customize-bar-${b}`} />
        ))}
      </div>
      {TOOLBARS.map((bar) => (
        <div key={bar}>
          <div className={w56.section}>{t("chrome.buttonsOf", "Buttons: {bar}", { bar: barLabel(bar) })}</div>
          <div className={styles.checks}>
            {TOOLBAR_ITEMS[bar].map((id) => (
              <Check key={id} label={itemLabel(id)} checked={isItemVisible(c, bar, id)} onChange={(v) => setC(withItem(c, bar, id, v))} testId={`customize-${bar}-${id}`} />
            ))}
          </div>
        </div>
      ))}
      <p className={styles.note}>{t("chrome.hint", "Hidden buttons stay available in the menus, with their shortcuts and in the command palette (⌘K).")}</p>
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Title strip (top toolbar hidden)
// ---------------------------------------------------------------------------

/** The document title on a thin strip that still drags the window and clears the traffic lights. */
export function TitleStrip({ inset, trafficLights }: { inset: boolean; trafficLights: boolean }) {
  const info = useDocStore((s) => s.info);
  const sidebarOpen = useUiStore((s) => s.sidebarOpen);
  const { onContextMenu, menu } = useBarMenu("toolbar");
  const title = info?.title ?? t("app.untitled", "Untitled");
  return (
    <header className={cx(styles.strip, trafficLights && styles.stripTall, inset && styles.stripInset)} data-tauri-drag-region data-testid="title-strip" onContextMenu={onContextMenu}>
      {!sidebarOpen ? (
        <button type="button" className={styles.stripBtn} aria-label="Show sidebar" title="Show sidebar  ⌘\" onClick={() => useUiStore.getState().setSidebar(true)}>
          <SidebarIcon />
        </button>
      ) : null}
      <span className={styles.stripTitle} data-tauri-drag-region>
        {title}
        {info?.dirty ? <span className={styles.stripEdited}> · {t("chrome.edited", "Edited")}</span> : null}
      </span>
      {menu}
    </header>
  );
}
