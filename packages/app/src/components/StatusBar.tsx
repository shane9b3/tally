/** Sum / Count of the selection (debounced 50 ms) and the zoom menu. */
import type { SelectionStats } from "@tally/bridge";
import { useEffect, useRef, useState } from "react";
import { useController } from "../context";
import { ZOOM_PRESETS } from "../formats";
import { useDocStore } from "../stores/docStore";
import { toSelectionDto, useSelectionStore } from "../stores/selectionStore";
import { useUiStore } from "../stores/uiStore";
import { Popover } from "./Popover";
import styles from "./StatusBar.module.css";

export const STATS_DEBOUNCE_MS = 50;

export function StatusBar() {
  const ctl = useController();
  const selection = useSelectionStore((s) => s.selection);
  const doc = useDocStore((s) => s.info?.doc);
  const sheet = useDocStore((s) => s.activeSheet);
  const rev = useDocStore((s) => s.rev);
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
  return (
    <div className={styles.status} data-testid="status-bar">
      {sum !== null ? (
        <span className={styles.stat} data-testid="stat-sum">
          Sum <b>{stats?.formattedSum ?? String(sum)}</b>
        </span>
      ) : null}
      {stats && stats.count > 0 ? (
        <span className={styles.stat} data-testid="stat-count">
          Count <b>{stats.count}</b>
        </span>
      ) : null}
      <button
        type="button"
        className={styles.zoom}
        aria-haspopup="menu"
        aria-label="Zoom"
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
          label="Zoom"
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
