/**
 * Conditional-format evaluation for the browser mock (`WebMockClient`), so
 * the demo and e2e flows show chips, colour scales, data bars and icon
 * sets. A small subset of the engine's rules (crates/tally-engine
 * condfmt.rs): cell value (numbers and quoted text), text tests,
 * duplicate/unique, top/bottom, above/below average, blank/error tests,
 * colour scales, data bars and icon sets. Formula and date rules never
 * match here. Style rules combine by priority (the first rule setting a
 * property wins), the first colour scale fills unless a style rule did,
 * the first data bar and icon set apply, `stopIfTrue` ends the evaluation.
 */
import type { CellView, CfRuleDto, CfStyleDto, CfThreshold, ConditionalFormatDto, DataBarView, IconView, RangeDto } from "@tally/bridge";

/** What conditional formatting does to one cell. */
export interface CfDecoration {
  /** Merged differential style (only set fields apply), or `null`. */
  patch: CfStyleDto | null;
  bar: DataBarView | null;
  icon: IconView | null;
  /** A data bar or icon set with `showValue: false`. */
  hideText: boolean;
}

const key = (r: number, c: number): string => `${r},${c}`;

function inRange(rg: RangeDto, r: number, c: number): boolean {
  return r >= rg.r0 && r <= rg.r1 && c >= rg.c0 && c <= rg.c1;
}

function numOf(v: CellView | undefined): number | null {
  if (!v || v.kind !== "number") return null;
  if (v.num !== null) return v.num;
  const n = Number(v.text.replace(/[^0-9.eE+-]/g, ""));
  return Number.isFinite(n) ? n : null;
}

/** A rule operand: a number, quoted text, or `null` (references and formulas are not evaluated). */
export function operand(f: string): number | string | null {
  const t = f.trim().replace(/^=/, "");
  const q = /^"((?:[^"]|"")*)"$/.exec(t);
  if (q) return (q[1] ?? "").replace(/""/g, '"');
  const n = Number(t.replace(/%$/, ""));
  if (t !== "" && Number.isFinite(n)) return t.endsWith("%") ? n / 100 : n;
  if (/^[A-Za-z_][A-Za-z ]*$/.test(t) && !/^\$?[A-Za-z]{1,3}\$?\d+$/.test(t)) return t; // bare text such as Paid
  return null;
}

function compare(v: CellView | undefined, x: number | string): number | null {
  const n = numOf(v);
  if (typeof x === "number") return n === null ? (v === undefined || v.text === "" ? 0 - x : null) : n - x;
  const t = (v?.text ?? "").toLowerCase();
  const y = x.toLowerCase();
  return t < y ? -1 : t > y ? 1 : 0;
}

/** Linear interpolation of two `0xRRGGBBAA` colours. */
export function mixColor(a: number, b: number, t: number): number {
  const ch = (x: number, s: number) => (x >>> s) & 255;
  const m = (s: number) => Math.round(ch(a, s) + (ch(b, s) - ch(a, s)) * t);
  return ((m(24) << 24) | (m(16) << 16) | (m(8) << 8) | 0xff) >>> 0;
}

function percentile(sorted: readonly number[], p: number): number {
  if (sorted.length === 0) return 0;
  const i = (Math.max(0, Math.min(100, p)) / 100) * (sorted.length - 1);
  const lo = Math.floor(i);
  const hi = Math.ceil(i);
  return (sorted[lo] ?? 0) + ((sorted[hi] ?? 0) - (sorted[lo] ?? 0)) * (i - lo);
}

interface Stats {
  nums: number[];
  sorted: number[];
  min: number;
  max: number;
  mean: number;
  counts: Map<string, number>;
}

function stats(cells: readonly CellView[]): Stats {
  const nums = cells.map(numOf).filter((n): n is number => n !== null);
  const sorted = [...nums].sort((a, b) => a - b);
  const counts = new Map<string, number>();
  for (const v of cells) {
    if (v.text === "") continue;
    const k = v.text.toLowerCase();
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  return {
    nums,
    sorted,
    min: sorted[0] ?? 0,
    max: sorted[sorted.length - 1] ?? 0,
    mean: nums.length ? nums.reduce((a, b) => a + b, 0) / nums.length : 0,
    counts,
  };
}

function thresholdValue(t: CfThreshold, s: Stats): number {
  const v = Number(t.value);
  switch (t.kind) {
    case "min":
      return s.min;
    case "max":
      return s.max;
    case "autoMin":
      return Math.min(0, s.min);
    case "autoMax":
      return Math.max(0, s.max);
    case "percent":
      return s.min + ((s.max - s.min) * (Number.isFinite(v) ? v : 0)) / 100;
    case "percentile":
      return percentile(s.sorted, Number.isFinite(v) ? v : 50);
    case "number":
      return Number.isFinite(v) ? v : 0;
    case "formula":
      return Number.isFinite(v) ? v : 0;
  }
}

/** Whether a style rule matches a cell. */
function matches(rule: CfRuleDto, v: CellView | undefined, s: Stats): boolean {
  const k = rule.kind;
  const text = v?.text ?? "";
  switch (k.type) {
    case "cellValue": {
      const xs = k.formulas.map(operand);
      if (xs.some((x) => x === null)) return false;
      const a = compare(v, xs[0]!);
      if (a === null) return false;
      switch (k.op) {
        case "eq":
          return a === 0;
        case "ne":
          return a !== 0;
        case "gt":
          return a > 0;
        case "ge":
          return a >= 0;
        case "lt":
          return a < 0;
        case "le":
          return a <= 0;
        case "between":
        case "notBetween": {
          const b = compare(v, xs[1]!);
          if (b === null) return false;
          const inside = a >= 0 && b <= 0;
          return k.op === "between" ? inside : !inside;
        }
      }
      return false;
    }
    case "text": {
      const t = text.toLowerCase();
      const x = k.text.toLowerCase();
      if (k.op === "contains") return t.includes(x);
      if (k.op === "notContains") return !t.includes(x);
      if (k.op === "beginsWith") return t.startsWith(x);
      return t.endsWith(x);
    }
    case "duplicate":
      return text !== "" && (s.counts.get(text.toLowerCase()) ?? 0) > 1;
    case "unique":
      return text !== "" && (s.counts.get(text.toLowerCase()) ?? 0) === 1;
    case "top": {
      const n = numOf(v);
      if (n === null || s.sorted.length === 0) return false;
      const count = Math.max(1, k.percent ? Math.floor((s.sorted.length * k.rank) / 100) : k.rank);
      const desc = [...s.sorted].reverse();
      const asc = s.sorted;
      return k.bottom ? n <= (asc[Math.min(count, asc.length) - 1] ?? -Infinity) : n >= (desc[Math.min(count, desc.length) - 1] ?? Infinity);
    }
    case "average": {
      const n = numOf(v);
      if (n === null) return false;
      return k.above ? (k.equal ? n >= s.mean : n > s.mean) : k.equal ? n <= s.mean : n < s.mean;
    }
    case "blank":
      return text.trim() === "";
    case "notBlank":
      return text.trim() !== "";
    case "error":
      return v?.kind === "error";
    case "notError":
      return v?.kind !== "error";
    default:
      return false;
  }
}

/** Earlier (higher-priority) fields win. */
function mergePatch(into: CfStyleDto | null, p: CfStyleDto): CfStyleDto {
  if (!into) return { ...p };
  const out = { ...into };
  for (const k of Object.keys(p) as (keyof CfStyleDto)[]) {
    if (out[k] === null && p[k] !== null) (out as Record<string, unknown>)[k] = p[k];
  }
  return out;
}

/**
 * Decorations for every cell of `cf`'s ranges that some rule changes, keyed
 * `r,c`. `cells` are the non-empty cells of those ranges.
 */
export function evaluateFormat(cf: ConditionalFormatDto, cells: readonly CellView[], out: Map<string, CfDecoration> = new Map()): Map<string, CfDecoration> {
  const inside = cells.filter((v) => cf.ranges.some((rg) => inRange(rg, v.r, v.c)));
  const s = stats(inside);
  for (const v of inside) {
    let d = out.get(key(v.r, v.c));
    const fresh = !d;
    if (!d) d = { patch: null, bar: null, icon: null, hideText: false };
    for (const rule of cf.rules) {
      const k = rule.kind;
      const n = numOf(v);
      if (k.type === "colorScale") {
        if (n === null || k.stops.length < 2 || d.patch?.fill != null) continue;
        const pts = k.stops.map((st) => ({ at: thresholdValue(st.threshold, s), color: st.color }));
        let fill = pts[0]!.color;
        if (n >= pts[pts.length - 1]!.at) fill = pts[pts.length - 1]!.color;
        else {
          for (let i = 0; i + 1 < pts.length; i++) {
            const a = pts[i]!;
            const b = pts[i + 1]!;
            if (n >= a.at && n <= b.at) {
              fill = mixColor(a.color, b.color, b.at === a.at ? 0 : (n - a.at) / (b.at - a.at));
              break;
            }
          }
        }
        d.patch = mergePatch(d.patch, { bold: null, italic: null, underline: null, strike: null, color: null, fill, borders: null, numberFormat: null, display: null });
        continue;
      }
      if (k.type === "dataBar") {
        if (n === null || d.bar) continue;
        const lo = thresholdValue(k.min, s);
        const hi = thresholdValue(k.max, s);
        const negative = n < 0;
        let bar: DataBarView;
        if (lo < 0 && hi > 0) {
          const axis = -lo / (hi - lo);
          bar = { fraction: Math.max(0, Math.min(1, negative ? n / lo : n / hi)), color: negative ? (k.negativeColor ?? k.color) : k.color, negative, axis, gradient: k.gradient };
        } else {
          bar = { fraction: hi === lo ? 1 : Math.max(0, Math.min(1, (n - lo) / (hi - lo))), color: k.color, negative: false, axis: null, gradient: k.gradient };
        }
        d.bar = bar;
        if (!k.showValue) d.hideText = true;
        continue;
      }
      if (k.type === "iconSet") {
        if (n === null || d.icon) continue;
        const count = k.set.startsWith("three") ? 3 : k.set.startsWith("four") ? 4 : 5;
        let index = 0;
        k.thresholds.slice(1, count).forEach((t, i) => {
          if (n >= thresholdValue(t, s)) index = i + 1;
        });
        d.icon = { set: k.set, index: k.reverse ? count - 1 - index : index };
        if (!k.showValue) d.hideText = true;
        continue;
      }
      if (matches(rule, v, s)) {
        d.patch = mergePatch(d.patch, rule.style);
        if (rule.stopIfTrue) break;
      }
    }
    if (fresh && (d.patch || d.bar || d.icon)) out.set(key(v.r, v.c), d);
  }
  return out;
}

/** Applies a differential style to a resolved style (the mock's synthesized cf styles). */
export function applyCfPatch<T extends { bold: boolean; italic: boolean; underline: boolean; strike: boolean; color: number | null; fill: number | null; numberFormat: string; display: unknown }>(base: T, p: CfStyleDto): T {
  return {
    ...base,
    bold: p.bold ?? base.bold,
    italic: p.italic ?? base.italic,
    underline: p.underline ?? base.underline,
    strike: p.strike ?? base.strike,
    color: p.color ?? base.color,
    fill: p.fill ?? base.fill,
    numberFormat: p.numberFormat ?? base.numberFormat,
    display: p.display ?? base.display,
  };
}
