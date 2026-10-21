/**
 * The Chart Editor side panel (W5-5): type gallery, data (source range,
 * series list with range picking), titles, legend, axes, data labels,
 * statistics (trend line, error bars, mean) and appearance. Every change
 * is one `updateChart` transaction ("Edit chart"); text fields commit on
 * blur / Enter, pickers at once.
 */
import type { ChartAxisDto, ChartDto, ChartKind, ChartSeriesDto, ErrorBarKind, LegendPos, Stacking, StockVariant, TrendKind } from "@tally/bridge";
import { useEffect, useState, type ReactNode } from "react";
import { cx, useController } from "../context";
import { t } from "../i18n";
import { hexToRgba, rgbaToHex } from "../palette";
import { CHART_KINDS, KindThumb } from "./chartKinds";
import { selectedChart, useChartStore } from "./chartStore";
import { DEFAULT_AXIS } from "./toEcharts";
import styles from "./ChartEditorPanel.module.css";

const LEGENDS: { id: LegendPos; label: string }[] = [
  { id: "none", label: "None" },
  { id: "right", label: "Right" },
  { id: "left", label: "Left" },
  { id: "top", label: "Top" },
  { id: "bottom", label: "Bottom" },
];

/** A blank series (values set by the user). */
export function emptySeries(values = ""): ChartSeriesDto {
  return { name: null, values, xValues: null, sizes: null, kind: null, secondaryAxis: false, color: null, marker: null, labels: { value: false, percent: false, category: false, seriesName: false, position: "auto" }, trend: null, errorBars: null, meanLine: false };
}

/** The series name text shown in the editor. */
export function seriesNameText(s: ChartSeriesDto): string {
  return s.name === null ? "" : s.name.type === "text" ? s.name.text : s.name.ref;
}

/** A typed name: a reference (`$Sheet1.$B$1`) or literal text. */
export function parseSeriesName(text: string): ChartSeriesDto["name"] {
  const v = text.trim();
  if (v === "") return null;
  return /^\$?[^.]+\.\$?[A-Za-z]{1,3}\$?\d+$/.test(v) ? { type: "ref", ref: v } : { type: "text", text: v };
}

/** Default error-bar parameters for a kind picked in the editor. */
export function errorBarKindFor(type: string): ErrorBarKind | null {
  switch (type) {
    case "constant":
      return { type: "constant", plus: 1, minus: 1 };
    case "percent":
      return { type: "percent", percent: 5 };
    case "stdDev":
      return { type: "stdDev", k: 1 };
    case "variance":
      return { type: "variance" };
    case "stdError":
      return { type: "stdError" };
    case "errorMargin":
      return { type: "errorMargin", percent: 5 };
    case "range":
      return { type: "range", plus: "", minus: "" };
    default:
      return null;
  }
}

export function ChartEditorPanel() {
  const ctl = useController();
  const chart = useChartStore((s) => selectedChart(s));
  const data = useChartStore((s) => (chart ? s.data[chart.id] : undefined));
  const [draft, setDraft] = useState<ChartDto | null>(chart);
  const [seriesIdx, setSeriesIdx] = useState(0);
  const [axisTab, setAxisTab] = useState<"x" | "y" | "y2">("y");
  useEffect(() => setDraft(chart), [chart]);
  if (!chart || !draft) return null;

  const commit = (next: ChartDto): void => {
    setDraft(next);
    void ctl.w5.charts.updateChart(next);
  };
  const patch = (p: Partial<ChartDto>): void => commit({ ...draft, ...p });
  const stage = (p: Partial<ChartDto>): void => setDraft({ ...draft, ...p });
  const commitDraft = (): void => {
    if (JSON.stringify(draft) !== JSON.stringify(chart)) void ctl.w5.charts.updateChart(draft);
  };
  const series = draft.series;
  const cur = series[Math.min(seriesIdx, Math.max(0, series.length - 1))];
  const setSeries = (i: number, p: Partial<ChartSeriesDto>, now = true): void => {
    const next = series.map((s, j) => (j === i ? { ...s, ...p } : s));
    if (now) patch({ series: next });
    else stage({ series: next });
  };
  const pick = async (apply: (ref: string) => void): Promise<void> => {
    const ref = await ctl.w5.charts.pickRange();
    if (ref) apply(ref);
  };
  const cartesian = draft.kind !== "pie" && draft.kind !== "radar";
  const xy = draft.kind === "scatter" || draft.kind === "bubble";
  const axisOf = (tab: "x" | "y" | "y2"): ChartAxisDto => (tab === "x" ? draft.xAxis : tab === "y" ? draft.yAxis : (draft.y2Axis ?? DEFAULT_AXIS));
  const setAxis = (tab: "x" | "y" | "y2", p: Partial<ChartAxisDto>, now = true): void => {
    const a = { ...axisOf(tab), ...p };
    const next = tab === "x" ? { xAxis: a } : tab === "y" ? { yAxis: a } : { y2Axis: a };
    if (now) patch(next);
    else stage(next);
  };
  const hasY2 = draft.y2Axis !== null || series.some((s) => s.secondaryAxis);

  return (
    <aside className={styles.panel} aria-label={t("chart.editor", "Chart editor")} data-testid="chart-editor">
      <header className={styles.head}>
        <div>
          <div className={styles.title}>{t("chart.editor", "Chart editor")}</div>
          <div className={styles.sub}>{draft.title || `Chart ${draft.id}`}</div>
        </div>
        <button type="button" className={styles.close} aria-label={t("chart.closeEditor", "Close chart editor")} onClick={() => ctl.w5.charts.closeEditor()}>
          ×
        </button>
      </header>
      <div className={styles.body}>
        <Section title={t("chart.type", "Type")} open testId="chart-section-type">
          <div className={styles.kinds} role="radiogroup" aria-label="Chart type">
            {CHART_KINDS.map((k) => (
              <button key={k.kind} type="button" role="radio" aria-checked={draft.kind === k.kind} className={draft.kind === k.kind ? styles.kindOn : styles.kind} data-kind={k.kind} title={k.label} onClick={() => patch(kindPatch(draft, k.kind))}>
                <KindThumb kind={k.kind} />
                <span>{k.label}</span>
              </button>
            ))}
          </div>
          {draft.kind === "column" || draft.kind === "bar" || draft.kind === "line" || draft.kind === "area" ? (
            <Row label="Stacking">
              <Segmented value={draft.stacking} options={[{ id: "none", label: "None" }, { id: "stacked", label: "Stacked" }, { id: "percent", label: "Percent" }] as { id: Stacking; label: string }[]} onChange={(stacking) => patch({ stacking })} />
            </Row>
          ) : null}
          {draft.kind === "line" || draft.kind === "scatter" ? (
            <div className={styles.checks}>
              <Check label="Lines" checked={draft.lines} onChange={(lines) => patch({ lines })} />
              <Check label="Markers" checked={draft.markers} onChange={(markers) => patch({ markers })} />
              <Check label="Smooth" checked={draft.smooth} onChange={(smooth) => patch({ smooth })} />
            </div>
          ) : null}
          {draft.kind === "pie" ? (
            <div className={styles.checks}>
              <Check label="Donut" checked={(draft.hole ?? 0) > 0} onChange={(on) => patch({ hole: on ? 0.5 : null })} />
              <Check label="Exploded" checked={draft.exploded} onChange={(exploded) => patch({ exploded })} />
            </div>
          ) : null}
          {draft.kind === "radar" ? (
            <div className={styles.checks}>
              <Check label="Filled" checked={draft.filled} onChange={(filled) => patch({ filled })} />
              <Check label="Markers" checked={draft.markers} onChange={(markers) => patch({ markers })} />
            </div>
          ) : null}
          {draft.kind === "stock" ? (
            <Row label="Variant">
              <select className={styles.select} value={draft.stock ?? "hlc"} onChange={(e) => patch({ stock: e.target.value as StockVariant })}>
                <option value="hlc">Low, high, close</option>
                <option value="ohlc">Open, low, high, close</option>
                <option value="volumeHlc">Volume + low, high, close</option>
                <option value="volumeOhlc">Volume + open, low, high, close</option>
              </select>
            </Row>
          ) : null}
          <Check label="3D (kept for the file; drawn flat)" checked={draft.threeD} onChange={(threeD) => patch({ threeD })} />
        </Section>

        <Section title={t("chart.data", "Data")} open testId="chart-section-data">
          <div className={styles.rowWide}>
            Data range
            <RefInput value={draft.source?.range ?? ""} testId="chart-source" onCommit={(range) => patch({ source: range === "" ? null : { range, seriesInRows: draft.source?.seriesInRows ?? false, firstRowLabel: draft.source?.firstRowLabel ?? true, firstColLabel: draft.source?.firstColLabel ?? true }, series: [] })} onPick={() => pick((ref) => patch({ source: { range: ref, seriesInRows: false, firstRowLabel: true, firstColLabel: true }, series: [] }))} />
          </div>
          {draft.source ? (
            <div className={styles.checks}>
              <Check label="Series in rows" checked={draft.source.seriesInRows} onChange={(seriesInRows) => patch({ source: { ...draft.source!, seriesInRows }, series: [] })} />
              <Check label="First row as label" checked={draft.source.firstRowLabel} onChange={(firstRowLabel) => patch({ source: { ...draft.source!, firstRowLabel }, series: [] })} />
              <Check label="First column as label" checked={draft.source.firstColLabel} onChange={(firstColLabel) => patch({ source: { ...draft.source!, firstColLabel }, series: [] })} />
            </div>
          ) : null}
          <div className={styles.rowWide}>
            Categories
            <RefInput value={draft.categories ?? ""} testId="chart-categories" onCommit={(v) => patch({ categories: v === "" ? null : v })} onPick={() => pick((ref) => patch({ categories: ref }))} />
          </div>
          {series.map((s, i) => (
            <div key={i} className={styles.series} data-testid={`chart-series-${i}`}>
              <div className={styles.seriesHead}>
                <input className={styles.input} placeholder={`Series ${i + 1}`} aria-label="Series name" defaultValue={seriesNameText(s)} key={`${draft.id}:${i}:${seriesNameText(s)}`} onBlur={(e) => setSeries(i, { name: parseSeriesName(e.target.value) })} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
                <input type="color" className={styles.swatch} aria-label="Series colour" value={s.color !== null ? rgbaToHex(s.color) : rgbaToHex(data?.series[i]?.color ?? 0x004586ff)} onChange={(e) => setSeries(i, { color: hexToRgba(e.target.value) })} />
                <button type="button" className={styles.remove} aria-label="Remove series" onClick={() => patch({ series: series.filter((_, j) => j !== i) })}>
                  ×
                </button>
              </div>
              <RefInput value={s.values} placeholder="Values ($Sheet1.$B$2:$B$9)" testId={`chart-series-${i}-values`} onCommit={(values) => setSeries(i, { values })} onPick={() => pick((ref) => setSeries(i, { values: ref }))} />
              {xy ? <RefInput value={s.xValues ?? ""} placeholder="X values" onCommit={(v) => setSeries(i, { xValues: v || null })} onPick={() => pick((ref) => setSeries(i, { xValues: ref }))} /> : null}
              {draft.kind === "bubble" ? <RefInput value={s.sizes ?? ""} placeholder="Bubble sizes" onCommit={(v) => setSeries(i, { sizes: v || null })} onPick={() => pick((ref) => setSeries(i, { sizes: ref }))} /> : null}
              {cartesian && !xy ? (
                <div className={styles.checks}>
                  <select className={styles.select} aria-label="Series type" value={s.kind ?? ""} onChange={(e) => setSeries(i, { kind: (e.target.value || null) as ChartKind | null })}>
                    <option value="">Same as chart</option>
                    <option value="column">Column</option>
                    <option value="line">Line</option>
                    <option value="area">Area</option>
                  </select>
                  <Check label="Secondary axis" checked={s.secondaryAxis} onChange={(secondaryAxis) => setSeries(i, { secondaryAxis })} />
                </div>
              ) : null}
            </div>
          ))}
          <button type="button" className={styles.add} data-testid="chart-add-series" onClick={() => patch({ series: [...series, emptySeries()], source: null })}>
            + Add series
          </button>
          {data && data.problems.length > 0 ? <div className={styles.problems}>{data.problems.join(" · ")}</div> : null}
          {data?.truncated ? <div className={styles.hint}>Some points were left out (the chart limits are 255 series and 32,000 points).</div> : null}
        </Section>

        <Section title={t("chart.titles", "Titles")} open testId="chart-section-titles">
          <Row label="Title">
            <input className={styles.input} data-testid="chart-title" value={draft.title ?? ""} onChange={(e) => stage({ title: e.target.value || null })} onBlur={commitDraft} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
          </Row>
          <Row label="Subtitle">
            <input className={styles.input} value={draft.subtitle ?? ""} onChange={(e) => stage({ subtitle: e.target.value || null })} onBlur={commitDraft} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
          </Row>
          <Row label="Legend">
            <select className={styles.select} data-testid="chart-legend" value={draft.legend} onChange={(e) => patch({ legend: e.target.value as LegendPos })}>
              {LEGENDS.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.label}
                </option>
              ))}
            </select>
          </Row>
        </Section>

        {cartesian ? (
          <Section title={t("chart.axes", "Axes")} testId="chart-section-axes">
            <div className={styles.tabs} role="tablist">
              {(["x", "y", ...(hasY2 ? ["y2" as const] : [])] as ("x" | "y" | "y2")[]).map((tab) => (
                <button key={tab} type="button" role="tab" aria-selected={axisTab === tab} className={axisTab === tab ? styles.tabOn : styles.tab} onClick={() => setAxisTab(tab)}>
                  {tab === "x" ? "X axis" : tab === "y" ? "Y axis" : "Secondary Y"}
                </button>
              ))}
            </div>
            <AxisEditor axis={axisOf(axisTab)} value={!xy && axisTab === "x" ? "category" : "value"} onStage={(p) => setAxis(axisTab, p, false)} onCommit={(p) => setAxis(axisTab, p)} commitDraft={commitDraft} />
          </Section>
        ) : null}

        <Section title={t("chart.labels", "Data labels")} testId="chart-section-labels">
          {series.length > 1 ? <SeriesPicker series={series} value={seriesIdx} onChange={setSeriesIdx} /> : null}
          {cur ? (
            <>
              <div className={styles.checks}>
                <Check label="Value" checked={cur.labels.value} onChange={(value) => setSeries(seriesIdx, { labels: { ...cur.labels, value } })} />
                {draft.kind === "pie" ? <Check label="Percent" checked={cur.labels.percent} onChange={(percent) => setSeries(seriesIdx, { labels: { ...cur.labels, percent } })} /> : null}
                <Check label="Category" checked={cur.labels.category} onChange={(category) => setSeries(seriesIdx, { labels: { ...cur.labels, category } })} />
                <Check label="Series name" checked={cur.labels.seriesName} onChange={(seriesName) => setSeries(seriesIdx, { labels: { ...cur.labels, seriesName } })} />
              </div>
              <Row label="Placement">
                <select className={styles.select} value={cur.labels.position} onChange={(e) => setSeries(seriesIdx, { labels: { ...cur.labels, position: e.target.value as ChartSeriesDto["labels"]["position"] } })}>
                  <option value="auto">Automatic</option>
                  <option value="outside">Outside</option>
                  <option value="inside">Inside</option>
                  <option value="center">Centre</option>
                  <option value="above">Above</option>
                  <option value="below">Below</option>
                </select>
              </Row>
            </>
          ) : (
            <div className={styles.hint}>Add a series first.</div>
          )}
        </Section>

        {cartesian ? (
          <Section title={t("chart.statistics", "Statistics")} testId="chart-section-statistics">
            {series.length > 1 ? <SeriesPicker series={series} value={seriesIdx} onChange={setSeriesIdx} /> : null}
            {cur ? (
              <>
                <Row label="Trend line">
                  <select className={styles.select} data-testid="chart-trend" value={cur.trend?.kind ?? ""} onChange={(e) => setSeries(seriesIdx, { trend: e.target.value === "" ? null : { kind: e.target.value as TrendKind, showEquation: cur.trend?.showEquation ?? false, showR2: cur.trend?.showR2 ?? false, name: cur.trend?.name ?? null } })}>
                    <option value="">None</option>
                    <option value="linear">Linear</option>
                    <option value="logarithmic">Logarithmic</option>
                    <option value="exponential">Exponential</option>
                    <option value="power">Power</option>
                  </select>
                </Row>
                {cur.trend ? (
                  <div className={styles.checks}>
                    <Check label="Show equation" checked={cur.trend.showEquation} onChange={(showEquation) => setSeries(seriesIdx, { trend: { ...cur.trend!, showEquation } })} />
                    <Check label="Show R²" checked={cur.trend.showR2} onChange={(showR2) => setSeries(seriesIdx, { trend: { ...cur.trend!, showR2 } })} />
                  </div>
                ) : null}
                <Row label="Error bars">
                  <select className={styles.select} value={cur.errorBars?.kind.type ?? ""} onChange={(e) => setSeries(seriesIdx, { errorBars: e.target.value === "" ? null : { kind: errorBarKindFor(e.target.value)!, direction: cur.errorBars?.direction ?? "both" } })}>
                    <option value="">None</option>
                    <option value="constant">Constant value</option>
                    <option value="percent">Percentage</option>
                    <option value="stdDev">Standard deviation</option>
                    <option value="stdError">Standard error</option>
                    <option value="variance">Variance</option>
                    <option value="errorMargin">Error margin</option>
                    <option value="range">Cell range</option>
                  </select>
                </Row>
                {cur.errorBars ? <ErrorBarFields series={cur} onChange={(errorBars) => setSeries(seriesIdx, { errorBars })} onPick={pick} /> : null}
                <Check label="Mean value line" checked={cur.meanLine} onChange={(meanLine) => setSeries(seriesIdx, { meanLine })} />
              </>
            ) : (
              <div className={styles.hint}>Add a series first.</div>
            )}
          </Section>
        ) : null}

        <Section title={t("chart.appearance", "Appearance")} testId="chart-section-appearance">
          <Row label="Background">
            <div className={styles.refRow}>
              <input type="color" className={styles.swatch} aria-label="Chart background" value={draft.background !== null ? rgbaToHex(draft.background) : "#ffffff"} onChange={(e) => patch({ background: hexToRgba(e.target.value) })} />
              <button type="button" className={styles.pick} onClick={() => patch({ background: null })}>
                None
              </button>
            </div>
          </Row>
          <Row label="Plot area">
            <div className={styles.refRow}>
              <input type="color" className={styles.swatch} aria-label="Plot background" value={draft.plotBackground !== null ? rgbaToHex(draft.plotBackground) : "#ffffff"} onChange={(e) => patch({ plotBackground: hexToRgba(e.target.value) })} />
              <button type="button" className={styles.pick} onClick={() => patch({ plotBackground: null })}>
                None
              </button>
            </div>
          </Row>
          <Check label="Include hidden cells" checked={draft.includeHidden} onChange={(includeHidden) => patch({ includeHidden })} />
        </Section>
        <div className={styles.actions}>
          <button type="button" className={styles.button} onClick={() => void ctl.w5.charts.copyAsImage()}>
            Copy as image
          </button>
          <button type="button" className={styles.danger} data-testid="chart-delete" onClick={() => void ctl.w5.charts.deleteSelected()}>
            Delete chart
          </button>
        </div>
      </div>
    </aside>
  );
}

/** Sensible defaults when switching kind (markers for XY, HLC for stock, lines for line). */
export function kindPatch(chart: ChartDto, kind: ChartKind): Partial<ChartDto> {
  const p: Partial<ChartDto> = { kind };
  if (kind === "stock" && !chart.stock) p.stock = "hlc";
  if (kind === "scatter" || kind === "bubble") p.markers = true;
  if (kind === "line" && !chart.lines && !chart.markers) p.lines = true;
  if (kind === "pie" || kind === "radar" || kind === "scatter" || kind === "bubble" || kind === "stock") p.stacking = "none";
  return p;
}

function Section({ title, open, children, testId }: { title: string; open?: boolean; children: ReactNode; testId?: string }) {
  return (
    <details className={styles.section} open={open} data-testid={testId}>
      <summary>{title}</summary>
      <div className={styles.content}>{children}</div>
    </details>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className={styles.row}>
      <span>{label}</span>
      {children}
    </label>
  );
}

function Check({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className={styles.check}>
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      {label}
    </label>
  );
}

function Segmented<T extends string>({ value, options, onChange }: { value: T; options: { id: T; label: string }[]; onChange: (v: T) => void }) {
  return (
    <span className={styles.segmented} role="radiogroup">
      {options.map((o) => (
        <button key={o.id} type="button" role="radio" aria-checked={value === o.id} className={value === o.id ? styles.segOn : styles.seg} onClick={() => onChange(o.id)}>
          {o.label}
        </button>
      ))}
    </span>
  );
}

/** A reference text field with a "pick" button (range-pick mode on the grid). */
function RefInput({ value, placeholder, onCommit, onPick, testId }: { value: string; placeholder?: string; onCommit: (v: string) => void; onPick: () => void; testId?: string }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  return (
    <div className={styles.refRow}>
      <input
        className={cx(styles.input, styles.mono)}
        placeholder={placeholder}
        value={text}
        data-testid={testId}
        aria-label={placeholder ?? "Range"}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => text.trim() !== value && onCommit(text.trim())}
        onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
      />
      <button type="button" className={styles.pick} title="Pick the range on the sheet" aria-label="Pick range" onClick={onPick}>
        ⤓
      </button>
    </div>
  );
}

function SeriesPicker({ series, value, onChange }: { series: ChartSeriesDto[]; value: number; onChange: (i: number) => void }) {
  return (
    <Row label="Series">
      <select className={styles.select} value={Math.min(value, series.length - 1)} onChange={(e) => onChange(Number(e.target.value))}>
        {series.map((s, i) => (
          <option key={i} value={i}>
            {seriesNameText(s) || `Series ${i + 1}`}
          </option>
        ))}
      </select>
    </Row>
  );
}

function AxisEditor({ axis, value, onStage, onCommit, commitDraft }: { axis: ChartAxisDto; value: "category" | "value"; onStage: (p: Partial<ChartAxisDto>) => void; onCommit: (p: Partial<ChartAxisDto>) => void; commitDraft: () => void }) {
  const num = (k: "min" | "max" | "major") => (
    <input className={styles.input} type="number" step="any" value={axis[k] ?? ""} onChange={(e) => onStage({ [k]: e.target.value === "" ? null : Number(e.target.value) })} onBlur={commitDraft} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
  );
  return (
    <>
      <Check label="Show axis" checked={axis.visible} onChange={(visible) => onCommit({ visible })} />
      <Row label="Title">
        <input className={styles.input} value={axis.title ?? ""} onChange={(e) => onStage({ title: e.target.value || null })} onBlur={commitDraft} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
      </Row>
      {value === "value" ? (
        <>
          <Row label="Minimum">{num("min")}</Row>
          <Row label="Maximum">{num("max")}</Row>
          <Row label="Major interval">{num("major")}</Row>
          <Row label="Minor divisions">
            <input className={styles.input} type="number" min={0} max={100} value={axis.minorCount ?? ""} onChange={(e) => onStage({ minorCount: e.target.value === "" ? null : Math.max(0, Math.trunc(Number(e.target.value))) })} onBlur={commitDraft} />
          </Row>
          <Row label="Number format">
            <input className={cx(styles.input, styles.mono)} placeholder="General" value={axis.numberFormat ?? ""} onChange={(e) => onStage({ numberFormat: e.target.value || null })} onBlur={commitDraft} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
          </Row>
        </>
      ) : null}
      <div className={styles.checks}>
        {value === "value" ? <Check label="Logarithmic" checked={axis.log} onChange={(log) => onCommit({ log })} /> : null}
        <Check label="Reverse" checked={axis.reverse} onChange={(reverse) => onCommit({ reverse })} />
        <Check label="Major grid" checked={axis.majorGrid} onChange={(majorGrid) => onCommit({ majorGrid })} />
        <Check label="Minor grid" checked={axis.minorGrid} onChange={(minorGrid) => onCommit({ minorGrid })} />
      </div>
    </>
  );
}

function ErrorBarFields({ series, onChange, onPick }: { series: ChartSeriesDto; onChange: (e: ChartSeriesDto["errorBars"]) => void; onPick: (apply: (ref: string) => void) => Promise<void> }) {
  const eb = series.errorBars!;
  const k = eb.kind;
  const set = (kind: ErrorBarKind) => onChange({ ...eb, kind });
  const numField = (label: string, v: number, apply: (n: number) => ErrorBarKind) => (
    <Row label={label}>
      <input className={styles.input} type="number" step="any" defaultValue={v} key={`${label}:${v}`} onBlur={(e) => set(apply(Number(e.target.value) || 0))} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
    </Row>
  );
  return (
    <>
      {k.type === "constant" ? (
        <>
          {numField("Positive", k.plus, (n) => ({ ...k, plus: n }))}
          {numField("Negative", k.minus, (n) => ({ ...k, minus: n }))}
        </>
      ) : null}
      {k.type === "percent" || k.type === "errorMargin" ? numField("Percent", k.percent, (n) => ({ ...k, percent: n })) : null}
      {k.type === "stdDev" ? numField("Multiplier", k.k, (n) => ({ ...k, k: n })) : null}
      {k.type === "range" ? (
        <>
          <RefInput value={k.plus} placeholder="Positive (range)" onCommit={(plus) => set({ ...k, plus })} onPick={() => void onPick((ref) => set({ ...k, plus: ref }))} />
          <RefInput value={k.minus} placeholder="Negative (range)" onCommit={(minus) => set({ ...k, minus })} onPick={() => void onPick((ref) => set({ ...k, minus: ref }))} />
        </>
      ) : null}
      <Row label="Direction">
        <Segmented value={eb.direction} options={[{ id: "both", label: "Both" }, { id: "plus", label: "Plus" }, { id: "minus", label: "Minus" }] as { id: ChartSeriesDto["errorBars"] extends infer E ? (E extends { direction: infer D } ? D : never) : never; label: string }[]} onChange={(direction) => onChange({ ...eb, direction })} />
      </Row>
    </>
  );
}
