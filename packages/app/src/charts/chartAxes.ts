/**
 * Date category axes and axis crossing positions (T18), AOO 4.1 semantics.
 *
 * Date axis (chart2 `ScaleAutomatism::calculateExplicitIncrementAndScaleForDateTimeAxis`):
 * the engine sends each category as days since 1970-01-01, floored to the
 * resolution (`ChartDataDto.dateAxis`). The scale runs from the first to
 * the last date, snapped to the resolution; column, bar and stock charts
 * shift the categories into the middle of their slot and add one unit at
 * the end (`ShiftedCategoryPosition`). The major interval is the axis'
 * own (raised to the resolution) or AOO's automatic choice: the day span
 * over the label count that fits, rounded up to days (2, then 7), months
 * or years. Labels sit at the major ticks, formatted with the category
 * cells' number format.
 *
 * Crossing ("Cross other axis at" + "Place labels"): Start and End put the
 * axis at an edge of the other axis; a value (or category number) needs
 * the plot layout, so `axisCrossFixes` lists what `applyAxisCrossFixes`
 * moves after ECharts laid the chart out (an `offset` for labels near the
 * axis, or a drawn axis line when the labels stay outside).
 */
import type { ChartAxisDto, ChartDataDto, ChartDto, DateAxisDto, TimeIntervalDto, TimeUnit } from "@tally/bridge";
import { formatNumberLite } from "../numfmtLite";
import { liteLocaleOf } from "../styles/numberFormat";

export const DAY_MS = 86_400_000;
/** Serial of 1970-01-01 on the 1899-12-30 epoch. */
const UNIX_SERIAL = 25_569;
const UNIT_ORDER: Record<TimeUnit, number> = { days: 0, months: 1, years: 2 };
const UNIT_DAYS: Record<TimeUnit, number> = { days: 1, months: 31, years: 365 };

/** Chart kinds with a date-capable category axis (AOO `isSupportingDateAxis`). */
export function supportsDateAxis(kind: ChartDto["kind"]): boolean {
  return kind === "column" || kind === "bar" || kind === "line" || kind === "area" || kind === "stock";
}

/** The date axis in effect, or null for text categories. */
export function activeDateAxis(chart: ChartDto, data: ChartDataDto): DateAxisDto | null {
  const d = data.dateAxis ?? null;
  if (!d || !supportsDateAxis(chart.kind) || chart.xAxis.axisType === "text") return null;
  return d.days.some((x) => x !== null) ? d : null;
}

function ymd(day: number): [number, number, number] {
  const d = new Date(day * DAY_MS);
  return [d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()];
}

const dayOf = (y: number, m: number, d: number): number => Math.round(Date.UTC(y, m, d) / DAY_MS);
const daysInMonth = (y: number, m: number): number => new Date(Date.UTC(y, m + 1, 0)).getUTCDate();

/** `day` moved by `n` units (AOO DateHelper: months and years keep the day, clamped to the month). */
export function addUnits(day: number, unit: TimeUnit, n: number): number {
  if (unit === "days") return day + n;
  const [y, m, d] = ymd(day);
  const ty = unit === "years" ? y + n : y + Math.floor((m + n) / 12);
  const tm = unit === "years" ? m : (((m + n) % 12) + 12) % 12;
  return dayOf(ty, tm, Math.min(d, daysInMonth(ty, tm)));
}

/** `day` floored to the start of its unit. */
export function floorUnit(day: number, unit: TimeUnit): number {
  if (unit === "days") return Math.floor(day);
  const [y, m] = ymd(day);
  return unit === "months" ? dayOf(y, m, 1) : dayOf(y, 0, 1);
}

export interface DateScale {
  /** First and last day of the axis. */
  min: number;
  max: number;
  /** Major tick days (labels). */
  ticks: number[];
  /** Minor tick days. */
  minorTicks: number[];
  major: TimeIntervalDto;
  minor: TimeIntervalDto;
  resolution: TimeUnit;
  /** Categories in the middle of their slot (column, bar, stock). */
  shifted: boolean;
  /** Day of category `j` on the axis, null for an empty category. */
  pos: (j: number) => number | null;
  /** Day (fractional) of a 1-based category position, interpolated. */
  posAt: (x: number) => number | null;
  /** Label text of a day. */
  label: (day: number) => string;
}

/** Serials (1899-12-30 epoch) of 0001-01-01 and 9999-12-31. */
const SERIAL_RANGE: [number, number] = [new Date(Date.UTC(2000, 0, 1)).setUTCFullYear(1) / DAY_MS + UNIX_SERIAL, new Date(Date.UTC(2000, 11, 31)).setUTCFullYear(9999) / DAY_MS + UNIX_SERIAL];

/**
 * A serial date as `yyyy-mm-dd` for a date input: `""` for none, `null`
 * when it is not a date in years 1-9999 (not finite, or a number set on a
 * value axis) — the editor then shows it as a number.
 */
export function serialToIso(v: number | null): string | null {
  if (v === null) return "";
  if (!Number.isFinite(v) || v < SERIAL_RANGE[0] || v >= SERIAL_RANGE[1] + 1) return null;
  const d = new Date(Math.floor(v - UNIX_SERIAL) * DAY_MS);
  const y = d.getUTCFullYear();
  if (y < 1 || y > 9999) return null;
  return `${String(y).padStart(4, "0")}-${String(d.getUTCMonth() + 1).padStart(2, "0")}-${String(d.getUTCDate()).padStart(2, "0")}`;
}

/** The serial of a date input's `yyyy-mm-dd` (null: empty or invalid). */
export function isoToSerial(v: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v.trim());
  if (!m) return null;
  const d = new Date(Date.UTC(2000, Number(m[2]) - 1, Number(m[3])));
  d.setUTCFullYear(Number(m[1]));
  const ms = d.getTime();
  if (!Number.isFinite(ms) || d.getUTCFullYear() < 1) return null;
  return Math.round(ms / DAY_MS) + UNIX_SERIAL;
}

/** Minimum at or above the maximum (AOO STR_MIN_GREATER_MAX; the engine refuses it too). */
export function axisRangeInvalid(a: Pick<ChartAxisDto, "min" | "max">): boolean {
  return a.min !== null && a.max !== null && a.min >= a.max;
}

/** The label formatter of a date axis: the axis or category format, else ISO dates. */
export function dateLabel(code: string | null): (day: number) => string {
  const fmt = code && code.trim() !== "" ? code : "YYYY-MM-DD";
  const loc = liteLocaleOf(fmt);
  return (day) => formatNumberLite(day + UNIX_SERIAL, fmt, loc) ?? new Date(day * DAY_MS).toISOString().slice(0, 10);
}

/** Labels that fit along the axis (AOO `estimateMaximumAutoMainIncrementCount`: axis length over the widest label). */
function maxLabelCount(chart: ChartDto, label: (d: number) => string, sample: number): number {
  const pxPerMm = 96 / 25.4;
  const horizontal = chart.kind !== "bar";
  const legendSide = chart.legend === "left" || chart.legend === "right" ? 110 : 0;
  const length = horizontal ? chart.anchor.widthMm * pxPerMm - 72 - legendSide : chart.anchor.heightMm * pxPerMm - 80;
  const single = horizontal ? Math.max(24, label(sample).length * 6.5 + 10) : 18;
  return Math.max(2, Math.floor(length / single));
}

/** The date scale of a chart's category axis (AOO's explicit scale and increments). */
export function dateScale(chart: ChartDto, d: DateAxisDto): DateScale | null {
  const present = d.days.filter((x): x is number => x !== null);
  if (present.length === 0) return null;
  const axis = chart.xAxis;
  const res = d.resolution;
  const shifted = chart.kind === "column" || chart.kind === "bar" || chart.kind === "stock";
  const label = dateLabel(axis.numberFormat ?? d.numberFormat);
  let min = axis.min !== null ? Math.floor(axis.min) - UNIX_SERIAL : Math.min(...present);
  let max = axis.max !== null ? Math.floor(axis.max) - UNIX_SERIAL : Math.max(...present);
  if (min > max) [min, max] = [max, min];
  const autoMax = axis.max === null;
  const autoMin = axis.min === null;
  if (res === "days") {
    if (shifted) max += 1;
  } else {
    min = floorUnit(min, res);
    max = floorUnit(max, res);
    if (shifted) max = addUnits(max, res, 1);
    if (addUnits(min, res, 1) > max) {
      if (autoMax || !autoMin) max = addUnits(min, res, 1);
      else min = addUnits(max, res, -1);
    }
  }
  const dayCount = max - min;
  let maxCount = maxLabelCount(chart, label, max);
  if (maxCount > 1) maxCount--;
  // Major interval.
  let major: TimeIntervalDto | null = axis.majorTime ? { ...axis.majorTime } : null;
  let count = 1;
  if (major) {
    if (UNIT_ORDER[major.unit] < UNIT_ORDER[res]) major.unit = res;
    count = Math.floor(dayCount / (major.number * UNIT_DAYS[major.unit]));
    if (count > maxCount) major = null;
  }
  if (!major) {
    const intervalDays = Math.floor(dayCount / maxCount);
    let unit: TimeUnit = intervalDays > 365 || res === "years" ? "years" : intervalDays > 31 || res === "months" ? "months" : "days";
    let n = Math.max(1, Math.ceil(intervalDays / UNIT_DAYS[unit]));
    if (unit === "days") {
      if (n > 2 && n < 7) n = 7;
      else if (n > 7) {
        unit = "months";
        n = Math.max(1, Math.ceil(intervalDays / 31));
      }
    }
    major = { number: n, unit };
    count = Math.floor(dayCount / (n * UNIT_DAYS[unit]));
  }
  // Minor interval.
  let minor: TimeIntervalDto | null = axis.minorTime ? { ...axis.minorTime } : null;
  if (minor) {
    if (UNIT_ORDER[minor.unit] > UNIT_ORDER[major.unit]) minor.unit = major.unit;
    if (dayCount / (minor.number * UNIT_DAYS[minor.unit]) > maxCount * 10) minor = null;
  }
  if (!minor) {
    minor = { number: 1, unit: major.unit };
    if (count > 100) minor.number = major.number;
    else if (major.number >= 2) {
      const m = major.number;
      minor.number = m % 2 === 0 ? m / 2 : m % 3 === 0 ? m / 3 : m % 5 === 0 ? m / 5 : m > 50 ? m : 1;
    } else if (major.unit === "months" && res === "days") minor.unit = "days";
    else if (major.unit === "years" && UNIT_ORDER[res] <= 1) minor.unit = "months";
  }
  const ticksOf = (step: TimeIntervalDto, limit: number): number[] => {
    const out: number[] = [];
    for (let k = 0; out.length < limit; k++) {
      const t = addUnits(min, step.unit, k * step.number);
      if (t > max) break;
      out.push(t);
    }
    return out;
  };
  const ticks = ticksOf(major, 1000);
  const minorTicks = ticksOf(minor, 5000).filter((t) => !ticks.includes(t));
  const pos = (j: number): number | null => {
    const day = d.days[j];
    if (day === null || day === undefined) return null;
    return shifted ? (day + addUnits(day, res, 1)) / 2 : day;
  };
  const posAt = (x: number): number | null => {
    const i = x - 1;
    const lo = pos(Math.floor(i));
    const hi = pos(Math.ceil(i));
    if (lo === null || hi === null) return lo ?? hi;
    return lo + (hi - lo) * (i - Math.floor(i));
  };
  return { min, max, ticks, minorTicks, major, minor, resolution: res, shifted, pos, posAt, label };
}

/** The ECharts `time` axis of a date scale (days → ms). */
export function timeAxisOption(a: ChartAxisDto, s: DateScale, theme: { textMuted: string; gridLine: string }): Record<string, unknown> {
  const ms = (d: number) => d * DAY_MS;
  return {
    type: "time",
    show: a.visible,
    name: a.title ?? undefined,
    nameLocation: "middle",
    nameGap: 28,
    nameTextStyle: { color: theme.textMuted },
    min: ms(s.min),
    max: ms(s.max),
    inverse: a.reverse,
    splitLine: { show: a.majorGrid, lineStyle: { color: theme.gridLine } },
    axisLine: { lineStyle: { color: theme.gridLine } },
    axisTick: { customValues: s.ticks.map(ms) },
    minorTick: { show: false },
    axisLabel: { color: theme.textMuted, customValues: s.ticks.map(ms), formatter: (v: number) => s.label(Math.round(v / DAY_MS)) },
  };
}

// ---------------------------------------------------------------------------
// Crossing
// ---------------------------------------------------------------------------

/** What `applyAxisCrossFixes` does to one ECharts axis after layout. */
export interface AxisCrossFix {
  /** The axis moved or drawn. */
  axis: "xAxis" | "yAxis";
  /** The perpendicular axis the value is read on. */
  other: "xAxis" | "yAxis";
  /** The crossing in the other axis' data coordinates (category index, ms, or value). */
  value: number;
  /** `move`: shift the whole axis (labels near the axis); `line`: draw only the line there (labels stay outside). */
  mode: "move" | "line";
  /** Edge the axis is laid out at before the fix. */
  position: "bottom" | "top" | "left" | "right";
  color: string;
}

type OtherKind = "category" | "time" | "value";

/** Position properties and the post-layout fix of one model axis. */
export function crossingOf(
  a: ChartAxisDto,
  echartsAxis: "xAxis" | "yAxis",
  otherAxis: ChartAxisDto,
  otherKind: OtherKind,
  color: string,
): { props: Record<string, unknown>; fix: AxisCrossFix | null } {
  const horizontal = echartsAxis === "xAxis";
  const other = horizontal ? "yAxis" : "xAxis";
  const startEdge = horizontal ? (otherAxis.reverse ? "top" : "bottom") : otherAxis.reverse ? "right" : "left";
  const endEdge = horizontal ? (otherAxis.reverse ? "bottom" : "top") : otherAxis.reverse ? "left" : "right";
  const c = a.crosses;
  const lp = a.labelPosition;
  const props: Record<string, unknown> = {};
  if (lp === "nearAxisOtherSide") props.axisLabel = { inside: true };
  if (c === null && lp !== "outsideStart" && lp !== "outsideEnd") return { props, fix: null };
  // The crossing in the other axis' coordinates (null: an edge).
  let edge: "start" | "end" | null = c?.type === "start" ? "start" : c?.type === "end" ? "end" : null;
  let value: number | null = null;
  if (c?.type === "value") value = otherKind === "category" ? c.value - 1 : otherKind === "time" ? (c.value - UNIX_SERIAL) * DAY_MS : c.value;
  else if (c === null) {
    // AOO's default: the other axis' value 0 (a category axis: its start).
    if (otherKind === "value") value = 0;
    else edge = "start";
  }
  props.axisLine = { onZero: false };
  if (lp === "outsideStart" || lp === "outsideEnd") {
    const labelEdge = lp === "outsideStart" ? "start" : "end";
    props.position = labelEdge === "start" ? startEdge : endEdge;
    if (edge === labelEdge) return { props, fix: null };
    props.axisLine = { onZero: false, show: false };
    if (a.markPosition === "atAxis") props.axisTick = { show: false };
    const line = edge === null ? value : null;
    if (line === null) {
      // The line sits at the opposite edge: draw it there via a fix at the extent.
      return { props, fix: { axis: echartsAxis, other, value: Number.NaN, mode: "line", position: props.position as AxisCrossFix["position"], color } };
    }
    return { props, fix: { axis: echartsAxis, other, value: line, mode: "line", position: props.position as AxisCrossFix["position"], color } };
  }
  if (edge !== null) {
    props.position = edge === "start" ? startEdge : endEdge;
    return { props, fix: null };
  }
  props.position = startEdge;
  return { props, fix: { axis: echartsAxis, other, value: value ?? 0, mode: "move", position: startEdge, color } };
}

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface ChartLike {
  convertToPixel(finder: Record<string, number>, value: number): number | number[];
  setOption(option: Record<string, unknown>, opts?: unknown): void;
}

function gridRect(inst: ChartLike): Rect | null {
  try {
    const model = (inst as unknown as { getModel(): { getComponent(t: string, i: number): { coordinateSystem?: { getRect(): Rect } } | undefined } }).getModel();
    return model.getComponent("grid", 0)?.coordinateSystem?.getRect() ?? null;
  } catch {
    return null;
  }
}

/** Moves or draws the crossing axes once ECharts laid the chart out (no-op without fixes). */
export function applyAxisCrossFixes(chart: object, fixes: readonly AxisCrossFix[]): void {
  if (fixes.length === 0) return;
  const inst = chart as ChartLike;
  for (let pass = 0; pass < 2; pass++) {
    const rect = gridRect(inst);
    if (!rect) return;
    const xAxis: Record<string, unknown>[] = [];
    const yAxis: Record<string, unknown>[] = [];
    const graphic: Record<string, unknown>[] = [];
    let changed = false;
    for (const f of fixes) {
      const horizontal = f.axis === "xAxis";
      const lo = horizontal ? rect.y : rect.x;
      const hi = horizontal ? rect.y + rect.height : rect.x + rect.width;
      let p: number;
      if (Number.isNaN(f.value)) p = f.position === "bottom" || f.position === "right" ? lo : hi;
      else {
        const raw = inst.convertToPixel({ [`${f.other}Index`]: 0 }, f.value);
        const px = Array.isArray(raw) ? raw[horizontal ? 1 : 0] : raw;
        if (typeof px !== "number" || !Number.isFinite(px)) continue;
        p = Math.max(lo, Math.min(hi, px));
      }
      if (f.mode === "move") {
        const offset = f.position === "bottom" || f.position === "right" ? p - hi : lo - p;
        (horizontal ? xAxis : yAxis).push({ offset: Math.round(offset * 10) / 10 });
        changed = true;
      } else {
        const shape = horizontal ? { x1: rect.x, y1: p, x2: rect.x + rect.width, y2: p } : { x1: p, y1: rect.y, x2: p, y2: rect.y + rect.height };
        graphic.push({ type: "line", id: `tally-cross-${f.axis}`, silent: true, z: 5, shape, style: { stroke: f.color, lineWidth: 1 } });
        changed = true;
      }
    }
    if (!changed) return;
    const opt: Record<string, unknown> = {};
    if (xAxis.length > 0) opt.xAxis = xAxis;
    if (yAxis.length > 0) opt.yAxis = yAxis;
    if (graphic.length > 0) opt.graphic = graphic;
    inst.setOption(opt);
    // A moved axis can change the label space; one more pass settles it.
    if (xAxis.length === 0 && yAxis.length === 0) return;
  }
}
