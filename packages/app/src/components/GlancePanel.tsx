/**
 * "At a glance" right panel: KPI tiles, counts per category and a bar
 * chart for the data region around the active cell (see glance.ts).
 * Recomputed (debounced 300 ms) on every engine result / doc-changed
 * (`rev`), active-cell or sheet change. Reads at most GLANCE_CELL_CAP cells.
 */
import type { CellView } from "@tally/bridge";
import { useEffect, useRef, useState } from "react";
import { useController } from "../context";
import { analyzeGlance, cappedRange, currentRegion, type GlanceResult } from "../glance";
import { rangeAddress } from "../sumRange";
import { useDocStore } from "../stores/docStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore } from "../stores/uiStore";
import styles from "./GlancePanel.module.css";

export const GLANCE_DEBOUNCE_MS = 300;

interface State {
  result: GlanceResult | null;
  sampled: boolean;
  loading: boolean;
}

export function GlancePanel() {
  const ctl = useController();
  const doc = useDocStore((s) => s.info?.doc);
  const sheet = useDocStore((s) => s.activeSheet);
  const rev = useDocStore((s) => s.rev);
  const active = useSelectionStore((s) => s.selection.active);
  const [state, setState] = useState<State>({ result: null, sampled: false, loading: true });
  const seq = useRef(0);

  useEffect(() => {
    if (doc === undefined) return undefined;
    const my = ++seq.current;
    const t = setTimeout(() => {
      void (async () => {
        try {
          const layout = await ctl.client.sheetLayout(doc, sheet);
          if (!layout.used) {
            if (my === seq.current) setState({ result: null, sampled: false, loading: false });
            return;
          }
          const { range, sampled } = cappedRange(layout.used);
          const [block] = await ctl.client.cells(doc, sheet, [range]);
          const cells = new Map<string, CellView>();
          for (const v of block?.cells ?? []) cells.set(`${v.r},${v.c}`, v);
          const get = (r: number, c: number) => cells.get(`${r},${c}`);
          const inside = active.r >= range.r0 && active.r <= range.r1 && active.c >= range.c0 && active.c <= range.c1;
          const region = (inside ? currentRegion(get, active, range) : null) ?? currentRegion(get, { r: range.r0, c: range.c0 }, range) ?? range;
          // Number format per column: the style of its first numeric body cell.
          const firstNum = new Map<number, number>();
          for (const v of block?.cells ?? []) {
            if (v.r > region.r0 && v.kind === "number" && !firstNum.has(v.c)) firstNum.set(v.c, v.style);
          }
          const ids = [...new Set(firstNum.values())];
          const styles = ids.length > 0 ? await ctl.client.styles(doc, ids) : [];
          const fmtById = new Map(styles.map((s) => [s.id, s.numberFormat]));
          const formatOf = (c: number) => fmtById.get(firstNum.get(c) ?? -1) ?? "General";
          const result = analyzeGlance(get, region, formatOf);
          if (my === seq.current) setState({ result, sampled, loading: false });
        } catch {
          if (my === seq.current) setState({ result: null, sampled: false, loading: false });
        }
      })();
    }, GLANCE_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [ctl, doc, sheet, rev, active.r, active.c]);

  const { result, sampled } = state;
  return (
    <aside className={styles.panel} aria-label="At a glance" data-testid="glance-panel">
      <header className={styles.head}>
        <div>
          <div className={styles.title}>At a glance</div>
          <div className={styles.sub}>
            {result ? `${rangeAddress(result.region)}${result.hasHeader ? " · header row" : ""}` : state.loading ? "Reading…" : "No data yet"}
            {sampled ? <span className={styles.badge}>Sampled</span> : null}
          </div>
        </div>
        <button type="button" className={styles.close} aria-label="Close At a glance" onClick={() => useUiStore.getState().setGlance(false)}>
          ×
        </button>
      </header>
      {result ? (
        <div className={styles.body}>
          <div className={styles.tiles}>
            <div className={styles.tile} data-testid="glance-rows">
              <div className={styles.tileLabel}>Rows</div>
              <div className={styles.tileValue}>{result.rows.toLocaleString()}</div>
            </div>
            {result.sums.map((s) => (
              <div key={s.c} className={styles.tile} data-testid={`glance-sum-${s.header}`} title={`Sum of ${s.header}`}>
                <div className={styles.tileLabel}>Sum of {s.header}</div>
                <div className={styles.tileValue}>{s.text}</div>
              </div>
            ))}
          </div>
          {result.category ? (
            <section className={styles.section} data-testid="glance-counts">
              <h4 className={styles.h}>{result.category.header}</h4>
              <ul className={styles.counts}>
                {result.category.counts.slice(0, 8).map((k) => {
                  const max = result.category?.counts[0]?.count ?? 1;
                  return (
                    <li key={k.value} className={styles.countRow} title={`${k.value}: ${k.count}`}>
                      <span className={styles.countLabel}>{k.value}</span>
                      <span className={styles.track}>
                        <span className={styles.fill} style={{ width: `${(k.count / max) * 100}%` }} />
                      </span>
                      <span className={styles.countValue}>{k.count}</span>
                    </li>
                  );
                })}
              </ul>
            </section>
          ) : null}
          {result.chart ? (
            <section className={styles.section} data-testid="glance-chart">
              <h4 className={styles.h}>
                {result.chart.valueHeader} by {result.chart.categoryHeader}
              </h4>
              <BarChart bars={result.chart.bars} />
            </section>
          ) : null}
          {result.sums.length === 0 && !result.category ? <p className={styles.empty}>No numeric or category columns here.</p> : null}
        </div>
      ) : (
        <p className={styles.empty}>{state.loading ? "" : "Type some data, or open a template, to see a summary here."}</p>
      )}
    </aside>
  );
}

/** Horizontal bars, one hue (single series: no legend), value labels in text ink. */
function BarChart({ bars }: { bars: { label: string; value: number; text: string }[] }) {
  const W = 268;
  const labelW = 84;
  const valueW = 70;
  const barH = 14;
  const gap = 10;
  const max = Math.max(...bars.map((b) => Math.abs(b.value)), 1e-12);
  const plotW = W - labelW - valueW;
  const H = bars.length * (barH + gap) - gap;
  return (
    <svg className={styles.chart} viewBox={`0 0 ${W} ${H}`} width="100%" role="img" aria-label="Bar chart">
      {bars.map((b, i) => {
        const y = i * (barH + gap);
        const w = Math.max(2, (Math.abs(b.value) / max) * plotW);
        return (
          <g key={b.label}>
            <title>{`${b.label}: ${b.text}`}</title>
            <rect x={0} y={y - gap / 2} width={W} height={barH + gap} fill="transparent" />
            <text x={0} y={y + barH / 2} dominantBaseline="central" className={styles.axisLabel}>
              {b.label.length > 13 ? `${b.label.slice(0, 12)}…` : b.label}
            </text>
            <rect x={labelW} y={y} width={w} height={barH} rx={4} className={styles.bar} />
            <text x={labelW + w + 6} y={y + barH / 2} dominantBaseline="central" className={styles.valueLabel}>
              {b.text}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
