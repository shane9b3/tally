/**
 * The W5-6 side model of the browser mock (`WebMockClient`): pivot tables
 * (computed from the mock's cells and written as plain values), outline
 * groups (collapsed rows are hidden), standard / advanced filters,
 * subtotals, consolidate, text to columns, remove duplicates, insert /
 * delete cells, delete contents, fill series, scenarios, named cell styles,
 * AutoFormat, sheet hiding and tab colours, database ranges, view options,
 * calculation settings and custom sort lists, plus the queries that read
 * them (`pivots`, `pivot_field_items`, `pivot_at`, `autocomplete`,
 * `cell_styles`, `scenarios`, `database_ranges`, `navigator`,
 * `calc_settings`). Plain JSON (snapshotted for undo/redo by the mock's
 * `DocExt`); pure functions apart from the cell lookups. Nothing here ships
 * in the desktop bundle.
 */
import type {
  AggFn,
  AutocompleteDto,
  CalcSettingsDto,
  CellStyleDto,
  CellView,
  DatabaseRangeDto,
  DeleteContentsFlags,
  NavItem,
  Op,
  PivotDto,
  PivotFieldItemsDto,
  PivotHit,
  RangeDto,
  ScenarioDto,
  SheetInfo,
  StdCondition,
  StdFilterOptions,
  StyleDto,
  StylePatch,
  ViewOptionsDto,
} from "@tally/bridge";
import { completionFor, matchingItems } from "../autocorrect/autoinput";
import { absoluteRef } from "../dataOps";
import { applyOutlineOp, autoOutlineGroups, emptyOutlineModel, hiddenByOutline, type OutlineModel } from "./mockOutline";
import { splitLine, type SeparatorState } from "../data/textToColumns";
import { SUBTOTAL_FNS } from "../data/builders";
import { AUTOFORMAT_PRESETS } from "../styles/autoformat";
import { aggLabel } from "../pivot/pivotModel";

export interface StoredScenario extends ScenarioDto {
  /** The stored inputs of each range (row-major). */
  values: string[][][];
}

export interface W56Ext {
  pivots: PivotDto[];
  nextPivot: number;
  scenarios: Record<number, StoredScenario[]>;
  nextScenario: number;
  /** User-defined cell styles (the built-in ones are generated). */
  styles: CellStyleDto[];
  dbRanges: DatabaseRangeDto[];
  outline: Record<number, OutlineModel>;
  view: Record<number, ViewOptionsDto>;
  calc: CalcSettingsDto;
  lists: string[][];
  hiddenSheets: number[];
  tabColors: Record<number, string>;
  /** Standard / advanced filter per sheet (rows are hidden through `filteredRows`). */
  stdFilters: Record<number, { range: RangeDto; conditions: StdCondition[]; options: StdFilterOptions } | null>;
  /** Every W5-6 op applied (e2e hook). */
  log: string[];
}

export function emptyW56(): W56Ext {
  return {
    pivots: [],
    nextPivot: 1,
    scenarios: {},
    nextScenario: 1,
    styles: [],
    dbRanges: [],
    outline: {},
    view: {},
    calc: { iterations: null, dateEpoch: "d1899", caseSensitive: false, precisionAsShown: false, wholeCellMatch: true, criteriaMode: "wildcards", textToNumber: "convertUnambiguous" },
    lists: [],
    hiddenSheets: [],
    tabColors: {},
    stdFilters: {},
    log: [],
  };
}

/** What the mock gives the op emulation. */
export interface W56Ctx {
  cells(sheet: number, range: RangeDto): Promise<CellView[]>;
  inputs(sheet: number, range: RangeDto): Promise<string[][]>;
  used(sheet: number): Promise<RangeDto | null>;
  sheetName(sheet: number): string;
  sheets(): SheetInfo[];
  notes(sheet: number): { r: number; c: number }[];
  /** Hides/shows rows of a sheet in the mock's layout (outline, filters). */
  setHiddenRows(sheet: number, rows: number[], hidden: boolean): void;
  setHiddenCols(sheet: number, cols: number[], hidden: boolean): void;
}

export type W56Outcome =
  | { kind: "pass" }
  | { kind: "handled"; ops: Op[]; touched: Map<number, RangeDto[]>; layout: Set<number>; pivotsChanged: boolean }
  | { kind: "error"; code: string; message: string };

const inside = (rg: RangeDto, r: number, c: number): boolean => r >= rg.r0 && r <= rg.r1 && c >= rg.c0 && c <= rg.c1;

function numberOf(text: string): number | null {
  const t = text.replace(/[$,\s]/g, "");
  if (t === "" || !/^-?\d*\.?\d+(e-?\d+)?%?$/i.test(t)) return null;
  const n = Number(t.replace(/%$/, ""));
  return Number.isFinite(n) ? (t.endsWith("%") ? n / 100 : n) : null;
}

function cellValue(v: CellView | undefined): number | null {
  if (!v) return null;
  if (v.kind === "number") return v.num ?? numberOf(v.text);
  return null;
}

function aggregate(fn: AggFn, values: readonly (CellView | undefined)[]): number | null {
  const nums = values.map(cellValue).filter((n): n is number => n !== null);
  const nonEmpty = values.filter((v) => v && v.kind !== "empty" && v.text !== "").length;
  switch (fn) {
    case "sum":
      return nums.reduce((a, b) => a + b, 0);
    case "count":
      return nonEmpty;
    case "countNums":
      return nums.length;
    case "average":
      return nums.length ? nums.reduce((a, b) => a + b, 0) / nums.length : null;
    case "max":
      return nums.length ? Math.max(...nums) : null;
    case "min":
      return nums.length ? Math.min(...nums) : null;
    case "product":
      return nums.length ? nums.reduce((a, b) => a * b, 1) : null;
    case "stDev":
    case "var": {
      if (nums.length < 2) return null;
      const m = nums.reduce((a, b) => a + b, 0) / nums.length;
      const v = nums.reduce((a, b) => a + (b - m) ** 2, 0) / (nums.length - 1);
      return fn === "var" ? v : Math.sqrt(v);
    }
    case "stDevP":
    case "varP": {
      if (nums.length < 1) return null;
      const m = nums.reduce((a, b) => a + b, 0) / nums.length;
      const v = nums.reduce((a, b) => a + (b - m) ** 2, 0) / nums.length;
      return fn === "varP" ? v : Math.sqrt(v);
    }
  }
}

const fmt = (n: number | null): string => (n === null ? "" : String(Number(n.toPrecision(12))));

function grid(cells: readonly CellView[]): Map<string, CellView> {
  return new Map(cells.map((v) => [`${v.r},${v.c}`, v]));
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

function matches(text: string, c: StdCondition, opts: StdFilterOptions, column: readonly string[]): boolean {
  const fold = (s: string) => (opts.caseSensitive ? s : s.toLowerCase());
  const a = numberOf(text);
  const b = numberOf(c.value);
  const cmp = a !== null && b !== null ? a - b : fold(text).localeCompare(fold(c.value));
  const eq = opts.regex ? safeRegex(c.value, opts.caseSensitive)?.test(text) === true : fold(text) === fold(c.value);
  switch (c.op) {
    case "eq":
      return eq;
    case "ne":
      return !eq;
    case "lt":
      return cmp < 0;
    case "gt":
      return cmp > 0;
    case "le":
      return cmp <= 0;
    case "ge":
      return cmp >= 0;
    case "contains":
      return fold(text).includes(fold(c.value));
    case "notContains":
      return !fold(text).includes(fold(c.value));
    case "beginsWith":
      return fold(text).startsWith(fold(c.value));
    case "notBeginsWith":
      return !fold(text).startsWith(fold(c.value));
    case "endsWith":
      return fold(text).endsWith(fold(c.value));
    case "notEndsWith":
      return !fold(text).endsWith(fold(c.value));
    case "largest":
    case "smallest":
    case "largestPct":
    case "smallestPct": {
      const nums = column.map(numberOf).filter((n): n is number => n !== null).sort((x, y) => y - x);
      const n = Number(c.value);
      const k = c.op.endsWith("Pct") ? Math.max(1, Math.round((nums.length * n) / 100)) : Math.max(1, Math.round(n));
      const me = numberOf(text);
      if (me === null) return false;
      const top = c.op.startsWith("largest") ? nums.slice(0, k) : nums.slice(-k);
      return top.includes(me);
    }
  }
}

function safeRegex(src: string, cs: boolean): RegExp | null {
  try {
    return new RegExp(`^(?:${src})$`, cs ? "" : "i");
  } catch {
    return null;
  }
}

/** Body rows hidden by a standard filter. */
export function stdFilterHidden(f: { range: RangeDto; conditions: StdCondition[]; options: StdFilterOptions }, cells: readonly CellView[]): number[] {
  const g = grid(cells);
  const r0 = f.options.hasHeader ? f.range.r0 + 1 : f.range.r0;
  const columns = new Map<number, string[]>();
  for (const c of f.conditions) {
    const col: string[] = [];
    for (let r = r0; r <= f.range.r1; r++) col.push(g.get(`${r},${c.col}`)?.text ?? "");
    columns.set(c.col, col);
  }
  const hidden: number[] = [];
  const seen = new Set<string>();
  for (let r = r0; r <= f.range.r1; r++) {
    let ok: boolean | null = null;
    for (const c of f.conditions) {
      const m = matches(g.get(`${r},${c.col}`)?.text ?? "", c, f.options, columns.get(c.col) ?? []);
      ok = ok === null ? m : c.connector === "or" ? ok || m : ok && m;
    }
    if (ok === false) {
      hidden.push(r);
      continue;
    }
    if (f.options.noDuplicates) {
      const key: string[] = [];
      for (let c = f.range.c0; c <= f.range.c1; c++) key.push(g.get(`${r},${c}`)?.text ?? "");
      const k = key.join("\u0001");
      if (seen.has(k)) hidden.push(r);
      seen.add(k);
    }
  }
  return hidden;
}

/** Criteria block → standard conditions: each row after the labels is an OR alternative. */
function criteriaConditions(criteria: readonly CellView[], crit: RangeDto, range: RangeDto, headers: Map<string, number>): StdCondition[] {
  const g = grid(criteria);
  const out: StdCondition[] = [];
  for (let r = crit.r0 + 1; r <= crit.r1; r++) {
    let first = true;
    for (let c = crit.c0; c <= crit.c1; c++) {
      const label = g.get(`${crit.r0},${c}`)?.text.trim().toLowerCase() ?? "";
      const text = g.get(`${r},${c}`)?.text.trim() ?? "";
      const col = headers.get(label);
      if (text === "" || col === undefined || col < range.c0 || col > range.c1) continue;
      const m = /^(<=|>=|<>|<|>|=)?(.*)$/.exec(text);
      const op = m?.[1] === "<=" ? "le" : m?.[1] === ">=" ? "ge" : m?.[1] === "<>" ? "ne" : m?.[1] === "<" ? "lt" : m?.[1] === ">" ? "gt" : "eq";
      out.push({ connector: first ? "or" : "and", col, op, value: m?.[2] ?? text });
      first = false;
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Pivot computation
// ---------------------------------------------------------------------------

interface PivotCache {
  header: string[];
  rows: CellView[][];
}

async function readSource(p: PivotDto, ctx: W56Ctx, dbRanges: readonly DatabaseRangeDto[]): Promise<{ sheet: number; range: RangeDto; cache: PivotCache } | null> {
  let sheet: number;
  let range: RangeDto;
  if (p.source.type === "range") {
    sheet = p.source.sheet;
    range = p.source.range;
  } else {
    const db = dbRanges.find((d) => d.name.toLowerCase() === (p.source as { name: string }).name.toLowerCase());
    if (!db) return null;
    sheet = db.sheet;
    range = db.range;
  }
  const cells = await ctx.cells(sheet, range);
  const g = grid(cells);
  const header: string[] = [];
  for (let c = range.c0; c <= range.c1; c++) header.push(g.get(`${range.r0},${c}`)?.text ?? `Column ${c - range.c0 + 1}`);
  const rows: CellView[][] = [];
  for (let r = range.r0 + 1; r <= range.r1; r++) {
    const row: CellView[] = [];
    let any = false;
    for (let c = range.c0; c <= range.c1; c++) {
      const v = g.get(`${r},${c}`) ?? { r, c, text: "", kind: "empty" as const, num: null, style: 0, color: null, flags: 0 };
      if (v.kind !== "empty" && v.text !== "") any = true;
      row.push(v);
    }
    if (any || !p.ignoreEmptyRows) rows.push(row);
  }
  return { sheet, range, cache: { header, rows } };
}

function sortItems(items: string[]): string[] {
  return [...items].sort((a, b) => {
    const na = numberOf(a);
    const nb = numberOf(b);
    if (na !== null && nb !== null) return na - nb;
    if (na !== null) return -1;
    if (nb !== null) return 1;
    return a.localeCompare(b, undefined, { sensitivity: "base" });
  });
}

/** Lays the pivot out as text rows (AOO tabular layout, one row and one column level). */
export function computePivot(p: PivotDto, cache: PivotCache): string[][] {
  const col = (sc: number) => sc;
  let rows = cache.rows;
  for (const pf of p.pageFields) if (pf.pageItem !== null) rows = rows.filter((r) => (r[col(pf.sourceCol)]?.text ?? "") === pf.pageItem);
  const rf = p.rowFields;
  const cf = p.colFields;
  const key = (r: CellView[], fields: typeof rf) => fields.map((f) => r[col(f.sourceCol)]?.text ?? "");
  const rowKeys = new Map<string, string[]>();
  const colKeys = new Map<string, string[]>();
  for (const r of rows) {
    const rk = key(r, rf);
    const ck = key(r, cf);
    rowKeys.set(rk.join("\u0001"), rk);
    colKeys.set(ck.join("\u0001"), ck);
  }
  const orderKeys = (m: Map<string, string[]>, fields: typeof rf) => {
    const list = [...m.values()];
    const hidden = (k: string[]) => fields.some((f, i) => f.hiddenItems.includes(k[i] ?? ""));
    const out = list.filter((k) => !hidden(k));
    out.sort((a, b) => {
      for (let i = 0; i < fields.length; i++) {
        const sorted = sortItems([a[i] ?? "", b[i] ?? ""]);
        const f = fields[i];
        const d = (a[i] ?? "") === (b[i] ?? "") ? 0 : sorted[0] === (a[i] ?? "") ? -1 : 1;
        if (d !== 0) return f?.sort.type === "name" && !f.sort.ascending ? -d : d;
      }
      return 0;
    });
    return out;
  };
  const rks = orderKeys(rowKeys, rf);
  const cks = orderKeys(colKeys, cf);
  const data = p.dataFields;
  const multi = data.length > 1;
  const captions = data.map((d) => d.caption ?? `${aggLabel(d.function)} - ${cache.header[d.sourceCol] ?? ""}`);
  const out: string[][] = [];
  if (p.showFilterButton) out.push(["Filter"]);
  for (const pf of p.pageFields) out.push([cache.header[pf.sourceCol] ?? "", pf.pageItem ?? "- all -"]);
  if (p.pageFields.length > 0) out.push([]);
  const nRowCols = Math.max(1, rf.length);
  // Header rows.
  if (cf.length > 0) {
    const h1: string[] = Array.from({ length: nRowCols }, (_, i) => (i === 0 ? (multi ? "Data" : captions[0] ?? "") : ""));
    h1[0] = cf.map((f) => cache.header[f.sourceCol] ?? "").join(" / ");
    for (const ck of cks) for (let d = 0; d < data.length; d++) h1.push(multi ? `${ck.join(" ")} ${captions[d] ?? ""}` : ck.join(" "));
    if (p.colTotals) for (let d = 0; d < data.length; d++) h1.push(multi ? `Total Result ${captions[d] ?? ""}` : "Total Result");
    out.push(h1);
    const h2: string[] = rf.map((f) => cache.header[f.sourceCol] ?? "");
    if (rf.length === 0) h2.push("");
    out.push(h2);
  } else {
    const h: string[] = rf.map((f) => cache.header[f.sourceCol] ?? "");
    if (rf.length === 0) h.push("");
    for (let d = 0; d < data.length; d++) h.push(captions[d] ?? "");
    out.push(h);
  }
  const agg = (rk: string[] | null, ck: string[] | null, d: number): string => {
    const df = data[d];
    if (!df) return "";
    const vals = rows.filter((r) => (rk === null || key(r, rf).join("\u0001") === rk.join("\u0001")) && (ck === null || key(r, cf).join("\u0001") === ck.join("\u0001"))).map((r) => r[col(df.sourceCol)]);
    return fmt(aggregate(df.function, vals));
  };
  const line = (label: string[], rk: string[] | null): string[] => {
    const row = [...label];
    while (row.length < nRowCols) row.push("");
    if (cf.length > 0) {
      for (const ck of cks) for (let d = 0; d < data.length; d++) row.push(agg(rk, ck, d));
      if (p.colTotals) for (let d = 0; d < data.length; d++) row.push(agg(rk, null, d));
    } else {
      for (let d = 0; d < data.length; d++) row.push(agg(rk, null, d));
    }
    return row;
  };
  let prev: string[] | null = null;
  for (const rk of rks) {
    const label = rk.map((k, i) => (prev && prev[i] === k && rf.slice(0, i + 1).every((_, j) => prev?.[j] === rk[j]) ? "" : k));
    // Subtotal of the outer field when it changes (two row fields, auto subtotals).
    if (rf.length > 1 && prev && prev[0] !== rk[0] && rf[0]?.subtotals.type !== "none") out.push(line([`${prev[0] ?? ""} ${aggLabel(data[0]?.function ?? "sum")}`, ...Array.from({ length: nRowCols - 1 }, () => "")], [prev[0] ?? ""]).map((v, i) => (i === 0 ? v : i < nRowCols ? "" : v)));
    out.push(line(label, rk));
    prev = rk;
  }
  if (rf.length > 1 && prev && rf[0]?.subtotals.type !== "none") out.push(line([`${prev[0] ?? ""} ${aggLabel(data[0]?.function ?? "sum")}`], [prev[0] ?? ""]));
  if (p.rowTotals) out.push(line(["Total Result"], null));
  const width = Math.max(...out.map((r) => r.length));
  return out.map((r) => [...r, ...Array.from({ length: width - r.length }, () => "")]);
}

/** Group subtotal rows need the data filtered by the outer key only: `line()` above handles that through `rk` of length 1. */

async function pivotOps(x: W56Ext, p: PivotDto, ctx: W56Ctx, overwrite: boolean, previous: RangeDto | null): Promise<{ ops: Op[]; output: RangeDto } | { error: string; code: string }> {
  const src = await readSource(p, ctx, x.dbRanges);
  if (!src) return { error: "The pivot source range does not exist.", code: "not_found" };
  const rows = computePivot(p, src.cache);
  const h = rows.length;
  const w = Math.max(1, ...rows.map((r) => r.length));
  const output = { r0: p.target.r, c0: p.target.c, r1: p.target.r + h - 1, c1: p.target.c + w - 1 };
  if (!overwrite) {
    const cells = await ctx.cells(p.targetSheet, output);
    const clash = cells.some((v) => v.text !== "" && !(previous && inside(previous, v.r, v.c)));
    if (clash) return { error: "Destination range is not empty.", code: "target_not_empty" };
  }
  const ops: Op[] = [];
  if (previous) ops.push({ type: "clear", sheet: p.targetSheet, ranges: [{ ...previous }], what: "all" });
  ops.push({ type: "setRangeInputs", sheet: p.targetSheet, r: output.r0, c: output.c0, rows });
  const headerRows = (p.showFilterButton ? 1 : 0) + (p.pageFields.length > 0 ? p.pageFields.length + 1 : 0);
  const bold: StylePatch = { bold: true, italic: null, underline: null, strike: null, fontFamily: null, fontSizePt: null, color: null, fill: null, hAlign: null, vAlign: null, wrap: null, numberFormat: null };
  const hdr = { r0: output.r0 + headerRows, c0: output.c0, r1: Math.min(output.r1, output.r0 + headerRows + (p.colFields.length > 0 ? 1 : 0)), c1: output.c1 };
  ops.push({ type: "applyStyle", sheet: p.targetSheet, ranges: [hdr], patch: bold });
  if (p.rowTotals) ops.push({ type: "applyStyle", sheet: p.targetSheet, ranges: [{ r0: output.r1, c0: output.c0, r1: output.r1, c1: output.c1 }], patch: { ...bold, fill: 0xeef2ffff } });
  return { ops, output };
}

/** `pivot_at`. */
export function pivotHitAt(x: W56Ext, sheet: number, r: number, c: number): PivotHit | null {
  for (const p of x.pivots) {
    const o = p.output;
    if (p.targetSheet !== sheet || !o || !inside(o, r, c)) continue;
    const headerRows = (p.showFilterButton ? 1 : 0) + (p.pageFields.length > 0 ? p.pageFields.length + 1 : 0);
    const top = o.r0 + headerRows + (p.colFields.length > 0 ? 1 : 0);
    if (r < o.r0 + headerRows) return { pivot: p.id, role: "pageField", field: null, item: null };
    if (r <= top) return { pivot: p.id, role: "header", field: null, item: null };
    if (p.rowTotals && r === o.r1) return { pivot: p.id, role: "total", field: null, item: null };
    if (c < o.c0 + Math.max(1, p.rowFields.length)) return { pivot: p.id, role: "rowItem", field: p.rowFields[c - o.c0]?.sourceCol ?? null, item: null };
    if (p.colTotals && c === o.c1) return { pivot: p.id, role: "total", field: null, item: null };
    return { pivot: p.id, role: "data", field: null, item: null };
  }
  return null;
}

/** `pivot_field_items`. */
export async function pivotFieldItemsOf(source: PivotDto["source"], ctx: W56Ctx, dbRanges: readonly DatabaseRangeDto[]): Promise<PivotFieldItemsDto[]> {
  const fake: PivotDto = { id: 0, name: "", source, targetSheet: 0, target: { r: 0, c: 0 }, output: null, rowFields: [], colFields: [], pageFields: [], dataFields: [], dataLayout: "column", rowTotals: true, colTotals: true, ignoreEmptyRows: true, identifyCategories: false, showFilterButton: false, drillDown: true };
  const src = await readSource(fake, ctx, dbRanges);
  if (!src) throw { code: "not_found", message: "the source range does not exist" };
  return src.cache.header.map((name, i) => {
    const counts = new Map<string, number>();
    let numeric = src.cache.rows.length > 0;
    for (const row of src.cache.rows) {
      const v = row[i];
      const text = v?.text ?? "";
      counts.set(text, (counts.get(text) ?? 0) + 1);
      if (!v || v.kind !== "number") numeric = numeric && text === "";
    }
    const items = sortItems([...counts.keys()]).map((text) => ({ text, count: counts.get(text) ?? 0 }));
    return { sourceCol: i, name: name || `Column ${i + 1}`, numeric, date: false, items: items.slice(0, 10_000) };
  });
}

// ---------------------------------------------------------------------------
// Other emulations
// ---------------------------------------------------------------------------

function styleToPatch(s: StyleDto): StylePatch {
  return { bold: s.bold, italic: s.italic, underline: s.underline, strike: s.strike, fontFamily: s.fontFamily, fontSizePt: s.fontSizePt, color: s.color, fill: s.fill, hAlign: s.hAlign, vAlign: s.vAlign, wrap: s.wrap, numberFormat: s.numberFormat };
}

const BASE_STYLE: StyleDto = { id: 0, bold: false, italic: false, underline: false, strike: false, fontFamily: null, fontSizePt: 11, color: null, fill: null, hAlign: "general", vAlign: "bottom", wrap: false, indent: 0, borders: { top: null, bottom: null, left: null, right: null }, numberFormat: "General", display: null };

/** The built-in styles (AOO's Default, Heading, Heading 1, Result, Result 2) plus the user's. */
export function builtinStyles(): CellStyleDto[] {
  return [
    { name: "Default", parent: null, builtin: true, style: BASE_STYLE },
    { name: "Heading", parent: "Default", builtin: true, style: { ...BASE_STYLE, bold: true, fontSizePt: 16, hAlign: "center" } },
    { name: "Heading 1", parent: "Heading", builtin: true, style: { ...BASE_STYLE, bold: true, fontSizePt: 16, rotationDeg: 90 } },
    { name: "Result", parent: "Default", builtin: true, style: { ...BASE_STYLE, bold: true, italic: true, underline: true } },
    { name: "Result 2", parent: "Result", builtin: true, style: { ...BASE_STYLE, bold: true, italic: true, underline: true, numberFormat: "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00" } },
  ];
}

export function cellStylesOf(x: W56Ext): CellStyleDto[] {
  const custom = x.styles.filter((s) => !builtinStyles().some((b) => b.name === s.name));
  const overridden = builtinStyles().map((b) => x.styles.find((s) => s.name === b.name) ?? b);
  return [...overridden, ...custom];
}

function seriesValues(op: Extract<Op, { type: "fillSeries" }>, first: string, count: number): string[] {
  const start = op.start ?? first;
  const inc = Number(op.increment ?? "1");
  const end = op.end === null ? null : Number(op.end);
  const out: string[] = [];
  if (op.kind === "date") {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(start) ?? /^(\d{1,2})\/(\d{1,2})\/(\d{4})$/.exec(start);
    if (!m) return [];
    const iso = start.includes("-");
    let d = iso ? new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]))) : new Date(Date.UTC(Number(m[3]), Number(m[1]) - 1, Number(m[2])));
    for (let i = 0; i < count; i++) {
      const y = d.getUTCFullYear();
      const mo = d.getUTCMonth() + 1;
      const da = d.getUTCDate();
      out.push(iso ? `${y}-${String(mo).padStart(2, "0")}-${String(da).padStart(2, "0")}` : `${mo}/${da}/${y}`);
      const step = Number.isFinite(inc) ? inc : 1;
      if (op.unit === "month") d = new Date(Date.UTC(y, mo - 1 + step, da));
      else if (op.unit === "year") d = new Date(Date.UTC(y + step, mo - 1, da));
      else {
        d = new Date(d.getTime() + step * 86_400_000);
        if (op.unit === "weekday") while (d.getUTCDay() === 0 || d.getUTCDay() === 6) d = new Date(d.getTime() + 86_400_000);
      }
    }
    return out;
  }
  const s = Number(start);
  if (!Number.isFinite(s) || !Number.isFinite(inc)) return op.kind === "autoFill" ? Array.from({ length: count }, () => start) : [];
  let v = s;
  for (let i = 0; i < count; i++) {
    if (end !== null && Number.isFinite(end) && (inc >= 0 ? v > end : v < end)) break;
    out.push(fmt(v));
    v = op.kind === "growth" ? v * inc : v + inc;
  }
  return out;
}

/** The hidden/visible sheet list and tab colours applied to a `WorkbookInfo`. */
export function decorateSheets(x: W56Ext, sheets: SheetInfo[]): SheetInfo[] {
  return sheets.map((s) => ({ ...s, hidden: x.hiddenSheets.includes(s.id), tabColor: x.tabColors[s.id] ?? s.tabColor }));
}

/** `autocomplete`: text cells of the column's data block around `r`. */
export function autocompleteOf(cells: readonly CellView[], r: number, c: number, prefix: string): AutocompleteDto {
  const col = cells.filter((v) => v.c === c && v.r !== r && v.kind === "text" && v.text !== "").sort((a, b) => a.r - b.r);
  // The contiguous block: walk up and down from r over non-empty cells.
  const present = new Set(cells.filter((v) => v.c === c && v.text !== "").map((v) => v.r));
  const inBlock = new Set<number>();
  for (let i = r - 1; i >= 0 && present.has(i); i--) inBlock.add(i);
  for (let i = r + 1; present.has(i); i++) inBlock.add(i);
  const texts = col.filter((v) => inBlock.has(v.r)).map((v) => v.text);
  return { completion: completionFor(prefix, texts), items: matchingItems(prefix, texts) };
}

/** `navigator`. */
export async function navigatorOf(x: W56Ext, ctx: W56Ctx, names: { name: string; refersTo: string }[], charts: { sheet: number; id: number; title: string; r: number; c: number }[]): Promise<{ sheets: NavItem[]; names: NavItem[]; databaseRanges: NavItem[]; charts: NavItem[]; images: NavItem[]; pivots: NavItem[]; notes: NavItem[]; scenarios: NavItem[] }> {
  const sheets: NavItem[] = [];
  const notes: NavItem[] = [];
  const scenarios: NavItem[] = [];
  for (const s of ctx.sheets()) {
    sheets.push({ title: s.name, sheet: s.id, range: await ctx.used(s.id), id: null });
    for (const n of ctx.notes(s.id)) notes.push({ title: `${s.name}.${absoluteRef("", { r0: n.r, c0: n.c, r1: n.r, c1: n.c }).replace(/^\$?\.?/, "").replace(/\$/g, "")}`, sheet: s.id, range: { r0: n.r, c0: n.c, r1: n.r, c1: n.c }, id: null });
    for (const sc of x.scenarios[s.id] ?? []) scenarios.push({ title: sc.name, sheet: s.id, range: sc.ranges[0] ?? null, id: sc.id });
  }
  return {
    sheets,
    names: names.map((n) => ({ title: n.name, sheet: null, range: null, id: null })),
    databaseRanges: x.dbRanges.map((d) => ({ title: d.name, sheet: d.sheet, range: d.range, id: null })),
    charts: charts.map((c) => ({ title: c.title, sheet: c.sheet, range: { r0: c.r, c0: c.c, r1: c.r, c1: c.c }, id: c.id })),
    images: [],
    pivots: x.pivots.map((p) => ({ title: p.name, sheet: p.targetSheet, range: p.output, id: p.id })),
    notes,
    scenarios,
  };
}

// ---------------------------------------------------------------------------
// The op emulation
// ---------------------------------------------------------------------------

export async function applyW56Op(x: W56Ext, op: Op, ctx: W56Ctx): Promise<W56Outcome> {
  const err = (code: string, message: string): W56Outcome => ({ kind: "error", code, message });
  const handled = (ops: Op[], sheet: number | null, touched: RangeDto[] = [], layout = false, pivotsChanged = false): W56Outcome => {
    const t = new Map<number, RangeDto[]>();
    if (sheet !== null && touched.length > 0) t.set(sheet, touched);
    const l = new Set<number>();
    if (sheet !== null && layout) l.add(sheet);
    x.log.push(op.type);
    return { kind: "handled", ops, touched: t, layout: l, pivotsChanged };
  };
  switch (op.type) {
    // -- outline ---------------------------------------------------------------
    case "group":
    case "ungroup":
    case "setOutlineCollapsed":
    case "showOutlineLevel":
    case "clearOutline":
    case "autoOutline": {
      const before = x.outline[op.sheet] ?? emptyOutlineModel();
      let next: OutlineModel;
      if (op.type === "autoOutline") {
        const range = op.range ?? (await ctx.used(op.sheet));
        if (!range) return handled([], op.sheet);
        const cells = await ctx.cells(op.sheet, range);
        const g = grid(cells);
        next = autoOutlineGroups((r, c) => ((g.get(`${r},${c}`)?.flags ?? 0) & 1) !== 0, range);
      } else next = applyOutlineOp(before, op);
      x.outline[op.sheet] = next;
      const wasRows = hiddenByOutline(before.rows);
      const wasCols = hiddenByOutline(before.cols);
      ctx.setHiddenRows(op.sheet, wasRows, false);
      ctx.setHiddenCols(op.sheet, wasCols, false);
      ctx.setHiddenRows(op.sheet, hiddenByOutline(next.rows), true);
      ctx.setHiddenCols(op.sheet, hiddenByOutline(next.cols), true);
      return handled([], op.sheet, [], true);
    }
    // -- filters ---------------------------------------------------------------
    case "standardFilter": {
      if (op.conditions.length === 0 || op.conditions.length > 4) return err("invalid_argument", "between 1 and 4 conditions");
      if (op.options.copyTo) {
        const cells = await ctx.cells(op.sheet, op.range);
        const hidden = new Set(stdFilterHidden({ range: op.range, conditions: op.conditions, options: op.options }, cells));
        const inputs = await ctx.inputs(op.sheet, op.range);
        const rows = inputs.filter((_, i) => !hidden.has(op.range.r0 + i));
        return handled([{ type: "setRangeInputs", sheet: op.options.copyTo.sheet, r: op.options.copyTo.r, c: op.options.copyTo.c, rows }], op.options.copyTo.sheet);
      }
      x.stdFilters[op.sheet] = { range: { ...op.range }, conditions: structuredClone(op.conditions), options: { ...op.options } };
      return handled([], op.sheet, [], true);
    }
    case "advancedFilter": {
      const cells = await ctx.cells(op.sheet, op.range);
      const g = grid(cells);
      const headers = new Map<string, number>();
      for (let c = op.range.c0; c <= op.range.c1; c++) headers.set((g.get(`${op.range.r0},${c}`)?.text ?? "").trim().toLowerCase(), c);
      const crit = await ctx.cells(op.criteria.sheet, op.criteria.range);
      const conditions = criteriaConditions(crit, op.criteria.range, op.range, headers);
      if (conditions.length === 0) return err("invalid_argument", "the criteria range has no usable conditions");
      x.stdFilters[op.sheet] = { range: { ...op.range }, conditions, options: { ...op.options, hasHeader: true } };
      return handled([], op.sheet, [], true);
    }
    case "resetFilter": {
      x.stdFilters[op.sheet] = null;
      return handled([], op.sheet, [], true);
    }
    // -- subtotals ---------------------------------------------------------------
    case "subtotals": {
      if (op.groups.length === 0) return err("invalid_argument", "no groups");
      const body = { ...op.range, r0: op.range.r0 + 1 };
      if (body.r1 < body.r0) return handled([], op.sheet);
      const inputs = await ctx.inputs(op.sheet, body);
      const cells = await ctx.cells(op.sheet, body);
      const g = grid(cells);
      const textAt = (i: number, col: number) => g.get(`${body.r0 + i},${col}`)?.text ?? "";
      let order = inputs.map((_, i) => i);
      const g0 = op.groups[0]!;
      if (op.options.presort) order.sort((a, b) => textAt(a, g0.groupBy).localeCompare(textAt(b, g0.groupBy), undefined, { sensitivity: op.options.caseSensitive ? "case" : "base" }) * (op.options.ascending ? 1 : -1) || a - b);
      const fnMeta = SUBTOTAL_FNS.find((f) => f.fn === g0.function) ?? SUBTOTAL_FNS[0]!;
      const out: string[][] = [];
      const groupRows: { start: number; end: number }[] = [];
      const colLetter = (c: number) => {
        let s = "";
        let n = c + 1;
        while (n > 0) {
          const m = (n - 1) % 26;
          s = String.fromCharCode(65 + m) + s;
          n = Math.floor((n - 1) / 26);
        }
        return s;
      };
      let i = 0;
      let outRow = body.r0;
      while (i < order.length) {
        const value = textAt(order[i]!, g0.groupBy);
        const start = outRow;
        let j = i;
        while (j < order.length && textAt(order[j]!, g0.groupBy) === value) {
          out.push(inputs[order[j]!] ?? []);
          outRow++;
          j++;
        }
        const total: string[] = Array.from({ length: op.range.c1 - op.range.c0 + 1 }, () => "");
        total[g0.groupBy - op.range.c0] = `${value} ${fnMeta.label}`;
        for (const c of g0.columns) total[c - op.range.c0] = `=SUBTOTAL(${fnMeta.n};${colLetter(c)}${start + 1}:${colLetter(c)}${outRow})`;
        out.push(total);
        groupRows.push({ start, end: outRow - 1 });
        outRow++;
        i = j;
      }
      const grand: string[] = Array.from({ length: op.range.c1 - op.range.c0 + 1 }, () => "");
      grand[g0.groupBy - op.range.c0] = "Grand Total";
      for (const c of g0.columns) grand[c - op.range.c0] = `=SUBTOTAL(${fnMeta.n};${colLetter(c)}${body.r0 + 1}:${colLetter(c)}${outRow})`;
      out.push(grand);
      const extra = out.length - inputs.length;
      const ops: Op[] = [];
      if (extra > 0) ops.push({ type: "insertRows", sheet: op.sheet, at: op.range.r1 + 1, count: extra });
      ops.push({ type: "setRangeInputs", sheet: op.sheet, r: body.r0, c: op.range.c0, rows: out });
      const model = x.outline[op.sheet] ?? emptyOutlineModel();
      model.rows = [{ start: body.r0, end: body.r0 + out.length - 2, level: 1, collapsed: false }, ...groupRows.map((gr) => ({ ...gr, level: 2, collapsed: false }))];
      x.outline[op.sheet] = model;
      return handled(ops, op.sheet, [{ ...op.range, r1: op.range.r1 + extra }], true);
    }
    case "removeSubtotals": {
      const inputs = await ctx.inputs(op.sheet, op.range);
      const rows: number[] = [];
      inputs.forEach((row, i) => {
        if (row.some((v) => /^=SUBTOTAL\(/i.test(v))) rows.push(op.range.r0 + i);
      });
      const ops: Op[] = [...rows].reverse().map((r): Op => ({ type: "deleteRows", sheet: op.sheet, at: r, count: 1 }));
      x.outline[op.sheet] = emptyOutlineModel();
      return handled(ops, op.sheet, [op.range], true);
    }
    // -- consolidate, text to columns, duplicates ------------------------------------------
    case "consolidate": {
      const byRow = new Map<string, Map<string, (CellView | undefined)[]>>();
      const colOrder: string[] = [];
      const rowOrder: string[] = [];
      for (const src of op.sources) {
        const cells = await ctx.cells(src.sheet, src.range);
        const g = grid(cells);
        for (let r = src.range.r0 + (op.byColLabels ? 1 : 0); r <= src.range.r1; r++) {
          const rl = op.byRowLabels ? (g.get(`${r},${src.range.c0}`)?.text ?? "") : String(r - src.range.r0);
          if (!rowOrder.includes(rl)) rowOrder.push(rl);
          const row = byRow.get(rl) ?? new Map<string, (CellView | undefined)[]>();
          for (let c = src.range.c0 + (op.byRowLabels ? 1 : 0); c <= src.range.c1; c++) {
            const cl = op.byColLabels ? (g.get(`${src.range.r0},${c}`)?.text ?? "") : String(c - src.range.c0);
            if (!colOrder.includes(cl)) colOrder.push(cl);
            const list = row.get(cl) ?? [];
            list.push(g.get(`${r},${c}`));
            row.set(cl, list);
          }
          byRow.set(rl, row);
        }
      }
      const rows: string[][] = [];
      if (op.byColLabels) rows.push([op.byRowLabels ? "" : "", ...colOrder].slice(op.byRowLabels ? 0 : 1));
      for (const rl of rowOrder) {
        const line: string[] = op.byRowLabels ? [rl] : [];
        for (const cl of colOrder) line.push(fmt(aggregate(op.function, byRow.get(rl)?.get(cl) ?? [])));
        rows.push(line);
      }
      return handled([{ type: "setRangeInputs", sheet: op.dest.sheet, r: op.dest.r, c: op.dest.c, rows }], op.dest.sheet);
    }
    case "textToColumns": {
      const cells = await ctx.cells(op.sheet, op.range);
      const g = grid(cells);
      const o = op.options;
      const st: SeparatorState = { tab: o.delimiters.includes("\t"), comma: o.delimiters.includes(","), semicolon: o.delimiters.includes(";"), space: o.delimiters.includes(" "), other: /[^\t,; ]/.test(o.delimiters), otherChar: o.delimiters.replace(/[\t,; ]/g, "")[0] ?? "", mergeDelimiters: o.mergeDelimiters, quote: o.quote === "'" ? "'" : '"', trim: o.trim, detectNumbers: o.detectNumbers, fixedWidth: o.fixedWidths !== null, widths: o.fixedWidths ?? [], columnTypes: o.columnTypes };
      const ops: Op[] = [];
      let maxW = 1;
      for (let r = op.range.r0; r <= op.range.r1; r++) {
        const text = g.get(`${r},${op.range.c0}`)?.text ?? "";
        if (text === "") continue;
        const parts = splitLine(text, st).map((p, i) => (o.columnTypes[i] === "hide" ? "" : o.columnTypes[i] === "text" && numberOf(p) !== null ? `'${p}` : p));
        maxW = Math.max(maxW, parts.length);
        ops.push({ type: "setRangeInputs", sheet: op.sheet, r, c: op.range.c0, rows: [parts] });
      }
      return handled(ops, op.sheet, [{ ...op.range, c1: op.range.c0 + maxW - 1 }]);
    }
    case "removeDuplicates": {
      const inputs = await ctx.inputs(op.sheet, op.range);
      const cells = await ctx.cells(op.sheet, op.range);
      const g = grid(cells);
      const cols = op.cols.length === 0 ? Array.from({ length: op.range.c1 - op.range.c0 + 1 }, (_, i) => op.range.c0 + i) : op.cols;
      const seen = new Set<string>();
      const kept: string[][] = [];
      inputs.forEach((row, i) => {
        if (op.hasHeader && i === 0) {
          kept.push(row);
          return;
        }
        const key = cols.map((c) => g.get(`${op.range.r0 + i},${c}`)?.text ?? "").join("\u0001");
        if (seen.has(key)) return;
        seen.add(key);
        kept.push(row);
      });
      const removed = inputs.length - kept.length;
      if (removed === 0) return handled([], op.sheet);
      return handled([{ type: "clear", sheet: op.sheet, ranges: [op.range], what: "contents" }, { type: "setRangeInputs", sheet: op.sheet, r: op.range.r0, c: op.range.c0, rows: kept }], op.sheet, [op.range]);
    }
    // -- cells ----------------------------------------------------------------------------
    case "insertCells":
    case "deleteCells": {
      const used = await ctx.used(op.sheet);
      if (!used) return handled([], op.sheet);
      const vertical = op.shift === "down" || op.shift === "up";
      const n = vertical ? op.range.r1 - op.range.r0 + 1 : op.range.c1 - op.range.c0 + 1;
      const insert = op.type === "insertCells";
      const from = insert ? (vertical ? op.range.r0 : op.range.c0) : (vertical ? op.range.r1 + 1 : op.range.c1 + 1);
      const end = vertical ? used.r1 : used.c1;
      const ops: Op[] = [];
      if (from <= end) {
        const block: RangeDto = vertical ? { r0: from, r1: end, c0: op.range.c0, c1: op.range.c1 } : { r0: op.range.r0, r1: op.range.r1, c0: from, c1: end };
        const inputs = await ctx.inputs(op.sheet, block);
        const shift = insert ? n : -n;
        const clearRange: RangeDto = vertical ? { ...block, r0: Math.min(op.range.r0, from) } : { ...block, c0: Math.min(op.range.c0, from) };
        ops.push({ type: "clear", sheet: op.sheet, ranges: [clearRange], what: "all" });
        ops.push({ type: "setRangeInputs", sheet: op.sheet, r: vertical ? block.r0 + shift : block.r0, c: vertical ? block.c0 : block.c0 + shift, rows: inputs });
      } else if (!insert) ops.push({ type: "clear", sheet: op.sheet, ranges: [op.range], what: "all" });
      return handled(ops, op.sheet, [vertical ? { ...op.range, r1: Math.max(end + n, op.range.r1) } : { ...op.range, c1: Math.max(end + n, op.range.c1) }]);
    }
    case "deleteContents": {
      const ops: Op[] = [];
      const w: DeleteContentsFlags = op.what;
      for (const range of op.ranges) {
        const cells = await ctx.cells(op.sheet, range);
        const contents: RangeDto[] = [];
        for (const v of cells) {
          const isFormula = (v.flags & 1) !== 0;
          const isDate = v.kind === "number" && /[/-]/.test(v.text);
          const kill = isFormula ? w.formulas : v.kind === "number" ? (isDate ? w.dates : w.numbers) : v.kind === "text" ? w.text : v.kind === "bool" ? w.numbers : w.text;
          if (kill && v.text !== "") contents.push({ r0: v.r, c0: v.c, r1: v.r, c1: v.c });
        }
        if (contents.length > 0) ops.push({ type: "clear", sheet: op.sheet, ranges: contents, what: "contents" });
        if (w.formats) ops.push({ type: "clear", sheet: op.sheet, ranges: [range], what: "formats" });
        if (w.notes) for (const n of ctx.notes(op.sheet)) if (inside(range, n.r, n.c)) ops.push({ type: "deleteNote", sheet: op.sheet, r: n.r, c: n.c });
      }
      return handled(ops, op.sheet, op.ranges);
    }
    case "fillSeries": {
      const r = op.range;
      const vertical = op.direction === "down" || op.direction === "up";
      const reverse = op.direction === "up" || op.direction === "left";
      const ops: Op[] = [];
      const lines = vertical ? r.c1 - r.c0 + 1 : r.r1 - r.r0 + 1;
      const count = vertical ? r.r1 - r.r0 + 1 : r.c1 - r.c0 + 1;
      for (let k = 0; k < lines; k++) {
        const first = vertical ? (reverse ? { r: r.r1, c: r.c0 + k } : { r: r.r0, c: r.c0 + k }) : reverse ? { r: r.r0 + k, c: r.c1 } : { r: r.r0 + k, c: r.c0 };
        const [v0] = await ctx.cells(op.sheet, { r0: first.r, c0: first.c, r1: first.r, c1: first.c });
        const values = seriesValues(op, v0?.text ?? "", count);
        if (values.length === 0) continue;
        const ordered = reverse ? [...values].reverse() : values;
        const start = reverse ? count - values.length : 0;
        if (vertical) ops.push({ type: "setRangeInputs", sheet: op.sheet, r: r.r0 + start, c: r.c0 + k, rows: ordered.map((v) => [v]) });
        else ops.push({ type: "setRangeInputs", sheet: op.sheet, r: r.r0 + k, c: r.c0 + start, rows: [ordered] });
      }
      return handled(ops, op.sheet, [r]);
    }
    // -- pivots ---------------------------------------------------------------------------------
    case "createPivot":
    case "updatePivot": {
      const existing = op.type === "updatePivot" ? x.pivots.find((p) => p.id === op.id) : undefined;
      if (op.type === "updatePivot" && !existing) return err("not_found", `pivot ${op.id} not found`);
      const id = existing?.id ?? x.nextPivot;
      const p: PivotDto = { ...structuredClone(op.pivot), id, name: op.pivot.name || existing?.name || `DataPilot${id}` };
      const res = await pivotOps(x, p, ctx, op.overwrite, existing?.output ?? null);
      if ("error" in res) return err(res.code, res.error);
      p.output = res.output;
      if (existing) x.pivots = x.pivots.map((q) => (q.id === id ? p : q));
      else {
        x.pivots.push(p);
        x.nextPivot++;
      }
      return handled(res.ops, p.targetSheet, [res.output, ...(existing?.output ? [existing.output] : [])], false, true);
    }
    case "refreshPivot": {
      const p = x.pivots.find((q) => q.id === op.id);
      if (!p) return err("not_found", `pivot ${op.id} not found`);
      const res = await pivotOps(x, p, ctx, true, p.output);
      if ("error" in res) return err(res.code, res.error);
      const prev = p.output;
      p.output = res.output;
      return handled(res.ops, p.targetSheet, [res.output, ...(prev ? [prev] : [])], false, true);
    }
    case "deletePivot": {
      const p = x.pivots.find((q) => q.id === op.id);
      if (!p) return err("not_found", `pivot ${op.id} not found`);
      x.pivots = x.pivots.filter((q) => q.id !== op.id);
      const ops: Op[] = op.clearOutput && p.output ? [{ type: "clear", sheet: p.targetSheet, ranges: [p.output], what: "all" }] : [];
      return handled(ops, p.targetSheet, p.output ? [p.output] : [], false, true);
    }
    case "pivotDrillDown": {
      const p = x.pivots.find((q) => q.id === op.id);
      if (!p) return err("not_found", `pivot ${op.id} not found`);
      const src = await readSource(p, ctx, x.dbRanges);
      if (!src) return err("not_found", "source missing");
      // The row item(s) of the clicked row: the labels left of the data area.
      const o = p.output!;
      const labels = await ctx.cells(p.targetSheet, { r0: op.r, c0: o.c0, r1: op.r, c1: o.c0 + Math.max(0, p.rowFields.length - 1) });
      const g = grid(labels);
      let rows = src.cache.rows;
      p.rowFields.forEach((f, i) => {
        let item = g.get(`${op.r},${o.c0 + i}`)?.text ?? "";
        // Blank label = same as the row above (walk up).
        for (let rr = op.r; item === "" && rr > o.r0; rr--) item = "";
        if (item !== "") rows = rows.filter((r) => (r[f.sourceCol]?.text ?? "") === item);
      });
      const out = [src.cache.header, ...rows.map((r) => r.map((v) => v.text))];
      return handled([{ type: "addSheet", name: null, index: null }, { type: "setRangeInputs", sheet: -1, r: 0, c: 0, rows: out }], p.targetSheet, [], false, false);
    }
    // -- scenarios ------------------------------------------------------------------------------------
    case "createScenario": {
      const values: string[][][] = [];
      for (const r of op.scenario.ranges) values.push(await ctx.inputs(op.sheet, r));
      const list = x.scenarios[op.sheet] ?? [];
      for (const s of list) s.active = false;
      const s: StoredScenario = { ...structuredClone(op.scenario), id: x.nextScenario++, active: true, values };
      list.push(s);
      x.scenarios[op.sheet] = list;
      return handled([], op.sheet);
    }
    case "updateScenario": {
      const list = x.scenarios[op.sheet] ?? [];
      const s = list.find((q) => q.id === op.id);
      if (!s) return err("not_found", "scenario not found");
      Object.assign(s, structuredClone(op.scenario), { id: s.id, values: s.values });
      return handled([], op.sheet);
    }
    case "showScenario": {
      const list = x.scenarios[op.sheet] ?? [];
      const s = list.find((q) => q.id === op.id);
      if (!s) return err("not_found", "scenario not found");
      const active = list.find((q) => q.active);
      if (active && active !== s && active.copyBack) {
        active.values = [];
        for (const r of active.ranges) active.values.push(await ctx.inputs(op.sheet, r));
      }
      for (const q of list) q.active = q === s;
      const ops: Op[] = s.ranges.map((r, i): Op => ({ type: "setRangeInputs", sheet: op.sheet, r: r.r0, c: r.c0, rows: s.values[i] ?? [] }));
      return handled(ops, op.sheet, s.ranges);
    }
    case "deleteScenario": {
      x.scenarios[op.sheet] = (x.scenarios[op.sheet] ?? []).filter((q) => q.id !== op.id);
      return handled([], op.sheet);
    }
    // -- styles ---------------------------------------------------------------------------------------
    case "defineCellStyle": {
      const s = structuredClone(op.style);
      x.styles = [...x.styles.filter((q) => q.name !== s.name), s];
      return handled([], null);
    }
    case "deleteCellStyle": {
      if (builtinStyles().some((b) => b.name === op.name)) return err("invalid_argument", "built-in styles cannot be deleted");
      x.styles = x.styles.filter((q) => q.name !== op.name);
      return handled([], null);
    }
    case "applyCellStyle": {
      const s = cellStylesOf(x).find((q) => q.name === op.name);
      if (!s) return err("not_found", `style ${op.name} not found`);
      return handled([{ type: "applyStyle", sheet: op.sheet, ranges: op.ranges, patch: styleToPatch(s.style) }], op.sheet, op.ranges);
    }
    case "applyAutoFormat": {
      const preset = AUTOFORMAT_PRESETS.find((p) => p.name === op.preset);
      if (!preset) return err("not_found", `AutoFormat ${op.preset} not found`);
      const rgba = (hex: string) => ((parseInt(hex.slice(1), 16) << 8) | 0xff) >>> 0;
      const r = op.range;
      const patch = (fill: string, color: string, bold: boolean): StylePatch => ({ bold: op.options.font ? bold : null, italic: null, underline: null, strike: null, fontFamily: null, fontSizePt: null, color: op.options.pattern ? rgba(color) : null, fill: op.options.pattern ? rgba(fill) : null, hAlign: op.options.alignment ? "right" : null, vAlign: null, wrap: null, numberFormat: op.options.numberFormat && preset.currency ? "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00" : null });
      const ops: Op[] = [
        { type: "applyStyle", sheet: op.sheet, ranges: [r], patch: { ...patch(preset.body[0], preset.body[1], false), hAlign: null } },
        { type: "applyStyle", sheet: op.sheet, ranges: [{ ...r, r1: r.r0 }, { ...r, c1: r.c0 }], patch: { ...patch(preset.header[0], preset.header[1], true), numberFormat: null, hAlign: null } },
        { type: "applyStyle", sheet: op.sheet, ranges: [{ ...r, r0: r.r1 }, { ...r, c0: r.c1 }], patch: { ...patch(preset.total[0], preset.total[1], true), hAlign: null } },
      ];
      return handled(ops, op.sheet, [r]);
    }
    // -- sheets, names, ranges, settings ---------------------------------------------------------------------
    case "setSheetHidden": {
      const visible = ctx.sheets().filter((s) => !x.hiddenSheets.includes(s.id)).length;
      if (op.hidden && visible <= 1 && !x.hiddenSheets.includes(op.sheet)) return err("invalid_argument", "the last visible sheet cannot be hidden");
      x.hiddenSheets = op.hidden ? [...new Set([...x.hiddenSheets, op.sheet])] : x.hiddenSheets.filter((s) => s !== op.sheet);
      return handled([{ type: "renameSheet", sheet: op.sheet, name: ctx.sheetName(op.sheet) }], op.sheet);
    }
    case "setTabColor": {
      if (op.color === null) delete x.tabColors[op.sheet];
      else x.tabColors[op.sheet] = `#${((op.color >>> 8) & 0xffffff).toString(16).padStart(6, "0")}`;
      return handled([{ type: "renameSheet", sheet: op.sheet, name: ctx.sheetName(op.sheet) }], op.sheet);
    }
    case "copySheet": {
      const used = await ctx.used(op.sheet);
      const rows = used ? await ctx.inputs(op.sheet, used) : [];
      const taken = ctx.sheets().map((s) => s.name.toLowerCase());
      let name = op.name ?? `${ctx.sheetName(op.sheet)}_2`;
      for (let i = 2; taken.includes(name.toLowerCase()); i++) name = `${ctx.sheetName(op.sheet)}_${i}`;
      const index = op.index ?? ctx.sheets().findIndex((s) => s.id === op.sheet) + 1;
      const ops: Op[] = [{ type: "addSheet", name, index }];
      if (used && rows.length > 0) ops.push({ type: "setRangeInputs", sheet: -1, r: used.r0, c: used.c0, rows });
      return handled(ops, op.sheet);
    }
    case "createNamesFromLabels": {
      const cells = await ctx.cells(op.sheet, op.range);
      const g = grid(cells);
      const sheetName = ctx.sheetName(op.sheet);
      const ops: Op[] = [];
      const valid = (s: string) => s.replace(/[^A-Za-z0-9_.]/g, "_").replace(/^(?=\d)/, "_");
      const r = op.range;
      if (op.from.top) for (let c = r.c0 + (op.from.left ? 1 : 0); c <= r.c1 - (op.from.right ? 1 : 0); c++) {
        const label = g.get(`${r.r0},${c}`)?.text.trim() ?? "";
        if (label) ops.push({ type: "defineName", name: valid(label), scope: null, refersTo: absoluteRef(sheetName, { r0: r.r0 + 1, c0: c, r1: r.r1 - (op.from.bottom ? 1 : 0), c1: c }) });
      }
      if (op.from.left) for (let rr = r.r0 + (op.from.top ? 1 : 0); rr <= r.r1 - (op.from.bottom ? 1 : 0); rr++) {
        const label = g.get(`${rr},${r.c0}`)?.text.trim() ?? "";
        if (label) ops.push({ type: "defineName", name: valid(label), scope: null, refersTo: absoluteRef(sheetName, { r0: rr, c0: r.c0 + 1, r1: rr, c1: r.c1 - (op.from.right ? 1 : 0) }) });
      }
      if (op.from.bottom) for (let c = r.c0 + (op.from.left ? 1 : 0); c <= r.c1 - (op.from.right ? 1 : 0); c++) {
        const label = g.get(`${r.r1},${c}`)?.text.trim() ?? "";
        if (label) ops.push({ type: "defineName", name: valid(label), scope: null, refersTo: absoluteRef(sheetName, { r0: r.r0 + (op.from.top ? 1 : 0), c0: c, r1: r.r1 - 1, c1: c }) });
      }
      if (op.from.right) for (let rr = r.r0 + (op.from.top ? 1 : 0); rr <= r.r1 - (op.from.bottom ? 1 : 0); rr++) {
        const label = g.get(`${rr},${r.c1}`)?.text.trim() ?? "";
        if (label) ops.push({ type: "defineName", name: valid(label), scope: null, refersTo: absoluteRef(sheetName, { r0: rr, c0: r.c0 + (op.from.left ? 1 : 0), r1: rr, c1: r.c1 - 1 }) });
      }
      return handled(ops, op.sheet);
    }
    case "defineDatabaseRange": {
      x.dbRanges = [...x.dbRanges.filter((d) => d.name.toLowerCase() !== op.name.toLowerCase()), { name: op.name, sheet: op.sheet, range: { ...op.range }, hasHeader: op.hasHeader, filter: null }];
      return handled([], op.sheet);
    }
    case "deleteDatabaseRange": {
      x.dbRanges = x.dbRanges.filter((d) => d.name.toLowerCase() !== op.name.toLowerCase());
      return handled([], null);
    }
    case "setViewOptions": {
      x.view[op.sheet] = { ...op.options };
      return handled([], op.sheet);
    }
    case "setCalcSettings": {
      if (op.settings.iterations && (op.settings.iterations.steps < 1 || op.settings.iterations.minChange < 0)) return err("invalid_argument", "iterations: steps ≥ 1 and minChange ≥ 0");
      x.calc = structuredClone(op.settings);
      return handled([], null);
    }
    case "setCustomSortLists": {
      x.lists = op.lists.map((l) => [...l]);
      return handled([], null);
    }
    default:
      return { kind: "pass" };
  }
}
