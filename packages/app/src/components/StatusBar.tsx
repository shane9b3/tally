/**
 * Selected sheets, the selection mode (STD / EXT / ADD), one function of
 * the selection (debounced 50 ms; chosen from the field's menu, as in AOO)
 * and the zoom menu.
 */
import type { SelectionStats } from "@tally/bridge";
import { useEffect, useRef, useState } from "react";
import { useController } from "../context";
import { ZOOM_PRESETS } from "../formats";
import { useDocStore } from "../stores/docStore";
import { toSelectionDto, useSelectionStore } from "../stores/selectionStore";
import { useUiStore } from "../stores/uiStore";
import { Popover } from "./Popover";
import { t } from "../i18n";
import { useW56Store } from "../data/store";
import { STATUS_FUNCTIONS, nextSelectionMode, selectionModeHint, selectionModeLabel, selectionModeOf, statusFunctionMenuLabel, statusFunctionText } from "../statusFunction";
import styles from "./StatusBar.module.css";

export const STATS_DEBOUNCE_MS = 50;

export function StatusBar() {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const selection = useSelectionStore((s) => s.selection);
  const doc = useDocStore((s) => s.info?.doc);
  const sheet = useDocStore((s) => s.activeSheet);
  const rev = useDocStore((s) => s.rev);
  const groupSize = useDocStore((s) => s.groupSheets.length);
  const zoom = useUiStore((s) => s.zoom);
  const [stats, setStats] = useState<SelectionStats | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [funcMenu, setFuncMenu] = useState<{ x: number; y: number } | null>(null);
  const fn = useW56Store((s) => s.prefs.statusFunction);
  const mode = selectionModeOf(
    useW56Store((s) => s.extendMode),
    useW56Store((s) => s.addMode),
  );
  const seq = useRef(0);

  useEffect(() => {
    if (doc === undefined) return undefined;
    const my = ++seq.current;
    const t = setTimeout(() => {
      ctl.client.selectionStats(doc, toSelectionDto(selection, sheet)).then(
        (s) => {
          if (my === seq.current) setStats(s);
        },
        () => {
          if (my === seq.current) setStats(null);
        },
      );
    }, STATS_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [ctl, doc, sheet, selection, rev]);

  const funcText = statusFunctionText(fn, stats);
  const funcLabel = t("status.function", "Selection function");
  return (
    <div className={styles.status} data-testid="status-bar">
      {groupSize > 1 ? (
        <span className={styles.group} data-testid="stat-sheets" role="status">
          {t("status.sheetsSelected", "{n} sheets selected", { n: groupSize })}
        </span>
      ) : null}
      {/* AOO: a click steps STD → EXT → ADD → STD (SvxSelectionModeControl::Click). */}
      <button
        type="button"
        className={styles.zoom}
        title={selectionModeHint(mode)}
        aria-label={`${t("status.selMode", "Selection mode")}: ${selectionModeLabel(mode)}, ${selectionModeHint(mode)}`}
        data-testid="stat-selmode"
        data-mode={mode}
        onClick={() => {
          const next = nextSelectionMode(mode);
          const st = useW56Store.getState();
          st.setExtendMode(next === "ext");
          st.setAddMode(next === "add");
          ctl.focusGrid();
        }}
      >
        {selectionModeLabel(mode)}
      </button>
      {/* AOO opens the menu on a right click only; here a click opens it too, so the keyboard can reach it. */}
      <button
        type="button"
        className={`${styles.zoom} ${styles.func}`}
        aria-haspopup="menu"
        aria-label={`${funcLabel}: ${funcText === "" ? statusFunctionMenuLabel("none") : funcText}`}
        data-testid="stat-func"
        data-function={fn}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setFuncMenu(funcMenu ? null : { x: r.right, y: r.top - 6 });
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          setFuncMenu({ x: e.clientX, y: e.currentTarget.getBoundingClientRect().top - 6 });
        }}
      >
        {funcText}
      </button>
      {funcMenu ? (
        <Popover
          label={funcLabel}
          x={funcMenu.x}
          y={funcMenu.y}
          above
          alignRight
          testId="stat-func-menu"
          onClose={() => setFuncMenu(null)}
          items={STATUS_FUNCTIONS.map((f) => ({
            id: f,
            label: statusFunctionMenuLabel(f),
            checked: f === fn,
            onSelect: () => {
              const st = useW56Store.getState();
              if (f !== st.prefs.statusFunction) void ctl.w56.savePrefs({ ...st.prefs, statusFunction: f });
              ctl.focusGrid();
            },
          }))}
        />
      ) : null}
      <button
        type="button"
        className={styles.zoom}
        aria-haspopup="menu"
        aria-label={`${t("toolbar.zoom", "Zoom")} ${Math.round(zoom * 100)}%`}
        data-testid="zoom"
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenu(menu ? null : { x: r.right, y: r.top - 6 });
        }}
      >
        {Math.round(zoom * 100)}%
      </button>
      {menu ? (
        <Popover
          label={t("status.zoom", "Zoom")}
          x={menu.x}
          y={menu.y}
          above
          alignRight
          onClose={() => setMenu(null)}
          items={ZOOM_PRESETS.map((z) => ({
            id: String(z),
            label: `${Math.round(z * 100)}%`,
            checked: Math.abs(z - zoom) < 1e-6,
            onSelect: () => {
              useUiStore.getState().setZoom(z);
              ctl.focusGrid();
            },
          }))}
        />
      ) : null}
    </div>
  );
}
