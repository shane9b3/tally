/**
 * The Chart Editor side panel (W5-5): type gallery, data (source range,
 * series list with range picking), titles, legend, axes, data labels,
 * statistics (trend line, error bars, mean) and appearance. Every change
 * is one `updateChart` transaction ("Edit chart"); text fields commit on
 * blur / Enter, pickers at once.
 */
import type { AxisCross, AxisLabelPos, AxisMarkPos, AxisType, ChartAxisDto, ChartDto, ChartKind, TimeIntervalDto, TimeUnit, ChartSeriesDto, ErrorBarKind, LegendPos, Stacking, StockVariant, TrendKind } from "@tally/bridge";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { cx, useController } from "../context";
import { t } from "../i18n";
import { hexToRgba, rgbaToHex } from "../palette";
import { CHART_KINDS, KindThumb } from "./chartKinds";
import { useW56Store } from "../data/store";
import { selectedChart, useChartStore } from "./chartStore";
import { DEFAULT_AXIS } from "./toEcharts";
import { activeDateAxis, axisRangeInvalid, isoToSerial, serialToIso, supportsDateAxis } from "./chartAxes";
import styles from "./ChartEditorPanel.module.css";
import { arrowNav } from "../components/dialogA11y";

const legends = (): { id: LegendPos; label: string }[] => [
  { id: "none", label: t("cp.noneLegend", "None") },
  { id: "right", label: t("cp.right", "Right") },
  { id: "left", label: t("cp.left", "Left") },
  { id: "top", label: t("cp.top", "Top") },
  { id: "bottom", label: t("cp.bottom", "Bottom") },
];

/** Chart-kind labels (the table lives in chartKinds.tsx; translated here at render). */
const KIND_KEYS: Record<ChartKind, string> = { column: "chart.column", bar: "chart.bar", line: "chart.line", area: "chart.area", pie: "chart.pie", scatter: "cp.kindXY", bubble: "chart.bubble", radar: "chart.radar", stock: "chart.stock" };
const kindLabel = (k: { kind: ChartKind; label: string }): string => t(KIND_KEYS[k.kind] ?? "", k.label);

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
  useW56Store((s) => s.i18nRev);
  const chart = useChartStore((s) => selectedChart(s));
  const data = useChartStore((s) => (chart ? s.data[chart.id] : undefined));
  const [draft, setDraft] = useState<ChartDto | null>(chart);
  const [seriesIdx, setSeriesIdx] = useState(0);
  const [axisTab, setAxisTab] = useState<"x" | "y" | "y2">("y");
  // The latest draft, for edits that land after an await (range picking): a
  // closure over `draft` would re-apply a stale chart (and undo e.g. a title
  // committed on the way to the pick button).
  const latest = useRef<ChartDto | null>(draft);
  latest.current = draft;
  useEffect(() => setDraft(chart), [chart]);
  if (!chart || !draft) return null;

  const now = (): ChartDto => latest.current ?? draft;
  const commit = (next: ChartDto): void => {
    latest.current = next;
    setDraft(next);
    void ctl.w5.charts.updateChart(next);
  };
  const patch = (p: Partial<ChartDto>): void => commit({ ...now(), ...p });
  const stage = (p: Partial<ChartDto>): void => {
    const next = { ...now(), ...p };
    latest.current = next;
    setDraft(next);
  };
  const commitDraft = (): void => {
    const d = now();
    if (JSON.stringify(d) !== JSON.stringify(chart)) void ctl.w5.charts.updateChart(d);
  };
  const series = draft.series;
  const cur = series[Math.min(seriesIdx, Math.max(0, series.length - 1))];
  const setSeries = (i: number, p: Partial<ChartSeriesDto>, commitNow = true): void => {
    const next = now().series.map((s, j) => (j === i ? { ...s, ...p } : s));
    if (commitNow) patch({ series: next });
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
    <aside className={styles.panel} aria-label={t("chart.editor", "Chart editor")} data-testid="chart-editor" onKeyDown={(e) => void arrowNav(e)}>
      <header className={styles.head}>
        <div>
          <div className={styles.title}>{t("chart.editor", "Chart editor")}</div>
          <div className={styles.sub}>{draft.title || t("cp.chartN", "Chart {id}", { id: draft.id })}</div>
        </div>
        <button type="button" className={styles.close} aria-label={t("chart.closeEditor", "Close chart editor")} onClick={() => ctl.w5.charts.closeEditor()}>
          ×
        </button>
      </header>
      <div className={styles.body}>
        <Section title={t("cp.sectionType", "Type")} open testId="chart-section-type">
          <div className={styles.kinds} role="radiogroup" aria-label={t("chart.type", "Chart type")} data-select-on-focus="false">
            {CHART_KINDS.map((k) => (
              <button key={k.kind} type="button" role="radio" aria-checked={draft.kind === k.kind} className={draft.kind === k.kind ? styles.kindOn : styles.kind} data-kind={k.kind} title={kindLabel(k)} onClick={() => patch(kindPatch(draft, k.kind))}>
                <KindThumb kind={k.kind} />
                <span>{kindLabel(k)}</span>
              </button>
            ))}
          </div>
          {draft.kind === "column" || draft.kind === "bar" || draft.kind === "line" || draft.kind === "area" ? (
            <Row label={t("cp.stacking", "Stacking")}>
              <Segmented value={draft.stacking} options={[{ id: "none", label: t("cp.noneStacking", "None") }, { id: "stacked", label: t("chart.stacked", "Stacked") }, { id: "percent", label: t("cp.percent", "Percent") }] as { id: Stacking; label: string }[]} onChange={(stacking) => patch({ stacking })} />
            </Row>
          ) : null}
          {draft.kind === "line" || draft.kind === "scatter" ? (
            <div className={styles.checks}>
              <Check label={t("cp.lines", "Lines")} checked={draft.lines} onChange={(lines) => patch({ lines })} />
              <Check label={t("cp.markers", "Markers")} checked={draft.markers} onChange={(markers) => patch({ markers })} />
              <Check label={t("cp.smooth", "Smooth")} checked={draft.smooth} onChange={(smooth) => patch({ smooth })} />
            </div>
          ) : null}
          {draft.kind === "pie" ? (
            <div className={styles.checks}>
              <Check label={t("cp.donut", "Donut")} checked={(draft.hole ?? 0) > 0} onChange={(on) => patch({ hole: on ? 0.5 : null })} />
              <Check label={t("cp.exploded", "Exploded")} checked={draft.exploded} onChange={(exploded) => patch({ exploded })} />
            </div>
          ) : null}
          {draft.kind === "radar" ? (
            <div className={styles.checks}>
              <Check label={t("cp.filled", "Filled")} checked={draft.filled} onChange={(filled) => patch({ filled })} />
              <Check label={t("cp.markers", "Markers")} checked={draft.markers} onChange={(markers) => patch({ markers })} />
            </div>
          ) : null}
          {draft.kind === "stock" ? (
            <Row label={t("cp.variant", "Variant")}>
              <select className={styles.select} value={draft.stock ?? "hlc"} onChange={(e) => patch({ stock: e.target.value as StockVariant })}>
                <option value="hlc">{t("cp.stockHlc", "Low, high, close")}</option>
                <option value="ohlc">{t("cp.stockOhlc", "Open, low, high, close")}</option>
                <option value="volumeHlc">{t("cp.stockVolumeHlc", "Volume + low, high, close")}</option>
                <option value="volumeOhlc">{t("cp.stockVolumeOhlc", "Volume + open, low, high, close")}</option>
              </select>
            </Row>
          ) : null}
          <Check label={t("cp.threeD", "3D (kept for the file; drawn flat)")} checked={draft.threeD} onChange={(threeD) => patch({ threeD })} />
        </Section>

        <Section title={t("chart.data", "Data")} open testId="chart-section-data">
          <div className={styles.rowWide}>
            {t("chart.dataRange", "Data range")}
            <RefInput value={draft.source?.range ?? ""} testId="chart-source" onCommit={(range) => patch({ source: range === "" ? null : { range, seriesInRows: draft.source?.seriesInRows ?? false, firstRowLabel: draft.source?.firstRowLabel ?? true, firstColLabel: draft.source?.firstColLabel ?? true }, series: [] })} onPick={() => pick((ref) => patch({ source: { range: ref, seriesInRows: false, firstRowLabel: true, firstColLabel: true }, series: [] }))} />
          </div>
          {draft.source ? (
            <div className={styles.checks}>
              <Check label={t("cp.seriesInRows", "Series in rows")} checked={draft.source.seriesInRows} onChange={(seriesInRows) => patch({ source: { ...draft.source!, seriesInRows }, series: [] })} />
              <Check label={t("chart.firstRowLabel", "First row as label")} checked={draft.source.firstRowLabel} onChange={(firstRowLabel) => patch({ source: { ...draft.source!, firstRowLabel }, series: [] })} />
              <Check label={t("chart.firstColumnLabel", "First column as label")} checked={draft.source.firstColLabel} onChange={(firstColLabel) => patch({ source: { ...draft.source!, firstColLabel }, series: [] })} />
            </div>
          ) : null}
          <div className={styles.rowWide}>
            {t("cp.categories", "Categories")}
            <RefInput value={draft.categories ?? ""} testId="chart-categories" onCommit={(v) => patch({ categories: v === "" ? null : v })} onPick={() => pick((ref) => patch({ categories: ref }))} />
          </div>
          {series.map((s, i) => (
            <div key={i} className={styles.series} data-testid={`chart-series-${i}`}>
              <div className={styles.seriesHead}>
                <input className={styles.input} placeholder={t("cp.seriesN", "Series {n}", { n: i + 1 })} aria-label={t("cp.seriesName", "Series name")} defaultValue={seriesNameText(s)} key={`${draft.id}:${i}:${seriesNameText(s)}`} onBlur={(e) => setSeries(i, { name: parseSeriesName(e.target.value) })} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
                <input type="color" className={styles.swatch} aria-label={t("cp.seriesColour", "Series colour")} value={s.color !== null ? rgbaToHex(s.color) : rgbaToHex(data?.series[i]?.color ?? 0x004586ff)} onChange={(e) => setSeries(i, { color: hexToRgba(e.target.value) })} />
                <button type="button" className={styles.remove} aria-label={t("cp.removeSeries", "Remove series")} onClick={() => patch({ series: series.filter((_, j) => j !== i) })}>
                  ×
                </button>
              </div>
              <RefInput value={s.values} placeholder={t("cp.valuesPlaceholder", "Values ($Sheet1.$B$2:$B$9)")} testId={`chart-series-${i}-values`} onCommit={(values) => setSeries(i, { values })} onPick={() => pick((ref) => setSeries(i, { values: ref }))} />
              {xy ? <RefInput value={s.xValues ?? ""} placeholder={t("cp.xValues", "X values")} onCommit={(v) => setSeries(i, { xValues: v || null })} onPick={() => pick((ref) => setSeries(i, { xValues: ref }))} /> : null}
              {draft.kind === "bubble" ? <RefInput value={s.sizes ?? ""} placeholder={t("cp.bubbleSizes", "Bubble sizes")} onCommit={(v) => setSeries(i, { sizes: v || null })} onPick={() => pick((ref) => setSeries(i, { sizes: ref }))} /> : null}
              {cartesian && !xy ? (
                <div className={styles.checks}>
                  <select className={styles.select} aria-label={t("cp.seriesType", "Series type")} value={s.kind ?? ""} onChange={(e) => setSeries(i, { kind: (e.target.value || null) as ChartKind | null })}>
                    <option value="">{t("cp.sameAsChart", "Same as chart")}</option>
                    <option value="column">{t("chart.column", "Column")}</option>
                    <option value="line">{t("chart.line", "Line")}</option>
                    <option value="area">{t("chart.area", "Area")}</option>
                  </select>
                  <Check label={t("cp.secondaryAxis", "Secondary axis")} checked={s.secondaryAxis} onChange={(secondaryAxis) => setSeries(i, { secondaryAxis })} />
                </div>
              ) : null}
            </div>
          ))}
          <button type="button" className={styles.add} data-testid="chart-add-series" onClick={() => patch({ series: [...series, emptySeries()], source: null })}>
            {t("cp.addSeries", "+ Add series")}
          </button>
          {data && data.problems.length > 0 ? <div className={styles.problems}>{data.problems.join(" · ")}</div> : null}
          {data?.truncated ? <div className={styles.hint}>{t("cp.truncated", "Some points were left out (the chart limits are 255 series and 32,000 points).")}</div> : null}
        </Section>

        <Section title={t("chart.titles", "Titles")} open testId="chart-section-titles">
          <Row label={t("chart.chartTitle", "Title")}>
            <input className={styles.input} data-testid="chart-title" value={draft.title ?? ""} onChange={(e) => stage({ title: e.target.value || null })} onBlur={commitDraft} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
          </Row>
          <Row label={t("chart.subtitle", "Subtitle")}>
            <input className={styles.input} value={draft.subtitle ?? ""} onChange={(e) => stage({ subtitle: e.target.value || null })} onBlur={commitDraft} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
          </Row>
          <Row label={t("chart.legend", "Legend")}>
            <select className={styles.select} data-testid="chart-legend" value={draft.legend} onChange={(e) => patch({ legend: e.target.value as LegendPos })}>
              {legends().map((l) => (
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
                  {tab === "x" ? t("chart.xAxis", "X axis") : tab === "y" ? t("chart.yAxis", "Y axis") : t("cp.secondaryY", "Secondary Y")}
                </button>
              ))}
            </div>
            <AxisEditor
              axis={axisOf(axisTab)}
              value={!xy && axisTab === "x" ? "category" : "value"}
              onStage={(p) => setAxis(axisTab, p, false)}
              onCommit={(p) => setAxis(axisTab, p)}
              commitDraft={commitDraft}
              dateCapable={!xy && axisTab === "x" && supportsDateAxis(draft.kind)}
              dateActive={!xy && axisTab === "x" && data !== undefined && activeDateAxis(draft, data) !== null}
              crossing={axisTab === "y2" ? null : axisTab === "x" ? "value" : xy ? "value" : data !== undefined && activeDateAxis(draft, data) !== null ? "date" : "category"}
              categories={data?.categories ?? []}
            />
          </Section>
        ) : null}

        <Section title={t("chart.labels", "Data labels")} testId="chart-section-labels">
          {series.length > 1 ? <SeriesPicker series={series} value={seriesIdx} onChange={setSeriesIdx} /> : null}
          {cur ? (
            <>
              <div className={styles.checks}>
                <Check label={t("dialog.value", "Value")} checked={cur.labels.value} onChange={(value) => setSeries(seriesIdx, { labels: { ...cur.labels, value } })} />
                {draft.kind === "pie" ? <Check label={t("cp.percent", "Percent")} checked={cur.labels.percent} onChange={(percent) => setSeries(seriesIdx, { labels: { ...cur.labels, percent } })} /> : null}
                <Check label={t("cp.category", "Category")} checked={cur.labels.category} onChange={(category) => setSeries(seriesIdx, { labels: { ...cur.labels, category } })} />
                <Check label={t("cp.seriesName", "Series name")} checked={cur.labels.seriesName} onChange={(seriesName) => setSeries(seriesIdx, { labels: { ...cur.labels, seriesName } })} />
              </div>
              <Row label={t("cp.placement", "Placement")}>
                <select className={styles.select} value={cur.labels.position} onChange={(e) => setSeries(seriesIdx, { labels: { ...cur.labels, position: e.target.value as ChartSeriesDto["labels"]["position"] } })}>
                  <option value="auto">{t("cp.automatic", "Automatic")}</option>
                  <option value="outside">{t("cp.outside", "Outside")}</option>
                  <option value="inside">{t("cp.inside", "Inside")}</option>
                  <option value="center">{t("cp.centre", "Centre")}</option>
                  <option value="above">{t("cp.above", "Above")}</option>
                  <option value="below">{t("cp.below", "Below")}</option>
                </select>
              </Row>
            </>
          ) : (
            <div className={styles.hint}>{t("cp.addSeriesFirst", "Add a series first.")}</div>
          )}
        </Section>

        {cartesian ? (
          <Section title={t("chart.statistics", "Statistics")} testId="chart-section-statistics">
            {series.length > 1 ? <SeriesPicker series={series} value={seriesIdx} onChange={setSeriesIdx} /> : null}
            {cur ? (
              <>
                <Row label={t("chart.trendLine", "Trend line")}>
                  <select className={styles.select} data-testid="chart-trend" value={cur.trend?.kind ?? ""} onChange={(e) => setSeries(seriesIdx, { trend: e.target.value === "" ? null : { kind: e.target.value as TrendKind, showEquation: cur.trend?.showEquation ?? false, showR2: cur.trend?.showR2 ?? false, name: cur.trend?.name ?? null } })}>
                    <option value="">{t("cp.noneTrend", "None")}</option>
                    <option value="linear">{t("cp.linear", "Linear")}</option>
                    <option value="logarithmic">{t("cp.logarithmic", "Logarithmic")}</option>
                    <option value="exponential">{t("cp.exponential", "Exponential")}</option>
                    <option value="power">{t("cp.power", "Power")}</option>
                  </select>
                </Row>
                {cur.trend ? (
                  <div className={styles.checks}>
                    <Check label={t("cp.showEquation", "Show equation")} checked={cur.trend.showEquation} onChange={(showEquation) => setSeries(seriesIdx, { trend: { ...cur.trend!, showEquation } })} />
                    <Check label={t("cp.showR2", "Show R²")} checked={cur.trend.showR2} onChange={(showR2) => setSeries(seriesIdx, { trend: { ...cur.trend!, showR2 } })} />
                  </div>
                ) : null}
                <Row label={t("chart.errorBars", "Error bars")}>
                  <select className={styles.select} value={cur.errorBars?.kind.type ?? ""} onChange={(e) => setSeries(seriesIdx, { errorBars: e.target.value === "" ? null : { kind: errorBarKindFor(e.target.value)!, direction: cur.errorBars?.direction ?? "both" } })}>
                    <option value="">{t("cp.noneErrorBars", "None")}</option>
                    <option value="constant">{t("cp.constantValue", "Constant value")}</option>
                    <option value="percent">{t("cp.percentage", "Percentage")}</option>
                    <option value="stdDev">{t("cp.stdDev", "Standard deviation")}</option>
                    <option value="stdError">{t("cp.stdError", "Standard error")}</option>
                    <option value="variance">{t("cp.variance", "Variance")}</option>
                    <option value="errorMargin">{t("cp.errorMargin", "Error margin")}</option>
                    <option value="range">{t("validation.cellRange", "Cell range")}</option>
                  </select>
                </Row>
                {cur.errorBars ? <ErrorBarFields series={cur} onChange={(errorBars) => setSeries(seriesIdx, { errorBars })} onPick={pick} /> : null}
                <Check label={t("chart.meanLine", "Mean value line")} checked={cur.meanLine} onChange={(meanLine) => setSeries(seriesIdx, { meanLine })} />
              </>
            ) : (
              <div className={styles.hint}>{t("cp.addSeriesFirst", "Add a series first.")}</div>
            )}
          </Section>
        ) : null}

        <Section title={t("chart.appearance", "Appearance")} testId="chart-section-appearance">
          <Row label={t("cells.background", "Background")}>
            <div className={styles.refRow}>
              <input type="color" className={styles.swatch} aria-label={t("cp.chartBackground", "Chart background")} value={draft.background !== null ? rgbaToHex(draft.background) : "#ffffff"} onChange={(e) => patch({ background: hexToRgba(e.target.value) })} />
              <button type="button" className={styles.pick} onClick={() => patch({ background: null })}>
                {t("cp.noneBackground", "None")}
              </button>
            </div>
          </Row>
          <Row label={t("cp.plotArea", "Plot area")}>
            <div className={styles.refRow}>
              <input type="color" className={styles.swatch} aria-label={t("cp.plotBackground", "Plot background")} value={draft.plotBackground !== null ? rgbaToHex(draft.plotBackground) : "#ffffff"} onChange={(e) => patch({ plotBackground: hexToRgba(e.target.value) })} />
              <button type="button" className={styles.pick} onClick={() => patch({ plotBackground: null })}>
                {t("cp.nonePlot", "None")}
              </button>
            </div>
          </Row>
          <Check label={t("cp.includeHidden", "Include hidden cells")} checked={draft.includeHidden} onChange={(includeHidden) => patch({ includeHidden })} />
        </Section>
        <div className={styles.actions}>
          <button type="button" className={styles.button} onClick={() => void ctl.w5.charts.copyAsImage()}>
            {t("cp.copyAsImage", "Copy as image")}
          </button>
          <button type="button" className={styles.danger} data-testid="chart-delete" onClick={() => void ctl.w5.charts.deleteSelected()}>
            {t("cmd.chart.delete", "Delete chart")}
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
    <span className={styles.segmented} role="radiogroup" data-select-on-focus="false">
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
        aria-label={placeholder ?? t("dialog.range", "Range")}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => text.trim() !== value && onCommit(text.trim())}
        onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
      />
      <button type="button" className={styles.pick} title={t("cp.pickTitle", "Pick the range on the sheet")} aria-label={t("cp.pickRange", "Pick range")} onClick={onPick}>
        ⤓
      </button>
    </div>
  );
}

function SeriesPicker({ series, value, onChange }: { series: ChartSeriesDto[]; value: number; onChange: (i: number) => void }) {
  return (
    <Row label={t("cp.series", "Series")}>
      <select className={styles.select} value={Math.min(value, series.length - 1)} onChange={(e) => onChange(Number(e.target.value))}>
        {series.map((s, i) => (
          <option key={i} value={i}>
            {seriesNameText(s) || t("cp.seriesN", "Series {n}", { n: i + 1 })}
          </option>
        ))}
      </select>
    </Row>
  );
}

const timeUnits = (): { id: TimeUnit; label: string }[] => [
  { id: "days", label: t("cp.days", "Days") },
  { id: "months", label: t("cp.months", "Months") },
  { id: "years", label: t("cp.years", "Years") },
];


/** A date axis interval: Automatic, or a count and a unit (AOO "Major interval" / "Minor interval"). */
function IntervalRow({ label, value, onCommit, testId }: { label: string; value: TimeIntervalDto | null; onCommit: (v: TimeIntervalDto | null) => void; testId: string }) {
  const cur = value ?? { number: 1, unit: "days" as TimeUnit };
  return (
    <Row label={label}>
      <span className={styles.refRow}>
        <select className={styles.select} data-testid={`${testId}-mode`} value={value ? "set" : "auto"} onChange={(e) => onCommit(e.target.value === "auto" ? null : cur)}>
          <option value="auto">{t("cp.automatic", "Automatic")}</option>
          <option value="set">{t("cp.fixed", "Fixed")}</option>
        </select>
        {value ? (
          <>
            <input className={styles.input} type="number" min={1} max={10000} data-testid={`${testId}-number`} defaultValue={value.number} key={`${value.number}`} onBlur={(e) => onCommit({ ...cur, number: Math.min(10000, Math.max(1, Math.trunc(Number(e.target.value) || 1))) })} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
            <select className={styles.select} data-testid={`${testId}-unit`} value={value.unit} onChange={(e) => onCommit({ ...cur, unit: e.target.value as TimeUnit })}>
              {timeUnits().map((u) => (
                <option key={u.id} value={u.id}>
                  {u.label}
                </option>
              ))}
            </select>
          </>
        ) : null}
      </span>
    </Row>
  );
}

/** "Positioning": where the axis crosses the other one, and where its labels and marks go (AOO Axis ▸ Positioning). */
function PositionEditor({ axis, crossing, categories, onCommit }: { axis: ChartAxisDto; crossing: "value" | "category" | "date"; categories: readonly string[]; onCommit: (p: Partial<ChartAxisDto>) => void }) {
  const c = axis.crosses;
  // AOO's default (no setting) is the other axis' value 0; on a category axis it shows as its start.
  const mode = c === null ? (crossing === "category" ? "start" : "value") : c.type;
  const value = c?.type === "value" ? c.value : crossing === "category" ? 1 : 0;
  const setMode = (m: string): void => {
    const next: AxisCross | null = m === "start" ? { type: "start" } : m === "end" ? { type: "end" } : crossing === "category" ? { type: "value", value: 1 } : null;
    onCommit({ crosses: next });
  };
  const setValue = (v: number): void => onCommit({ crosses: crossing !== "category" && v === 0 ? null : { type: "value", value: v } });
  return (
    <>
      <Row label={t("cp.crossAt", "Cross other axis at")}>
        <select className={styles.select} data-testid="chart-axis-cross" value={mode} onChange={(e) => setMode(e.target.value)}>
          <option value="start">{t("cp.crossStart", "Start")}</option>
          <option value="end">{t("cp.crossEnd", "End")}</option>
          <option value="value">{crossing === "category" ? t("cp.crossCategory", "Category") : t("cp.crossValue", "Value")}</option>
        </select>
      </Row>
      {mode === "value" ? (
        <Row label={crossing === "category" ? t("cp.crossCategory", "Category") : t("cp.crossValue", "Value")}>
          {crossing === "category" ? (
            <select className={styles.select} data-testid="chart-axis-cross-category" value={Math.round(value)} onChange={(e) => setValue(Number(e.target.value))}>
              {categories.map((name, i) => (
                <option key={i} value={i + 1}>
                  {name || String(i + 1)}
                </option>
              ))}
            </select>
          ) : crossing === "date" && serialToIso(c?.type === "value" ? c.value : null) !== null ? (
            <input className={styles.input} type="date" data-testid="chart-axis-cross-date" defaultValue={serialToIso(c?.type === "value" ? c.value : null) ?? ""} key={`d${value}`} onBlur={(e) => setValue(isoToSerial(e.target.value) ?? 0)} />
          ) : (
            <input className={styles.input} type="number" step="any" data-testid="chart-axis-cross-value" defaultValue={value} key={`v${value}`} onBlur={(e) => setValue(Number(e.target.value) || 0)} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
          )}
        </Row>
      ) : null}
      <Row label={t("cp.placeLabels", "Place labels")}>
        <select className={styles.select} data-testid="chart-axis-labels" value={axis.labelPosition} onChange={(e) => onCommit({ labelPosition: e.target.value as AxisLabelPos })}>
          <option value="nearAxis">{t("cp.nearAxis", "Near axis")}</option>
          <option value="nearAxisOtherSide">{t("cp.nearAxisOther", "Near axis (other side)")}</option>
          <option value="outsideStart">{t("cp.outsideStart", "Outside start")}</option>
          <option value="outsideEnd">{t("cp.outsideEnd", "Outside end")}</option>
        </select>
      </Row>
      <Row label={t("cp.placeMarks", "Place marks")}>
        <select className={styles.select} data-testid="chart-axis-marks" value={axis.markPosition} onChange={(e) => onCommit({ markPosition: e.target.value as AxisMarkPos })}>
          <option value="atLabels">{t("cp.atLabels", "At labels")}</option>
          <option value="atAxis">{t("cp.atAxis", "At axis")}</option>
          <option value="atLabelsAndAxis">{t("cp.atAxisAndLabels", "At axis and labels")}</option>
        </select>
      </Row>
    </>
  );
}

function AxisEditor({
  axis,
  value,
  onStage,
  onCommit,
  commitDraft,
  dateCapable = false,
  dateActive = false,
  crossing = null,
  categories = [],
}: {
  axis: ChartAxisDto;
  value: "category" | "value";
  onStage: (p: Partial<ChartAxisDto>) => void;
  onCommit: (p: Partial<ChartAxisDto>) => void;
  commitDraft: () => void;
  /** The axis can be a date axis (category axis of a column/bar/line/area/stock chart). */
  dateCapable?: boolean;
  /** A date axis is drawn now (explicit, or automatic over date categories). */
  dateActive?: boolean;
  /** What the other axis is, for "Cross other axis at" (null: no positioning, e.g. the secondary Y axis). */
  crossing?: "value" | "category" | "date" | null;
  categories?: readonly string[];
}) {
  // AOO's Scale tab refuses a minimum at or above the maximum with a warning (STR_MIN_GREATER_MAX): keep it staged and say so.
  const badRange = axisRangeInvalid(axis);
  const commitRange = (): void => {
    if (!badRange) commitDraft();
  };
  const num = (k: "min" | "max" | "major") => (
    <input
      className={styles.input}
      type="number"
      step="any"
      data-testid={`chart-axis-${k}`}
      aria-invalid={k !== "major" && badRange ? true : undefined}
      value={axis[k] ?? ""}
      onChange={(e) => onStage({ [k]: e.target.value === "" || !Number.isFinite(Number(e.target.value)) ? null : Number(e.target.value) })}
      onBlur={k === "major" ? commitDraft : commitRange}
      onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
    />
  );
  /** A date field; a limit outside years 1-9999 (set as a number on another chart type) stays a number field. */
  const date = (k: "min" | "max") => {
    const iso = serialToIso(axis[k]);
    if (iso === null) {
      // The field turns into a date field once the value is empty or a date: commit then (its blur never comes).
      return (
        <input
          className={styles.input}
          type="number"
          step="any"
          data-testid={`chart-axis-${k}`}
          aria-invalid={badRange ? true : undefined}
          value={axis[k] ?? ""}
          onChange={(e) => {
            const v = e.target.value === "" || !Number.isFinite(Number(e.target.value)) ? null : Number(e.target.value);
            const p = { [k]: v } as Partial<ChartAxisDto>;
            if (serialToIso(v) !== null && !axisRangeInvalid({ ...axis, ...p })) onCommit(p);
            else onStage(p);
          }}
          onBlur={commitRange}
          onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
        />
      );
    }
    return (
      <input
        className={styles.input}
        type="date"
        data-testid={`chart-axis-${k}-date`}
        aria-invalid={badRange ? true : undefined}
        defaultValue={iso}
        key={`${k}${axis[k]}`}
        onBlur={(e) => {
          const p = { [k]: isoToSerial(e.target.value) } as Partial<ChartAxisDto>;
          if (axisRangeInvalid({ ...axis, ...p })) onStage(p);
          else onCommit(p);
        }}
      />
    );
  };
  const rangeError = badRange ? (
    <div className={styles.danger} role="alert" data-testid="chart-axis-range-error">
      {t("cp.minBelowMax", "The minimum must be lower than the maximum. Check your input.")}
    </div>
  ) : null;
  const showDate = dateCapable && (axis.axisType === "date" || (axis.axisType === "auto" && dateActive));
  return (
    <>
      <Check label={t("cp.showAxis", "Show axis")} checked={axis.visible} onChange={(visible) => onCommit({ visible })} />
      <Row label={t("chart.chartTitle", "Title")}>
        <input className={styles.input} value={axis.title ?? ""} onChange={(e) => onStage({ title: e.target.value || null })} onBlur={commitDraft} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
      </Row>
      {dateCapable ? (
        <Row label={t("cp.axisType", "Type")}>
          <select className={styles.select} data-testid="chart-axis-type" value={axis.axisType} onChange={(e) => onCommit({ axisType: e.target.value as AxisType })}>
            <option value="auto">{t("cp.automatic", "Automatic")}</option>
            <option value="text">{t("cp.axisTypeText", "Text")}</option>
            <option value="date">{t("cp.axisTypeDate", "Date")}</option>
          </select>
        </Row>
      ) : null}
      {showDate ? (
        <>
          <Row label={t("cp.minimum", "Minimum")}>{date("min")}</Row>
          <Row label={t("cp.maximum", "Maximum")}>{date("max")}</Row>
          {rangeError}
          <Row label={t("cp.resolution", "Resolution")}>
            <select className={styles.select} data-testid="chart-axis-resolution" value={axis.timeResolution ?? "auto"} onChange={(e) => onCommit({ timeResolution: e.target.value === "auto" ? null : (e.target.value as TimeUnit) })}>
              <option value="auto">{t("cp.automatic", "Automatic")}</option>
              {timeUnits().map((u) => (
                <option key={u.id} value={u.id}>
                  {u.label}
                </option>
              ))}
            </select>
          </Row>
          <IntervalRow label={t("cp.majorInterval", "Major interval")} value={axis.majorTime} onCommit={(majorTime) => onCommit({ majorTime })} testId="chart-axis-major-time" />
          <IntervalRow label={t("cp.minorInterval", "Minor interval")} value={axis.minorTime} onCommit={(minorTime) => onCommit({ minorTime })} testId="chart-axis-minor-time" />
        </>
      ) : null}
      {value === "value" ? (
        <>
          <Row label={t("cp.minimum", "Minimum")}>{num("min")}</Row>
          <Row label={t("cp.maximum", "Maximum")}>{num("max")}</Row>
          {rangeError}
          <Row label={t("cp.majorInterval", "Major interval")}>{num("major")}</Row>
          <Row label={t("cp.minorDivisions", "Minor divisions")}>
            <input className={styles.input} type="number" min={0} max={100} value={axis.minorCount ?? ""} onChange={(e) => onStage({ minorCount: e.target.value === "" ? null : Math.max(0, Math.trunc(Number(e.target.value))) })} onBlur={commitDraft} />
          </Row>
          <Row label={t("menu.format.numberFormat", "Number format")}>
            <input className={cx(styles.input, styles.mono)} placeholder={t("fmt.general", "General")} value={axis.numberFormat ?? ""} onChange={(e) => onStage({ numberFormat: e.target.value || null })} onBlur={commitDraft} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
          </Row>
        </>
      ) : null}
      <div className={styles.checks}>
        {value === "value" ? <Check label={t("cp.logarithmic", "Logarithmic")} checked={axis.log} onChange={(log) => onCommit({ log })} /> : null}
        <Check label={t("cp.reverse", "Reverse")} checked={axis.reverse} onChange={(reverse) => onCommit({ reverse })} />
        <Check label={t("cp.majorGrid", "Major grid")} checked={axis.majorGrid} onChange={(majorGrid) => onCommit({ majorGrid })} />
        <Check label={t("cp.minorGrid", "Minor grid")} checked={axis.minorGrid} onChange={(minorGrid) => onCommit({ minorGrid })} />
      </div>
      {crossing ? <PositionEditor axis={axis} crossing={crossing} categories={categories} onCommit={onCommit} /> : null}
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
          {numField(t("cp.positive", "Positive"), k.plus, (n) => ({ ...k, plus: n }))}
          {numField(t("cp.negative", "Negative"), k.minus, (n) => ({ ...k, minus: n }))}
        </>
      ) : null}
      {k.type === "percent" || k.type === "errorMargin" ? numField(t("cp.percent", "Percent"), k.percent, (n) => ({ ...k, percent: n })) : null}
      {k.type === "stdDev" ? numField(t("cp.multiplier", "Multiplier"), k.k, (n) => ({ ...k, k: n })) : null}
      {k.type === "range" ? (
        <>
          <RefInput value={k.plus} placeholder={t("cp.positiveRange", "Positive (range)")} onCommit={(plus) => set({ ...k, plus })} onPick={() => void onPick((ref) => set({ ...k, plus: ref }))} />
          <RefInput value={k.minus} placeholder={t("cp.negativeRange", "Negative (range)")} onCommit={(minus) => set({ ...k, minus })} onPick={() => void onPick((ref) => set({ ...k, minus: ref }))} />
        </>
      ) : null}
      <Row label={t("cp.direction", "Direction")}>
        <Segmented value={eb.direction} options={[{ id: "both", label: t("cp.both", "Both") }, { id: "plus", label: t("cp.plus", "Plus") }, { id: "minus", label: t("cp.minus", "Minus") }] as { id: ChartSeriesDto["errorBars"] extends infer E ? (E extends { direction: infer D } ? D : never) : never; label: string }[]} onChange={(direction) => onChange({ ...eb, direction })} />
      </Row>
    </>
  );
}
