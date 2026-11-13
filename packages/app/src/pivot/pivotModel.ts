/**
 * DataPilot layout model: the dialog's state (fields in the Page / Column /
 * Row / Data areas plus the result options), pure reducers for moving
 * fields between areas, and the conversion to and from `PivotDto`.
 */
import type { AggFn, CellPos, DataLayout, PivotDataFieldDto, PivotDto, PivotFieldDto, PivotFieldItemsDto, PivotLayout, PivotSort, PivotSource, PivotSubtotals, RangeDto, ShowAsKind } from "@tally/bridge";

export type PivotArea = "page" | "col" | "row" | "data";

/** The 11 DataPilot functions (AOO order) with their `t()` keys. */
export const AGG_FNS: readonly { fn: AggFn; key: string; label: string }[] = [
  { fn: "sum", key: "pivot.sum", label: "Sum" },
  { fn: "count", key: "pivot.count", label: "Count" },
  { fn: "average", key: "pivot.average", label: "Average" },
  { fn: "max", key: "pivot.max", label: "Max" },
  { fn: "min", key: "pivot.min", label: "Min" },
  { fn: "product", key: "pivot.product", label: "Product" },
  { fn: "countNums", key: "pivot.countNums", label: "Count (Numbers only)" },
  { fn: "stDev", key: "pivot.stdev", label: "StDev (Sample)" },
  { fn: "stDevP", key: "pivot.stdevp", label: "StDevP (Population)" },
  { fn: "var", key: "pivot.var", label: "Var (Sample)" },
  { fn: "varP", key: "pivot.varp", label: "VarP (Population)" },
];

export const SHOW_AS: readonly { kind: ShowAsKind; label: string }[] = [
  { kind: "difference", label: "Difference from" },
  { kind: "percentOf", label: "% of" },
  { kind: "percentDifference", label: "% difference from" },
  { kind: "runningTotal", label: "Running total in" },
  { kind: "percentOfRow", label: "% of row" },
  { kind: "percentOfColumn", label: "% of column" },
  { kind: "percentOfTotal", label: "% of total" },
  { kind: "index", label: "Index" },
];

export const PIVOT_LAYOUTS: readonly { layout: PivotLayout; label: string }[] = [
  { layout: "tabular", label: "Tabular layout" },
  { layout: "outlineTop", label: "Outline layout with subtotals at the top" },
  { layout: "outlineBottom", label: "Outline layout with subtotals at the bottom" },
];

/** A row/column/page field in the layout. */
export interface LayoutField {
  sourceCol: number;
  name: string;
  subtotals: PivotSubtotals;
  showEmpty: boolean;
  sort: PivotSort;
  hiddenItems: string[];
  pageItem: string | null;
  layout: PivotLayout;
  emptyLineAfter: boolean;
  autoShow: { top: boolean; count: number; dataField: number } | null;
  /** Field grouping, carried through an edit unchanged (never edited in the layout dialog). */
  group?: PivotFieldDto["group"];
}

export interface LayoutDataField {
  sourceCol: number;
  name: string;
  function: AggFn;
  caption: string | null;
  showAs: { kind: ShowAsKind; baseField: number | null; baseItem: string | null } | null;
}

export interface PivotLayoutState {
  source: PivotSource;
  page: LayoutField[];
  col: LayoutField[];
  row: LayoutField[];
  data: LayoutDataField[];
  dataLayout: DataLayout;
  /** Output: `null` = a new sheet. */
  targetSheet: number | null;
  target: CellPos;
  rowTotals: boolean;
  colTotals: boolean;
  ignoreEmptyRows: boolean;
  identifyCategories: boolean;
  showFilterButton: boolean;
  drillDown: boolean;
}

export function defaultField(f: Pick<PivotFieldItemsDto, "sourceCol" | "name">): LayoutField {
  return { sourceCol: f.sourceCol, name: f.name, subtotals: { type: "auto" }, showEmpty: false, sort: { type: "name", ascending: true }, hiddenItems: [], pageItem: null, layout: "tabular", emptyLineAfter: false, autoShow: null };
}

/** Numbers default to Sum, everything else to Count (as AOO does). */
export function defaultDataField(f: PivotFieldItemsDto): LayoutDataField {
  return { sourceCol: f.sourceCol, name: f.name, function: f.numeric ? "sum" : "count", caption: null, showAs: null };
}

/** An empty layout for a source, placed right of the source by default. */
export function emptyLayout(source: PivotSource, sourceRange: RangeDto | null, sheet: number): PivotLayoutState {
  const target = sourceRange ? { r: sourceRange.r0, c: sourceRange.c1 + 2 } : { r: 0, c: 0 };
  return { source, page: [], col: [], row: [], data: [], dataLayout: "column", targetSheet: sheet, target, rowTotals: true, colTotals: true, ignoreEmptyRows: false, identifyCategories: false, showFilterButton: true, drillDown: true };
}

/** Fields not yet placed in any area. */
export function availableFields(st: PivotLayoutState, fields: readonly PivotFieldItemsDto[]): PivotFieldItemsDto[] {
  const used = new Set([...st.page, ...st.col, ...st.row].map((f) => f.sourceCol));
  return fields.filter((f) => !used.has(f.sourceCol));
}

/** Adds a field to an area (a source column sits in at most one axis area; data fields may repeat). */
export function addField(st: PivotLayoutState, field: PivotFieldItemsDto, area: PivotArea, at?: number): PivotLayoutState {
  if (area === "data") {
    const data = [...st.data];
    data.splice(at ?? data.length, 0, defaultDataField(field));
    return { ...st, data };
  }
  const next = removeField(st, field.sourceCol, null);
  const list = [...next[area]];
  list.splice(at ?? list.length, 0, defaultField(field));
  return { ...next, [area]: list };
}

/** Removes a field from the axis areas (or one data field by index). */
export function removeField(st: PivotLayoutState, sourceCol: number, dataIndex: number | null): PivotLayoutState {
  if (dataIndex !== null) return { ...st, data: st.data.filter((_, i) => i !== dataIndex) };
  return { ...st, page: st.page.filter((f) => f.sourceCol !== sourceCol), col: st.col.filter((f) => f.sourceCol !== sourceCol), row: st.row.filter((f) => f.sourceCol !== sourceCol) };
}

/** Moves a field between axis areas, keeping its options. */
export function moveField(st: PivotLayoutState, sourceCol: number, from: Exclude<PivotArea, "data">, to: Exclude<PivotArea, "data">, at?: number): PivotLayoutState {
  const f = st[from].find((x) => x.sourceCol === sourceCol);
  if (!f) return st;
  const without = { ...st, [from]: st[from].filter((x) => x.sourceCol !== sourceCol) } as PivotLayoutState;
  const list = [...without[to]];
  list.splice(at ?? list.length, 0, f);
  return { ...without, [to]: list };
}

export function updateField(st: PivotLayoutState, area: Exclude<PivotArea, "data">, sourceCol: number, patch: Partial<LayoutField>): PivotLayoutState {
  return { ...st, [area]: st[area].map((f) => (f.sourceCol === sourceCol ? { ...f, ...patch } : f)) };
}

export function updateDataField(st: PivotLayoutState, index: number, patch: Partial<LayoutDataField>): PivotLayoutState {
  return { ...st, data: st.data.map((f, i) => (i === index ? { ...f, ...patch } : f)) };
}

/**
 * Whether a field's layout options (outline layout, empty line after each
 * item) apply: row fields only, and not the innermost one unless the Data
 * field may follow it (AOO `ScDPLayoutDlg` `pvlaydlg.cxx:353`; the engine
 * ignores them elsewhere, `dptabsrc.cxx:690,935`).
 */
export function layoutOptionsEnabled(st: PivotLayoutState, area: PivotArea, sourceCol: number): boolean {
  if (area !== "row") return false;
  const i = st.row.findIndex((f) => f.sourceCol === sourceCol);
  return i >= 0 && (st.data.length > 1 || i + 1 < st.row.length);
}

/** The caption a data field shows (`Sum - Revenue`). */
export function dataCaption(f: LayoutDataField, fnLabel: (fn: AggFn) => string = aggLabel): string {
  return f.caption ?? `${fnLabel(f.function)} - ${f.name}`;
}

export function aggLabel(fn: AggFn): string {
  return AGG_FNS.find((a) => a.fn === fn)?.label ?? fn;
}

/** Problems that block Create (empty string = fine). */
export function layoutError(st: PivotLayoutState): string | null {
  if (st.data.length === 0) return "Add at least one data field.";
  if (st.row.length + st.col.length + st.page.length + st.data.length > 64) return "At most 64 fields.";
  return null;
}

function toField(f: LayoutField): PivotFieldDto {
  return { sourceCol: f.sourceCol, name: f.name, subtotals: f.subtotals, showEmpty: f.showEmpty, sort: f.sort, hiddenItems: [...f.hiddenItems], pageItem: f.pageItem, layout: f.layout, emptyLineAfter: f.emptyLineAfter, autoShow: f.autoShow ? { ...f.autoShow } : null, group: f.group ?? null };
}

function fromField(f: PivotFieldDto): LayoutField {
  return { sourceCol: f.sourceCol, name: f.name, subtotals: f.subtotals, showEmpty: f.showEmpty, sort: f.sort, hiddenItems: [...f.hiddenItems], pageItem: f.pageItem, layout: f.layout, emptyLineAfter: f.emptyLineAfter, autoShow: f.autoShow ? { ...f.autoShow } : null, group: f.group ?? null };
}

function toDataField(f: LayoutDataField): PivotDataFieldDto {
  return { sourceCol: f.sourceCol, function: f.function, caption: f.caption, showAs: f.showAs ? { kind: f.showAs.kind, baseField: f.showAs.baseField, baseItem: f.showAs.baseItem === null ? null : f.showAs.baseItem === "__prev" ? { type: "previous" } : f.showAs.baseItem === "__next" ? { type: "next" } : { type: "named", name: f.showAs.baseItem } } : null };
}

/** The `PivotDto` for Create / Update (`targetSheet` resolved by the caller when a new sheet was asked for). */
export function toPivotDto(st: PivotLayoutState, targetSheet: number, id = 0, name = ""): PivotDto {
  return {
    id,
    name,
    source: st.source,
    targetSheet,
    target: { ...st.target },
    output: null,
    rowFields: st.row.map(toField),
    colFields: st.col.map(toField),
    pageFields: st.page.map(toField),
    dataFields: st.data.map(toDataField),
    dataLayout: st.dataLayout,
    rowTotals: st.rowTotals,
    colTotals: st.colTotals,
    ignoreEmptyRows: st.ignoreEmptyRows,
    identifyCategories: st.identifyCategories,
    showFilterButton: st.showFilterButton,
    drillDown: st.drillDown,
  };
}

/** The layout of an existing pivot (Edit layout). */
export function fromPivotDto(p: PivotDto, fields: readonly PivotFieldItemsDto[]): PivotLayoutState {
  const nameOf = (c: number) => fields.find((f) => f.sourceCol === c)?.name ?? `Column ${c + 1}`;
  return {
    source: p.source,
    page: p.pageFields.map(fromField),
    col: p.colFields.map(fromField),
    row: p.rowFields.map(fromField),
    data: p.dataFields.map((d) => ({ sourceCol: d.sourceCol, name: nameOf(d.sourceCol), function: d.function, caption: d.caption, showAs: d.showAs ? { kind: d.showAs.kind, baseField: d.showAs.baseField, baseItem: d.showAs.baseItem === null ? null : d.showAs.baseItem.type === "previous" ? "__prev" : d.showAs.baseItem.type === "next" ? "__next" : d.showAs.baseItem.name } : null })),
    dataLayout: p.dataLayout,
    targetSheet: p.targetSheet,
    target: { ...p.target },
    rowTotals: p.rowTotals,
    colTotals: p.colTotals,
    ignoreEmptyRows: p.ignoreEmptyRows,
    identifyCategories: p.identifyCategories,
    showFilterButton: p.showFilterButton,
    drillDown: p.drillDown,
  };
}

/** Whether `cell` lies in a pivot's output area (`pivots` from the store). */
export function pivotAtCell(pivots: readonly PivotDto[], sheet: number, cell: CellPos): PivotDto | null {
  for (const p of pivots) {
    const o = p.output;
    if (p.targetSheet === sheet && o && cell.r >= o.r0 && cell.r <= o.r1 && cell.c >= o.c0 && cell.c <= o.c1) return p;
  }
  return null;
}

/**
 * The header band of a pivot output (page-field rows, the column-item row
 * and the row-field column), used for the distinct header fill.
 */
export function pivotHeaderRanges(p: PivotDto): RangeDto[] {
  const o = p.output;
  if (!o) return [];
  const out: RangeDto[] = [];
  // The filter button, the page fields and a blank row (AOO `ScDPOutput::CalcSizes`).
  const pageRows = p.showFilterButton || p.pageFields.length > 0 ? p.pageFields.length + 1 + (p.showFilterButton ? 1 : 0) : 0;
  const top = o.r0 + pageRows;
  if (top <= o.r1) out.push({ r0: top, c0: o.c0, r1: Math.min(o.r1, top + (p.colFields.length > 0 ? 1 : 0)), c1: o.c1 });
  const left = { r0: top, c0: o.c0, r1: o.r1, c1: Math.min(o.c1, o.c0 + Math.max(0, p.rowFields.length - 1)) };
  if (left.r0 <= left.r1) out.push(left);
  return out;
}
