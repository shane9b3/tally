/** Selected sheets, Sum / Count of the selection (debounced 50 ms) and the zoom menu. */
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

  const sum = stats?.sum ?? null;
  // With an error in the selection the engine sends no sum, only the
  // error's text in formattedSum (AOO shows the error).
  const sumText = stats?.formattedSum ?? (sum !== null ? String(sum) : null);
  return (
    <div className={styles.status} data-testid="status-bar">
      {groupSize > 1 ? (
        <span className={styles.group} data-testid="stat-sheets" role="status">
          {t("status.sheetsSelected", "{n} sheets selected", { n: groupSize })}
        </span>
      ) : null}
      {sumText !== null ? (
        <span className={styles.stat} data-testid="stat-sum">
          {t("status.sum", "Sum")} <b>{sumText}</b>
        </span>
      ) : null}
      {stats && stats.count > 0 ? (
        <span className={styles.stat} data-testid="stat-count">
          {t("status.count", "Count")} <b>{stats.count}</b>
        </span>
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
