/**
 * `WebMockClient`: the browser demo/e2e engine. It is @tally/bridge's
 * `MockClient` (formulas, styles, undo) plus emulations of the ops the mock
 * rejects as `unsupported`, so every UI flow can run in a browser:
 *
 * - insert/delete rows and columns: rewritten into clear + `setRangeInputs`
 *   + `applyStyle` ops that shift the cells (formulas are NOT re-anchored,
 *   sizes do not move);
 * - v0.2 ops (names, array formulas, move, merge, freeze, sort, AutoFilter,
 *   hide rows/columns, replace all, paste special, fill, borders): kept in a
 *   small side model (merges, frozen panes, hidden/filtered rows, names,
 *   array blocks) reported through `sheetLayout`, `filterInfo`, `names`,
 *   `find` and `cellEditInfo`, and/or rewritten into cell ops. Undo and redo
 *   restore the side model together with the mock's own snapshots.
 *
 * Every op passed to `apply` is recorded in {@link WebMockClient.recorded}
 * (an e2e hook: the mock does not resolve defined names in formulas, so
 * tests assert on the ops instead).
 *
 * Demo only: the real engine implements all of this natively.
 */
import {
  MockClient,
  type ApplyResult,
  type AutoFilterInfo,
  type BordersDto,
  type BordersPatch,
  type CellEditInfo,
  type CellView,
  type ClipboardOut,
  type ColumnFilter,
  type DimRunPx,
  type DocId,
  type FilterCondition,
  type FindHit,
  type FindQuery,
  type FindResult,
  type NameInfo,
  type Op,
  type OpError,
  type RangeDto,
  type SelectionDto,
  type SheetLayout,
  type StylePatch,
  type Transaction,
} from "@tally/bridge";
import { emulateFill } from "./mockFill";
import { applyCfPatch, evaluateFormat, type CfDecoration } from "./mockCf";
import { applyV3Op, cellKey, DEFAULT_VALIDATION_MESSAGE, emptyV3, MOCK_LOCALES, passes, protectionRefusal, validationAt, type V3Ext } from "./mockV3";
import { applyW56Op, autocompleteOf, cellStylesOf, decorateSheets, emptyW56, navigatorOf, pivotFieldItemsOf, pivotHitAt, stdFilterHidden, type W56Ctx, type W56Ext } from "./mockW56";
import { applyW5Op, BLANK_PNG, chartDataFor, chartsTouched, dependentsOf, emptyW5, pageBreaksFor, precedentsOf, printLayoutFor, renderPagePng, suggestChartFor, type W5Ext } from "./mockW5";
import { parseAbsoluteRef } from "../dataOps";
import { previewCall, WIZARD_FUNCTIONS, type MockValue } from "./mockWizard";
import type {
  FunctionInfo,
  AutocompleteDto,
  CalcSettingsDto,
  CellStyleDto,
  DatabaseRangeDto,
  DocumentStatsDto,
  NavigatorDto,
  PivotDto,
  PivotFieldItemsDto,
  PivotHit,
  PivotSource,
  ScenarioDto,
  WorkbookInfo,
  CellBlock,
  ChartDataDto,
  ChartDto,
  ChartKind,
  ChartSvgDto,
  CreatedId,
  ImageDto,
  PageBreaksDto,
  PrintJobInfo,
  PrintLayoutDto,
  PrintOptionsDto,
  TraceDto,
  CfStyleDto,
  ConditionalFormatDto,
  DocPropertiesDto,
  GoalSeekRequest,
  GoalSeekResult,
  LocaleInfo,
  NoteDto,
  PageSetupDto,
  ProtectionInfo,
  SheetRanges,
  StyleDto,
  ValidationDto,
  ValidationInfo,
} from "@tally/bridge";
import { defaultPageSetup } from "../v03";
import { INVALID_SCAN_CAP, invalidCells as invalidCellsOf } from "../detective/detective";
import type { CellPos, CellsOptions, ViewOptionsDto } from "@tally/bridge";

/** Synthesized conditional-format style ids start here (as in the engine). */
export const CF_STYLE_BASE = 0x4000_0000;

type Structural = Extract<Op, { type: "insertRows" | "deleteRows" | "insertCols" | "deleteCols" }>;

function isStructural(op: Op): op is Structural {
  return op.type === "insertRows" || op.type === "deleteRows" || op.type === "insertCols" || op.type === "deleteCols";
}

/** Per-sheet state the mock does not model. */
interface SheetExt {
  merges: RangeDto[];
  frozen: { r: number; c: number } | null;
  hiddenRows: number[];
  hiddenCols: number[];
  filter: { range: RangeDto; columns: Record<number, ColumnFilter> } | null;
  filteredRows: number[];
  arrays: { range: RangeDto; input: string }[];
}

interface DocExt {
  sheets: Record<number, SheetExt>;
  names: NameInfo[];
  /** v0.3 side model (conditional formats, validations, notes, links, protection, page setup…). */
  v3: V3Ext;
  /** W5-5 side model (charts, pictures, split). */
  w5: W5Ext;
  /** W5-6 side model (pivots, outline, filters, scenarios, styles, settings). */
  w56: W56Ext;
}

const emptySheet = (): SheetExt => ({ merges: [], frozen: null, hiddenRows: [], hiddenCols: [], filter: null, filteredRows: [], arrays: [] });

const EMPTY_PATCH: StylePatch = {
  bold: null,
  italic: null,
  underline: null,
  strike: null,
  fontFamily: null,
  fontSizePt: null,
  color: null,
  fill: null,
  hAlign: null,
  vAlign: null,
  wrap: null,
  numberFormat: null,
};

const inside = (r: RangeDto, row: number, col: number): boolean => row >= r.r0 && row <= r.r1 && col >= r.c0 && col <= r.c1;
const overlaps = (a: RangeDto, b: RangeDto): boolean => a.r0 <= b.r1 && b.r0 <= a.r1 && a.c0 <= b.c1 && b.c0 <= a.c1;
const contains = (outer: RangeDto, inner: RangeDto): boolean => outer.r0 <= inner.r0 && outer.c0 <= inner.c0 && outer.r1 >= inner.r1 && outer.c1 >= inner.c1;

/** `range` minus its top-left cell, as up to two rectangles. */
function withoutAnchor(range: RangeDto): RangeDto[] {
  const out: RangeDto[] = [];
  if (range.c1 > range.c0) out.push({ r0: range.r0, c0: range.c0 + 1, r1: range.r0, c1: range.c1 });
  if (range.r1 > range.r0) out.push({ r0: range.r0 + 1, c0: range.c0, r1: range.r1, c1: range.c1 });
  return out;
}

/** Sorts numbers before text (case-insensitive), empties last. */
function compareValues(a: CellView | undefined, b: CellView | undefined): number {
  const rank = (v: CellView | undefined): number => (!v || v.kind === "empty" || v.text === "" ? 3 : v.kind === "number" ? 0 : v.kind === "error" ? 2 : 1);
  const ra = rank(a);
  const rb = rank(b);
  if (ra !== rb) return ra - rb;
  if (ra === 0) return (a?.num ?? Number(a?.text)) - (b?.num ?? Number(b?.text));
  if (ra === 3) return 0;
  return (a?.text ?? "").localeCompare(b?.text ?? "", undefined, { sensitivity: "base" });
}

function numberOf(text: string): number | null {
  const t = text.replace(/[$,\s]/g, "");
  if (t === "" || !/^-?\d*\.?\d+(e-?\d+)?%?$/i.test(t)) return null;
  const n = Number(t.replace(/%$/, ""));
  return Number.isFinite(n) ? (t.endsWith("%") ? n / 100 : n) : null;
}

function passesCondition(text: string, cond: FilterCondition): boolean {
  const v = cond.value;
  const a = numberOf(text);
  const b = numberOf(v);
  const cmp = a !== null && b !== null ? a - b : text.toLowerCase().localeCompare(v.toLowerCase());
  const lt = text.toLowerCase();
  const lv = v.toLowerCase();
  switch (cond.op) {
    case "eq":
      return cmp === 0;
    case "ne":
      return cmp !== 0;
    case "gt":
      return cmp > 0;
    case "ge":
      return cmp >= 0;
    case "lt":
      return cmp < 0;
    case "le":
      return cmp <= 0;
    case "contains":
      return lt.includes(lv);
    case "beginsWith":
      return lt.startsWith(lv);
    case "endsWith":
      return lt.endsWith(lv);
    case "empty":
      return text === "";
    case "notEmpty":
      return text !== "";
  }
}

/** A matcher for a find query (literal, wildcards or regex; case; whole cell). */
function matcher(q: FindQuery): ((s: string) => boolean) | null {
  const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  let src: string;
  if (q.mode === "regex") src = q.text;
  else if (q.mode === "wildcards") {
    src = "";
    for (let i = 0; i < q.text.length; i++) {
      const ch = q.text[i] ?? "";
      if (ch === "~" && i + 1 < q.text.length) src += esc(q.text[++i] ?? "");
      else if (ch === "*") src += ".*";
      else if (ch === "?") src += ".";
      else src += esc(ch);
    }
  } else src = esc(q.text);
  if (q.wholeCell) src = `^(?:${src})$`;
  try {
    const re = new RegExp(src, q.matchCase ? "" : "i");
    return (s) => re.test(s);
  } catch {
    return null;
  }
}

export class WebMockClient extends MockClient {
  /** Every op passed to `apply` (e2e hook). */
  readonly recorded: Op[] = [];
  private readonly ext = new Map<DocId, DocExt>();
  private readonly extUndo = new Map<DocId, string[]>();
  private readonly extRedo = new Map<DocId, string[]>();
  private readonly clips = new Map<string, { inputs: string[][]; texts: string[][] }>();

  // -- side model -----------------------------------------------------------------

  private docExt(doc: DocId): DocExt {
    let d = this.ext.get(doc);
    if (!d) {
      d = { sheets: {}, names: [], v3: emptyV3(), w5: emptyW5(), w56: emptyW56() };
      this.ext.set(doc, d);
    }
    return d;
  }

  private sheetExt(doc: DocId, sheet: number): SheetExt {
    const d = this.docExt(doc);
    let s = d.sheets[sheet];
    if (!s) {
      s = emptySheet();
      d.sheets[sheet] = s;
    }
    return s;
  }

  // -- Function Wizard (mockWizard.ts) ----------------------------------------------

  override listFunctions(): Promise<FunctionInfo[]> {
    return Promise.resolve(WIZARD_FUNCTIONS.map((f) => structuredClone(f)));
  }

  override async previewFormula(doc: DocId, sheet: number, r: number, c: number, text: string): Promise<string | null> {
    const read = async (range: RangeDto): Promise<MockValue[][]> => {
      const [block] = await this.cells(doc, sheet, [range]);
      const rows: MockValue[][] = Array.from({ length: range.r1 - range.r0 + 1 }, () => Array.from({ length: range.c1 - range.c0 + 1 }, () => null));
      for (const v of block?.cells ?? []) {
        const row = rows[v.r - range.r0];
        if (!row) continue;
        row[v.c - range.c0] = v.kind === "number" ? (v.num ?? Number(v.text.replace(/[^0-9.+-eE]/g, ""))) : v.kind === "text" ? v.text : null;
      }
      return rows;
    };
    const emulated = await previewCall(text, read);
    return emulated ?? super.previewFormula(doc, sheet, r, c, text);
  }

  // -- queries ---------------------------------------------------------------------

  override async sheetLayout(doc: DocId, sheet: number): Promise<SheetLayout> {
    const base = await super.sheetLayout(doc, sheet);
    const s = this.ext.get(doc)?.sheets[sheet];
    if (!s) return base;
    const hidden = new Set(s.hiddenRows);
    const filtered = new Set(s.filteredRows);
    const o = this.ext.get(doc)?.w56.outline[sheet];
    const levels = (gs: readonly { level: number }[]) => gs.reduce((m, g) => Math.max(m, g.level), 0);
    return {
      ...base,
      merges: s.merges.map((m) => ({ ...m })),
      frozen: s.frozen ? { ...s.frozen } : null,
      rowRuns: overlayRuns(base.rowRuns, base.defaultRowHeightPx, hidden, filtered),
      colRuns: overlayRuns(base.colRuns, base.defaultColWidthPx, new Set(s.hiddenCols), new Set()),
      // Like the engine: the outline groups ride along with the layout.
      outline: o && (o.rows.length > 0 || o.cols.length > 0) ? { rows: o.rows.map((g) => ({ ...g })), cols: o.cols.map((g) => ({ ...g })), rowLevels: levels(o.rows), colLevels: levels(o.cols) } : null,
    };
  }

  // -- v0.6 queries the integration pass wired (view options, Mark Invalid Data, sort lists, picture bytes) --

  override async viewOptions(doc: DocId, sheet: number): Promise<ViewOptionsDto> {
    const base = await super.viewOptions(doc, sheet);
    const x = this.docExt(doc);
    const v = x.w56.view[sheet];
    const split = x.w5.split[sheet] ?? null;
    return { ...base, ...(v ? { ...v } : {}), split: split ? { ...split } : null };
  }

  override async customSortLists(doc: DocId): Promise<string[][]> {
    await super.customSortLists(doc);
    return this.docExt(doc).w56.lists.map((l) => [...l]);
  }

  override async imageData(doc: DocId, sheet: number, id: number): Promise<string> {
    const im = (this.docExt(doc).w5.images[sheet] ?? []).find((i) => i.id === id);
    if (!im) throw { code: "not_found", message: `no picture ${id}` };
    this.recordedCalls.push({ type: "imageData", sheet, id });
    return im.data;
  }

  /** Mark Invalid Data: the client-side approximation of the engine's `invalid_cells` (literal bounds, list values). */
  override async invalidCells(doc: DocId, sheet: number, range: RangeDto | null): Promise<CellPos[]> {
    const validations = await this.validations(doc, sheet);
    const used = range ?? (await super.sheetLayout(doc, sheet)).used;
    if (validations.length === 0 || !used) return [];
    let scanned = 0;
    const ranges: RangeDto[] = [];
    for (const v of validations) {
      for (const rg of v.ranges) {
        const clipped = { r0: Math.max(rg.r0, used.r0), c0: Math.max(rg.c0, used.c0), r1: Math.min(rg.r1, used.r1), c1: Math.min(rg.c1, used.c1) };
        if (clipped.r0 > clipped.r1 || clipped.c0 > clipped.c1) continue;
        const n = (clipped.r1 - clipped.r0 + 1) * (clipped.c1 - clipped.c0 + 1);
        if (scanned + n > INVALID_SCAN_CAP) {
          clipped.r1 = clipped.r0 + Math.max(0, Math.floor((INVALID_SCAN_CAP - scanned) / (clipped.c1 - clipped.c0 + 1)) - 1);
          if (clipped.r1 < clipped.r0) break;
        }
        scanned += (clipped.r1 - clipped.r0 + 1) * (clipped.c1 - clipped.c0 + 1);
        ranges.push(clipped);
      }
    }
    const cells: CellView[] = [];
    if (ranges.length > 0) for (const b of await this.cells(doc, sheet, ranges.slice(0, 8))) cells.push(...b.cells);
    const present = new Set(cells.map((c) => `${c.r},${c.c}`));
    for (const rg of ranges) {
      for (let r = rg.r0; r <= rg.r1; r++) for (let c = rg.c0; c <= rg.c1; c++) if (!present.has(`${r},${c}`)) cells.push({ r, c, text: "", kind: "empty", num: null, style: 0, color: null, flags: 0 });
    }
    return invalidCellsOf(validations, cells).slice(0, 10_000);
  }

  override async cellsWith(doc: DocId, sheet: number, ranges: RangeDto[], opts: CellsOptions): Promise<CellBlock[]> {
    this.recordedCalls.push({ type: "cellsWith", opts: { ...opts } });
    return super.cellsWith(doc, sheet, ranges, opts);
  }

  override async cellEditInfo(doc: DocId, sheet: number, r: number, c: number): Promise<CellEditInfo> {
    const block = this.ext.get(doc)?.sheets[sheet]?.arrays.find((a) => inside(a.range, r, c));
    if (block) return { input: `{${block.input}}`, hint: null, array: true, arrayRange: { ...block.range } };
    return super.cellEditInfo(doc, sheet, r, c);
  }

  override async names(doc: DocId): Promise<NameInfo[]> {
    await super.names(doc);
    return this.docExt(doc).names.map((n) => ({ ...n }));
  }

  override async filterInfo(doc: DocId, sheet: number): Promise<AutoFilterInfo> {
    await super.filterInfo(doc, sheet);
    const f = this.ext.get(doc)?.sheets[sheet]?.filter;
    if (!f) return { range: null, columns: [] };
    const body = { ...f.range, r0: f.range.r0 + 1 };
    const [block] = await this.cells(doc, sheet, [body]);
    const columns = [];
    for (let c = f.range.c0; c <= f.range.c1; c++) {
      const counts = new Map<string, number>();
      for (let r = body.r0; r <= body.r1; r++) {
        const t = block?.cells.find((v) => v.r === r && v.c === c)?.text ?? "";
        counts.set(t, (counts.get(t) ?? 0) + 1);
      }
      const unique = [...counts.keys()].sort((a, b) => {
        if (a === "" || b === "") return a === "" ? (b === "" ? 0 : 1) : -1;
        const na = numberOf(a);
        const nb = numberOf(b);
        if (na !== null && nb !== null) return na - nb;
        if (na !== null || nb !== null) return na !== null ? -1 : 1;
        return a.localeCompare(b, undefined, { sensitivity: "base" });
      });
      columns.push({ col: c, unique, counts: unique.map((u) => counts.get(u) ?? 0), active: f.columns[c] ?? null });
    }
    return { range: { ...f.range }, columns };
  }

  override async detectHeader(doc: DocId, sheet: number, range: RangeDto): Promise<boolean> {
    await super.detectHeader(doc, sheet, range);
    if (range.r1 <= range.r0) return false;
    const [block] = await this.cells(doc, sheet, [range]);
    const cells = block?.cells ?? [];
    const head = cells.filter((v) => v.r === range.r0);
    if (head.length === 0 || head.some((v) => v.kind !== "text")) return false;
    // Labels: text over columns that hold numbers, or labels that do not repeat below.
    return head.some((h) => cells.some((v) => v.c === h.c && v.r > range.r0 && (v.kind === "number" || v.text !== h.text)));
  }

  override async find(doc: DocId, query: FindQuery): Promise<FindResult> {
    await super.find(doc, query);
    const match = matcher(query);
    if (!match || query.text === "") return { hits: [], truncated: false };
    const order = this.sheetOrder.get(doc) ?? [this.firstSheet(doc)];
    const sheets = query.sheet === null ? order : [query.sheet];
    const hits: FindHit[] = [];
    for (const sheet of sheets) {
      const layout = await super.sheetLayout(doc, sheet);
      if (!layout.used) continue;
      const [block] = await this.cells(doc, sheet, [layout.used]);
      for (const v of block?.cells ?? []) {
        if (v.text === "" && v.kind === "empty") continue;
        const text = query.inFormulas ? (await super.cellEditInfo(doc, sheet, v.r, v.c)).input : v.text;
        if (text !== "" && match(text)) hits.push({ sheet, r: v.r, c: v.c });
      }
    }
    hits.sort((a, b) => sheets.indexOf(a.sheet) - sheets.indexOf(b.sheet) || a.r - b.r || a.c - b.c);
    if (query.from) {
      const from = query.from;
      const anchorSheet = query.sheet ?? sheets[0];
      const after = (h: FindHit) =>
        sheets.indexOf(h.sheet) > sheets.indexOf(anchorSheet ?? h.sheet) ||
        (h.sheet === anchorSheet && (h.r > from.r || (h.r === from.r && h.c > from.c)));
      const i = hits.findIndex(after);
      if (i > 0) hits.push(...hits.splice(0, i));
    }
    return { hits: hits.slice(0, 10_000), truncated: hits.length > 10_000 };
  }

  /** Sheet order per document, learned from results (`info`) and `newWorkbook`. */
  private readonly sheetOrder = new Map<DocId, number[]>();

  override async newWorkbook(template?: string | null) {
    const info = await super.newWorkbook(template);
    this.sheetOrder.set(info.doc, info.sheets.map((s) => s.id));
    this.sheetNames.set(info.doc, new Map(info.sheets.map((s) => [s.name.toLowerCase(), s.id])));
    return info;
  }

  /** Sheet ids by lower-case name (list validation sources). */
  private readonly sheetNames = new Map<DocId, Map<string, number>>();

  // -- v0.3 queries (from the side model) ----------------------------------------

  /** Every call of `setLocale`, `openUrl` and `goalSeek` (e2e hook). */
  readonly recordedCalls: { type: string; [k: string]: unknown }[] = [];

  override async conditionalFormats(doc: DocId, sheet: number): Promise<ConditionalFormatDto[]> {
    await super.conditionalFormats(doc, sheet);
    return structuredClone(this.docExt(doc).v3.cfs[sheet] ?? []);
  }

  override async validations(doc: DocId, sheet: number): Promise<ValidationDto[]> {
    await super.validations(doc, sheet);
    return structuredClone(this.docExt(doc).v3.validations[sheet] ?? []);
  }

  /** List items of a validation: inline, or read from its source range. */
  private async listItemsOf(doc: DocId, v: ValidationDto): Promise<string[] | null> {
    if (v.kind.type !== "list") return null;
    if (v.kind.source.type === "values") return v.kind.source.items;
    const ref = parseAbsoluteRef(v.kind.source.reference.replace(/^=/, ""));
    if (!ref) return null;
    const sheet = this.sheetNames.get(doc)?.get(ref.sheet.toLowerCase());
    if (sheet === undefined) return null;
    const [block] = await super.cells(doc, sheet, [ref.range]);
    return (block?.cells ?? []).map((c) => c.text).filter((t) => t !== "").slice(0, 10_000);
  }

  override async validationAt(doc: DocId, sheet: number, r: number, c: number): Promise<ValidationInfo | null> {
    await super.validationAt(doc, sheet, r, c);
    const v = validationAt(this.docExt(doc).v3, sheet, r, c);
    if (!v) return null;
    return { id: v.id, listItems: (await this.listItemsOf(doc, v)) ?? [], showDropdown: v.showDropdown && v.kind.type === "list", inputHelp: v.inputHelp, error: v.error, kind: v.kind };
  }

  override async note(doc: DocId, sheet: number, r: number, c: number): Promise<NoteDto | null> {
    await super.note(doc, sheet, r, c);
    return this.docExt(doc).v3.notes[sheet]?.[cellKey(r, c)] ?? null;
  }

  override async notes(doc: DocId, sheet: number): Promise<NoteDto[]> {
    await super.notes(doc, sheet);
    return Object.values(this.docExt(doc).v3.notes[sheet] ?? {}).sort((a, b) => a.r - b.r || a.c - b.c);
  }

  override async protectionInfo(doc: DocId, sheet: number): Promise<ProtectionInfo> {
    const base = await super.protectionInfo(doc, sheet);
    const x = this.docExt(doc).v3;
    const p = x.protection[sheet];
    return {
      sheetProtected: !!p,
      sheetHasPassword: !!p?.password,
      options: p?.options ?? base.options,
      structureProtected: !!x.structure,
      structureHasPassword: !!x.structure?.password,
    };
  }

  override async pageSetup(doc: DocId, sheet: number): Promise<PageSetupDto> {
    await super.pageSetup(doc, sheet);
    return structuredClone(this.docExt(doc).v3.pages[sheet] ?? defaultPageSetup());
  }

  override async documentProperties(doc: DocId): Promise<DocPropertiesDto> {
    await super.documentProperties(doc);
    return { ...this.docExt(doc).v3.props };
  }

  /**
   * Goal seek, emulated: assumes the formula is proportional to the
   * variable (true for Total = Qty × Price) and scales the variable.
   */
  override async goalSeek(doc: DocId, req: GoalSeekRequest): Promise<GoalSeekResult> {
    this.recordedCalls.push({ type: "goalSeek", req: structuredClone(req) });
    const read = async (sheet: number, p: { r: number; c: number }) => {
      const [b] = await super.cells(doc, sheet, [{ r0: p.r, c0: p.c, r1: p.r, c1: p.c }]);
      const v = b?.cells[0];
      return v && v.kind === "number" ? (v.num ?? Number(v.text.replace(/[^0-9.eE+-]/g, ""))) : null;
    };
    const f0 = await read(req.formulaSheet, req.formulaCell);
    const v0 = await read(req.variableSheet, req.variableCell);
    if (f0 === null || v0 === null || f0 === 0 || v0 === 0) return { found: false, value: v0 ?? 0, result: f0 ?? 0, iterations: 100 };
    const value = (v0 * req.target) / f0;
    return { found: true, value, result: req.target, iterations: 3 };
  }

  override async listLocales(): Promise<LocaleInfo[]> {
    return MOCK_LOCALES.map((l) => ({ ...l }));
  }

  override async setLocale(doc: DocId, locale: string): Promise<ApplyResult> {
    this.recordedCalls.push({ type: "setLocale", locale });
    if (!MOCK_LOCALES.some((l) => l.tag === locale)) throw { code: "invalid_argument", message: `unknown locale "${locale}"` };
    this.docExt(doc).v3.locale = locale;
    // The mock does not reformat; report the used areas as changed so the grid refetches.
    const res = await super.recalcAll(doc);
    const changed: SheetRanges[] = [];
    for (const sheet of this.sheetOrder.get(doc) ?? []) {
      const used = (await super.sheetLayout(doc, sheet)).used;
      if (used) changed.push({ sheet, ranges: [used] });
    }
    return { ...res, changed };
  }

  /** Records the URL instead of opening it (browser demo / e2e). */
  async openUrl(url: string): Promise<void> {
    this.recordedCalls.push({ type: "openUrl", url });
  }

  async userName(): Promise<string | null> {
    return "Demo User";
  }

  // -- W5-6 queries and shell stand-ins (mockW56.ts) ---------------------------------

  private w56Ctx(doc: DocId): W56Ctx {
    const inputs = async (sheet: number, range: RangeDto): Promise<string[][]> => (await this.snapshot(doc, sheet, range)).inputs;
    return {
      cells: (sheet, range) => this.cellsOf(doc, sheet, range),
      inputs,
      used: async (sheet) => (await super.sheetLayout(doc, sheet)).used,
      sheetName: (sheet) => [...(this.sheetNames.get(doc) ?? [])].find(([, id]) => id === sheet)?.[0]?.replace(/^./, (ch) => ch.toUpperCase()) ?? `Sheet${sheet}`,
      sheets: () => (this.sheetOrder.get(doc) ?? []).map((id) => ({ id, name: [...(this.sheetNames.get(doc) ?? [])].find(([, x]) => x === id)?.[0] ?? `sheet${id}`, tabColor: null, hidden: false })),
      notes: (sheet) => Object.keys(this.docExt(doc).v3.notes[sheet] ?? {}).map((k) => ({ r: Number(k.split(",")[0]), c: Number(k.split(",")[1]) })),
      setHiddenRows: (sheet, rows, hidden) => {
        const s = this.sheetExt(doc, sheet);
        const set = new Set(s.hiddenRows);
        for (const r of rows) hidden ? set.add(r) : set.delete(r);
        s.hiddenRows = [...set].sort((a, b) => a - b);
      },
      setHiddenCols: (sheet, cols, hidden) => {
        const s = this.sheetExt(doc, sheet);
        const set = new Set(s.hiddenCols);
        for (const c of cols) hidden ? set.add(c) : set.delete(c);
        s.hiddenCols = [...set].sort((a, b) => a - b);
      },
    };
  }

  override async pivots(doc: DocId): Promise<PivotDto[]> {
    await super.pivots(doc);
    return structuredClone(this.docExt(doc).w56.pivots);
  }

  override async pivotFieldItems(doc: DocId, source: PivotSource): Promise<PivotFieldItemsDto[]> {
    return pivotFieldItemsOf(source, this.w56Ctx(doc), this.docExt(doc).w56.dbRanges);
  }

  override async pivotAt(doc: DocId, sheet: number, r: number, c: number): Promise<PivotHit | null> {
    await super.pivotAt(doc, sheet, r, c);
    return pivotHitAt(this.docExt(doc).w56, sheet, r, c);
  }

  override async autocomplete(doc: DocId, sheet: number, r: number, c: number, prefix: string): Promise<AutocompleteDto> {
    await super.autocomplete(doc, sheet, r, c, prefix);
    const cells = await this.cellsOf(doc, sheet, { r0: Math.max(0, r - 2000), c0: c, r1: r + 2000, c1: c });
    return autocompleteOf(cells, r, c, prefix);
  }

  override async cellStyles(doc: DocId): Promise<CellStyleDto[]> {
    await super.cellStyles(doc);
    return structuredClone(cellStylesOf(this.docExt(doc).w56));
  }

  override async scenarios(doc: DocId, sheet: number): Promise<ScenarioDto[]> {
    await super.scenarios(doc, sheet);
    return (this.docExt(doc).w56.scenarios[sheet] ?? []).map(({ values: _values, ...s }) => structuredClone(s));
  }

  override async databaseRanges(doc: DocId): Promise<DatabaseRangeDto[]> {
    await super.databaseRanges(doc);
    return structuredClone(this.docExt(doc).w56.dbRanges);
  }

  override async navigator(doc: DocId): Promise<NavigatorDto> {
    await super.navigator(doc);
    const x = this.docExt(doc);
    const charts = Object.entries(x.w5.charts).flatMap(([sheet, list]) => list.map((c) => ({ sheet: Number(sheet), id: c.id, title: c.title ?? c.name, r: c.anchor.r, c: c.anchor.c })));
    return navigatorOf(x.w56, this.w56Ctx(doc), x.names, charts);
  }

  override async documentStatistics(doc: DocId): Promise<DocumentStatsDto> {
    const base = await super.documentStatistics(doc);
    const x = this.docExt(doc);
    return { ...base, names: x.names.length, charts: Object.values(x.w5.charts).reduce((n, l) => n + l.length, 0) };
  }

  override async calcSettings(doc: DocId): Promise<CalcSettingsDto> {
    await super.calcSettings(doc);
    return structuredClone(this.docExt(doc).w56.calc);
  }

  /** Shell stand-ins (`prefs.rs`, `autosave.rs`, `templates.rs`, `l10n.rs` in the desktop app). */
  async autosave(doc: DocId): Promise<void> {
    this.recordedCalls.push({ type: "autosave", doc });
  }

  async listRecovery(): Promise<{ path: string; title: string; savedAt: string }[]> {
    return [];
  }

  async saveAsTemplate(doc: DocId, name: string): Promise<void> {
    this.recordedCalls.push({ type: "saveAsTemplate", doc, name });
    this.userTemplates.push({ id: `user:${name}`, title: name, description: "User template (browser demo)" });
  }

  private readonly userTemplates: { id: string; title: string; description: string }[] = [];

  async listUserTemplates(): Promise<{ id: string; title: string; description: string }[]> {
    return [...this.userTemplates];
  }

  async newFromUserTemplate(id: string): Promise<WorkbookInfo> {
    this.recordedCalls.push({ type: "newFromUserTemplate", id });
    return this.newWorkbook(null);
  }

  async setUiLanguage(lang: string): Promise<void> {
    this.recordedCalls.push({ type: "setUiLanguage", lang });
  }

  // -- cells and styles with v0.3 decorations --------------------------------------

  /** Synthesized cf styles: id → (base style id, patch). */
  private readonly cfStyles: { base: number; patch: CfStyleDto }[] = [];
  private readonly cfStyleIndex = new Map<string, number>();
  private cfCache: { key: string; deco: Map<string, CfDecoration> } | null = null;

  private cfStyleId(base: number, patch: CfStyleDto): number {
    const k = `${base}|${JSON.stringify(patch)}`;
    let i = this.cfStyleIndex.get(k);
    if (i === undefined) {
      i = this.cfStyles.length;
      this.cfStyles.push({ base, patch });
      this.cfStyleIndex.set(k, i);
    }
    return CF_STYLE_BASE + i;
  }

  private async cfDecorations(doc: DocId, sheet: number, version: number, cfs: ConditionalFormatDto[]): Promise<Map<string, CfDecoration>> {
    const key = `${doc}:${sheet}:${version}:${JSON.stringify(cfs)}`;
    if (this.cfCache?.key === key) return this.cfCache.deco;
    const used = (await super.sheetLayout(doc, sheet)).used;
    const deco = new Map<string, CfDecoration>();
    if (used) {
      for (const cf of cfs) {
        const clipped = cf.ranges
          .map((rg) => ({ r0: Math.max(rg.r0, used.r0), c0: Math.max(rg.c0, used.c0), r1: Math.min(rg.r1, used.r1), c1: Math.min(rg.c1, used.c1) }))
          .filter((rg) => rg.r0 <= rg.r1 && rg.c0 <= rg.c1);
        if (clipped.length === 0) continue;
        const blocks = await super.cells(doc, sheet, clipped);
        evaluateFormat(cf, blocks.flatMap((b) => b.cells), deco);
      }
    }
    this.cfCache = { key, deco };
    return deco;
  }

  override async cells(doc: DocId, sheet: number, ranges: RangeDto[]): Promise<CellBlock[]> {
    const blocks = await super.cells(doc, sheet, ranges);
    const x = this.ext.get(doc)?.v3;
    if (!x) return blocks;
    const cfs = x.cfs[sheet] ?? [];
    const vals = (x.validations[sheet] ?? []).filter((v) => v.kind.type === "list" && v.showDropdown);
    const notes = x.notes[sheet] ?? {};
    const links = x.links[sheet] ?? {};
    if (cfs.length === 0 && vals.length === 0 && Object.keys(notes).length === 0 && Object.keys(links).length === 0) return blocks;
    const deco = cfs.length > 0 ? await this.cfDecorations(doc, sheet, blocks[0]?.version ?? 0, cfs) : new Map<string, CfDecoration>();
    return blocks.map((block) => {
      const byKey = new Map(block.cells.map((v) => [cellKey(v.r, v.c), { ...v }]));
      const at = (r: number, c: number): CellView => {
        const k = cellKey(r, c);
        let v = byKey.get(k);
        if (!v) {
          v = { r, c, text: "", kind: "empty", num: null, style: 0, color: null, flags: 0 };
          byKey.set(k, v);
        }
        return v;
      };
      const inBlock = (r: number, c: number) => r >= block.range.r0 && r <= block.range.r1 && c >= block.range.c0 && c <= block.range.c1;
      for (const k of Object.keys(notes)) {
        const [r, c] = k.split(",").map(Number) as [number, number];
        if (inBlock(r, c)) at(r, c).flags |= 8;
      }
      for (const [k, url] of Object.entries(links)) {
        const [r, c] = k.split(",").map(Number) as [number, number];
        if (!inBlock(r, c)) continue;
        const v = at(r, c);
        v.flags |= 32;
        v.link = url;
      }
      for (const v of vals) {
        for (const rg of v.ranges) {
          const r0 = Math.max(rg.r0, block.range.r0);
          const r1 = Math.min(rg.r1, block.range.r1);
          const c0 = Math.max(rg.c0, block.range.c0);
          const c1 = Math.min(rg.c1, block.range.c1);
          for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) at(r, c).flags |= 16;
        }
      }
      for (const [k, d] of deco) {
        const [r, c] = k.split(",").map(Number) as [number, number];
        if (!inBlock(r, c)) continue;
        const v = at(r, c);
        if (d.patch) v.cfStyle = this.cfStyleId(v.style, d.patch);
        if (d.bar) v.cfBar = d.bar;
        if (d.icon) v.cfIcon = d.icon;
        if (d.hideText) v.text = "";
      }
      const cells = [...byKey.values()].sort((a, b) => a.r - b.r || a.c - b.c);
      return { ...block, cells };
    });
  }

  override async styles(doc: DocId, ids: number[]): Promise<StyleDto[]> {
    const plain = ids.filter((id) => id < CF_STYLE_BASE);
    const synth = ids.filter((id) => id >= CF_STYLE_BASE);
    const bases = [...new Set(synth.map((id) => this.cfStyles[id - CF_STYLE_BASE]?.base ?? 0))];
    const all = await super.styles(doc, [...new Set([...plain, ...bases, 0])]);
    const byId = new Map(all.map((s) => [s.id, s]));
    const out = plain.flatMap((id) => (byId.has(id) ? [byId.get(id)!] : []));
    for (const id of synth) {
      const entry = this.cfStyles[id - CF_STYLE_BASE];
      const base = byId.get(entry?.base ?? 0) ?? byId.get(0);
      if (entry && base) out.push({ ...applyCfPatch(base, entry.patch), id });
    }
    return out;
  }

  override async copy(doc: DocId, sel: SelectionDto): Promise<ClipboardOut> {
    const out = await super.copy(doc, sel);
    const range = sel.ranges[0];
    if (range) this.clips.set(out.token, await this.snapshot(doc, sel.sheet, range));
    return out;
  }

  /** Inputs and display texts of a block. */
  private async snapshot(doc: DocId, sheet: number, range: RangeDto): Promise<{ inputs: string[][]; texts: string[][] }> {
    const [block] = await this.cells(doc, sheet, [range]);
    const inputs: string[][] = [];
    const texts: string[][] = [];
    for (let r = range.r0; r <= range.r1; r++) {
      const ir: string[] = [];
      const tr: string[] = [];
      for (let c = range.c0; c <= range.c1; c++) {
        const v = block?.cells.find((x) => x.r === r && x.c === c);
        ir.push(v ? (await super.cellEditInfo(doc, sheet, r, c)).input : "");
        tr.push(v?.text ?? "");
      }
      inputs.push(ir);
      texts.push(tr);
    }
    return { inputs, texts };
  }

  // -- mutations ---------------------------------------------------------------------

  override async apply(doc: DocId, tx: Transaction): Promise<ApplyResult> {
    this.recorded.push(...tx.ops.map((op) => structuredClone(op)));
    // v0.3 first: protection and validation checks, and side-model ops.
    const before = JSON.stringify(this.docExt(doc));
    const pre = await this.preV3(doc, tx.ops);
    const res = await this.applyKept(doc, { ...tx, ops: pre.kept.map((k) => k.op) }, before, pre.index);
    const errors = [...pre.errors, ...res.errors].sort((a, b) => a.opIndex - b.opIndex);
    const changed: SheetRanges[] = [...res.changed];
    for (const [sheet, ranges] of pre.touched) changed.push({ sheet, ranges });
    const out: ApplyResult = { ...res, errors, changed, layoutChanged: [...new Set([...res.layoutChanged, ...pre.layout])] };
    if (pre.warnings.length > 0) out.warnings = pre.warnings;
    if (tx.ops.some((o) => o.type === "createPivot" || o.type === "updatePivot" || o.type === "refreshPivot" || o.type === "deletePivot")) out.pivotsChanged = true;
    // W5-5: ids of added charts/pictures, object-list changes and charts whose ranges were edited.
    if (pre.created.length > 0) out.created = pre.created;
    if (pre.objects.size > 0) out.objectsChanged = [...pre.objects];
    const w5 = this.docExt(doc).w5;
    const chartsChanged = new Map<number, Set<number>>();
    for (const [sheet, ids] of pre.chartsChanged) chartsChanged.set(sheet, new Set(ids));
    for (const sr of changed) {
      for (const id of chartsTouched(w5, sr.sheet, sr.ranges, (n) => this.sheetNames.get(doc)?.get(n))) {
        const set = chartsChanged.get(sr.sheet) ?? new Set<number>();
        set.add(id);
        chartsChanged.set(sr.sheet, set);
      }
    }
    const list = [...chartsChanged].flatMap(([sheet, ids]) => [...ids].map((id) => ({ sheet, id })));
    if (list.length > 0) out.chartsChanged = list;
    return out;
  }

  /**
   * Splits a transaction into the ops the mock applies (`kept`, with their
   * original index) and the v0.3 ones handled here: refusals (`protected`,
   * `validation`), validation warnings and side-model changes.
   */
  private async preV3(doc: DocId, ops: readonly Op[]): Promise<{ kept: { op: Op; index: number }[]; errors: OpError[]; warnings: OpError[]; touched: Map<number, RangeDto[]>; layout: Set<number>; index: number[]; created: CreatedId[]; objects: Set<number>; chartsChanged: Map<number, number[]> }> {
    const x = this.docExt(doc).v3;
    const kept: { op: Op; index: number }[] = [];
    const errors: OpError[] = [];
    const warnings: OpError[] = [];
    const touched = new Map<number, RangeDto[]>();
    const layout = new Set<number>();
    const created: CreatedId[] = [];
    const objects = new Set<number>();
    const chartsChanged = new Map<number, number[]>();
    for (const [i, op] of ops.entries()) {
      // W5-6 data tools, pivots, styles and settings (emulated in mockW56.ts).
      const w56 = await applyW56Op(this.docExt(doc).w56, op, this.w56Ctx(doc));
      if (w56.kind === "error") {
        errors.push({ opIndex: i, code: w56.code, message: w56.message });
        continue;
      }
      if (w56.kind === "handled") {
        for (const o of w56.ops) kept.push({ op: o, index: i });
        for (const [sheet, rs] of w56.touched) touched.set(sheet, [...(touched.get(sheet) ?? []), ...rs]);
        for (const sh of w56.layout) layout.add(sh);
        continue;
      }
      // W5-5 objects first (they are refused on a protected sheet like the engine does).
      const w5 = await applyW5Op(this.docExt(doc).w5, op, (sheet, range) => this.cellsOf(doc, sheet, range), (n) => this.sheetNames.get(doc)?.get(n));
      if (w5.kind === "error") {
        errors.push({ opIndex: i, code: w5.code, message: w5.message });
        continue;
      }
      if (w5.kind === "handled") {
        if (w5.created !== null) created.push({ opIndex: i, id: w5.created });
        if (w5.objects) objects.add(w5.sheet);
        if (w5.chartsChanged.length > 0) chartsChanged.set(w5.sheet, [...(chartsChanged.get(w5.sheet) ?? []), ...w5.chartsChanged]);
        layout.add(w5.sheet);
        continue;
      }
      const refusal = protectionRefusal(x, op);
      if (refusal) {
        errors.push({ opIndex: i, code: "protected", message: refusal });
        continue;
      }
      if (op.type === "setCellInput") {
        const v = validationAt(x, op.sheet, op.r, op.c);
        if (v?.error && !passes(v, op.input, await this.listItemsOf(doc, v))) {
          const message = v.error.text || DEFAULT_VALIDATION_MESSAGE;
          if (v.error.style === "stop") {
            errors.push({ opIndex: i, code: "validation", message });
            continue;
          }
          warnings.push({ opIndex: i, code: "validation", message });
        }
      }
      const o = applyV3Op(x, op);
      if (o.kind === "pass") kept.push({ op, index: i });
      else if (o.kind === "error") errors.push({ opIndex: i, code: o.code, message: o.message });
      else {
        const sheet = "sheet" in op && typeof op.sheet === "number" ? op.sheet : this.firstSheet(doc);
        if (o.touched.length > 0) touched.set(sheet, [...(touched.get(sheet) ?? []), ...o.touched]);
        if (o.layout) layout.add(sheet);
      }
    }
    return { kept, errors, warnings, touched, layout, index: kept.map((k) => k.index), created, objects, chartsChanged };
  }

  /** Cells of a range for the W5 mock (empty cells left out). */
  private async cellsOf(doc: DocId, sheet: number, range: RangeDto): Promise<CellView[]> {
    const clipped = { r0: range.r0, c0: range.c0, r1: Math.min(range.r1, range.r0 + 4000), c1: Math.min(range.c1, range.c0 + 255) };
    const [block] = await this.cells(doc, sheet, [clipped]).catch((): CellBlock[] => []);
    return block?.cells ?? [];
  }

  // -- W5-5 queries (charts, pictures, print, detective) ------------------------

  override async charts(doc: DocId, sheet: number): Promise<ChartDto[]> {
    await super.charts(doc, sheet);
    return structuredClone(this.docExt(doc).w5.charts[sheet] ?? []);
  }

  override async images(doc: DocId, sheet: number): Promise<ImageDto[]> {
    await super.images(doc, sheet);
    // Like the engine: the bytes are not in the list (the UI keeps what it inserted this session).
    return (this.docExt(doc).w5.images[sheet] ?? []).map((im) => ({ ...im, data: "" }));
  }

  override async chartData(doc: DocId, sheet: number, id: number): Promise<ChartDataDto> {
    const chart = (this.docExt(doc).w5.charts[sheet] ?? []).find((c) => c.id === id);
    if (!chart) throw { code: "not_found", message: `chart ${id} not found` };
    return chartDataFor(sheet, chart, this.versions.get(doc) ?? 0, (s, r) => this.cellsOf(doc, s, r), (n) => this.sheetNames.get(doc)?.get(n));
  }

  override async suggestChart(doc: DocId, sheet: number, sel: RangeDto, kind: ChartKind): Promise<ChartDto> {
    const used = (await super.sheetLayout(doc, sheet)).used;
    const name = [...(this.sheetNames.get(doc) ?? [])].find(([, id]) => id === sheet)?.[0] ?? "sheet1";
    const proper = name.replace(/^./, (ch) => ch.toUpperCase());
    return suggestChartFor(proper, sel, kind, used, (s, r) => this.cellsOf(doc, s, r), sheet);
  }

  private async printSheets(doc: DocId, opts: PrintOptionsDto) {
    const order = this.sheetOrder.get(doc) ?? [this.firstSheet(doc)];
    const active = order[0] ?? 1;
    const ids = opts.sheets.type === "all" ? order : opts.sheets.type === "some" ? opts.sheets.ids : [active];
    const out = [];
    for (const id of ids) out.push({ id, setup: await this.pageSetup(doc, id), layout: await this.sheetLayout(doc, id) });
    return out;
  }

  override async printLayout(doc: DocId, opts: PrintOptionsDto): Promise<PrintLayoutDto> {
    this.recordedCalls.push({ type: "printLayout", opts: structuredClone(opts) });
    return printLayoutFor(await this.printSheets(doc, opts), opts);
  }

  override async pageBreaks(doc: DocId, sheet: number): Promise<PageBreaksDto> {
    return pageBreaksFor(await this.pageSetup(doc, sheet), await this.sheetLayout(doc, sheet));
  }

  /** Prepared print jobs (an LRU of 4, like the engine). */
  private readonly jobs: { job: number; doc: DocId; layout: PrintLayoutDto; charts: ChartSvgDto[] }[] = [];
  private nextJob = 1;

  override async preparePrintJob(doc: DocId, opts: PrintOptionsDto, charts: ChartSvgDto[]): Promise<PrintJobInfo> {
    this.recordedCalls.push({ type: "preparePrintJob", opts: structuredClone(opts), charts: charts.map((c) => ({ sheet: c.sheet, id: c.id, bytes: c.svg.length })) });
    const layout = printLayoutFor(await this.printSheets(doc, opts), opts);
    const job = this.nextJob++;
    this.jobs.push({ job, doc, layout, charts });
    if (this.jobs.length > 4) this.jobs.shift();
    return { job, layout };
  }

  override async printJobPagePng(job: number, page: number, dpi: number): Promise<string> {
    const j = this.jobs.find((x) => x.job === job);
    const p = j?.layout.pages[page];
    if (!j || !p) throw { code: "not_found", message: `print job ${job} page ${page} not found` };
    const layout = await this.sheetLayout(j.doc, p.sheet);
    const cells = await this.cellsOf(j.doc, p.sheet, { r0: p.rows[0], c0: p.cols[0], r1: p.rows[1], c1: p.cols[1] });
    const boxes = (this.docExt(j.doc).w5.charts[p.sheet] ?? [])
      .filter((c) => c.anchor.r >= p.rows[0] && c.anchor.r <= p.rows[1] && c.anchor.c >= p.cols[0] && c.anchor.c <= p.cols[1])
      .map((c) => ({ id: c.id, x: p.originMm[0] + 10 + (c.anchor.c - p.cols[0]) * 20, y: p.originMm[1] + 10 + (c.anchor.r - p.rows[0]) * 6, w: c.anchor.widthMm * (p.scalePct / 100), h: c.anchor.heightMm * (p.scalePct / 100) }));
    const png = renderPagePng(p, cells, layout, Math.min(dpi, 200), j.charts, boxes);
    return png || BLANK_PNG;
  }

  override async precedents(doc: DocId, sheet: number, r: number, c: number): Promise<TraceDto[]> {
    await super.precedents(doc, sheet, r, c);
    const input = (await super.cellEditInfo(doc, sheet, r, c)).input;
    const sheets = (n: string) => this.sheetNames.get(doc)?.get(n);
    const isError = (s: number, rg: RangeDto): boolean => false && s === 0 && rg.r0 === 0;
    const list = precedentsOf(input, sheet, sheets, isError);
    const out: TraceDto[] = [];
    for (const t of list) {
      const cells = await this.cellsOf(doc, t.sheet, t.range);
      out.push({ ...t, error: cells.some((v) => v.kind === "error") });
    }
    return out;
  }

  override async dependents(doc: DocId, sheet: number, r: number, c: number): Promise<TraceDto[]> {
    await super.dependents(doc, sheet, r, c);
    const used = (await super.sheetLayout(doc, sheet)).used;
    if (!used) return [];
    const cells = await this.cellsOf(doc, sheet, used);
    const formulas: { r: number; c: number; input: string; error: boolean }[] = [];
    for (const v of cells) {
      if ((v.flags & 1) === 0) continue;
      const input = (await super.cellEditInfo(doc, sheet, v.r, v.c)).input;
      formulas.push({ r: v.r, c: v.c, input, error: v.kind === "error" });
    }
    return dependentsOf({ r, c }, formulas, sheet, (n) => this.sheetNames.get(doc)?.get(n));
  }

  /** The mock's own apply of the kept ops; OpError indices map back through `index`. */
  private async applyKept(doc: DocId, tx: Transaction, before: string, index: number[]): Promise<ApplyResult> {
    const mapErrors = (res: ApplyResult): ApplyResult => ({ ...res, errors: res.errors.map((e) => ({ ...e, opIndex: index[e.opIndex] ?? e.opIndex })) });
    if (tx.ops.some((o) => "sheet" in o && o.sheet === -1)) return mapErrors(await this.applyWithNewSheet(doc, tx, before));
    if (tx.ops.some(isStructural)) {
      // One step per op (later ops see the shifted cells).
      let last: ApplyResult | null = null;
      for (const op of tx.ops) {
        const b = JSON.stringify(this.docExt(doc));
        const ops = isStructural(op) ? await this.rewrite(doc, op) : await this.rewriteV02(doc, op, [], new Set());
        last = this.track(doc, await super.apply(doc, { ...tx, ops }), b);
      }
      return mapErrors(last as ApplyResult);
    }
    const errors: OpError[] = [];
    const layout = new Set<number>();
    const ops: Op[] = [];
    for (const [i, op] of tx.ops.entries()) {
      const errs: OpError[] = [];
      ops.push(...(await this.rewriteV02(doc, op, errs, layout)));
      errors.push(...errs.map((e) => ({ ...e, opIndex: index[i] ?? i })));
    }
    const changedExt = JSON.stringify(this.docExt(doc)) !== before;
    // A side-model-only change still needs an undo step in the mock: a no-op style touch.
    if (changedExt && ops.length === 0) {
      const sheet = [...layout][0] ?? this.firstSheet(doc);
      ops.push({ type: "applyStyle", sheet, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], patch: EMPTY_PATCH });
    }
    if (ops.length === 0) {
      const res = mapErrors(await super.apply(doc, { ...tx, ops: [] }));
      return this.track(doc, { ...res, errors: [...errors, ...res.errors] });
    }
    const res = mapErrors(await super.apply(doc, { ...tx, ops }));
    await this.reapplyFilters(doc, layout);
    return this.track(doc, { ...res, errors: [...errors, ...res.errors], layoutChanged: [...new Set([...res.layoutChanged, ...layout])] }, before);
  }

  override async undo(doc: DocId): Promise<ApplyResult> {
    const res = await super.undo(doc);
    const stack = this.extUndo.get(doc);
    const prev = stack?.pop();
    if (prev !== undefined) {
      const redo = this.extRedo.get(doc) ?? [];
      redo.push(JSON.stringify(this.docExt(doc)));
      this.extRedo.set(doc, redo);
      this.ext.set(doc, JSON.parse(prev) as DocExt);
    }
    return this.track(doc, res);
  }


  override async redo(doc: DocId): Promise<ApplyResult> {
    const res = await super.redo(doc);
    const next = this.extRedo.get(doc)?.pop();
    if (next !== undefined) {
      const undo = this.extUndo.get(doc) ?? [];
      undo.push(JSON.stringify(this.docExt(doc)));
      this.extUndo.set(doc, undo);
      this.ext.set(doc, JSON.parse(next) as DocExt);
    }
    return this.track(doc, res);
  }

  private pushUndo(doc: DocId, snapshot: string): void {
    const stack = this.extUndo.get(doc) ?? [];
    stack.push(snapshot);
    if (stack.length > 100) stack.shift();
    this.extUndo.set(doc, stack);
    this.extRedo.set(doc, []);
  }

  /**
   * Remembers the sheet order and the document version; when an apply made
   * a new mock undo step (`before` given and the version moved), stores the
   * side model as it was so undo can restore it.
   */
  private track(doc: DocId, res: ApplyResult, before?: string): ApplyResult {
    if (res.info) {
      res.info = { ...res.info, sheets: decorateSheets(this.docExt(doc).w56, res.info.sheets) };
      this.sheetOrder.set(doc, res.info.sheets.map((s) => s.id));
      this.sheetNames.set(doc, new Map(res.info.sheets.map((s) => [s.name.toLowerCase(), s.id])));
    }
    const v0 = this.versions.get(doc) ?? 0;
    if (before !== undefined && res.version !== v0) this.pushUndo(doc, before);
    this.versions.set(doc, res.version);
    return res;
  }

  private readonly versions = new Map<DocId, number>();

  private firstSheet(doc: DocId): number {
    return this.sheetOrder.get(doc)?.[0] ?? 1;
  }

  /** Recomputes the rows hidden by each AutoFilter (after any edit, as the engine does). */
  private async reapplyFilters(doc: DocId, layout: Set<number>): Promise<void> {
    const d = this.ext.get(doc);
    if (!d) return;
    for (const [id, s] of Object.entries(d.sheets)) {
      const sheet = Number(id);
      if (!s.filter) {
        if (s.filteredRows.length > 0) {
          s.filteredRows = [];
          layout.add(sheet);
        }
        continue;
      }
      const f = s.filter;
      const body = { ...f.range, r0: f.range.r0 + 1 };
      const [block] = await this.cells(doc, sheet, [body]);
      const rows: number[] = [];
      for (let r = body.r0; r <= body.r1; r++) {
        for (const [col, cf] of Object.entries(f.columns)) {
          const text = block?.cells.find((v) => v.r === r && v.c === Number(col))?.text ?? "";
          const okValues = cf.values === null || cf.values.includes(text);
          const okCond = cf.condition === null || passesCondition(text, cf.condition);
          if (!okValues || !okCond) {
            rows.push(r);
            break;
          }
        }
      }
      if (JSON.stringify(rows) !== JSON.stringify(s.filteredRows)) {
        s.filteredRows = rows;
        layout.add(sheet);
      }
    }
    // W5-6: standard / advanced filters hide rows the same way.
    for (const [id, f] of Object.entries(d.w56.stdFilters)) {
      if (!f) continue;
      const sheet = Number(id);
      const s = this.sheetExt(doc, sheet);
      const [block] = await this.cells(doc, sheet, [f.range]);
      const hidden = stdFilterHidden(f, block?.cells ?? []);
      const merged = [...new Set([...s.filteredRows, ...hidden])].sort((a, b) => a - b);
      if (JSON.stringify(merged) !== JSON.stringify(s.filteredRows)) {
        s.filteredRows = merged;
        layout.add(sheet);
      }
    }
  }

  /** `addSheet` followed by ops on the new sheet (`sheet: -1`): two mock steps. */
  private async applyWithNewSheet(doc: DocId, tx: Transaction, before: string): Promise<ApplyResult> {
    const known = new Set(this.sheetOrder.get(doc) ?? []);
    const add = tx.ops.find((o) => o.type === "addSheet");
    const first = this.track(doc, await super.apply(doc, { ...tx, ops: add ? [add] : [] }), before);
    const created = first.info?.sheets.find((sh) => !known.has(sh.id))?.id;
    const rest = tx.ops.filter((o) => o !== add).map((o) => ("sheet" in o && o.sheet === -1 && created !== undefined ? ({ ...o, sheet: created } as Op) : o));
    if (rest.length === 0) return first;
    const b = JSON.stringify(this.docExt(doc));
    const second = this.track(doc, await super.apply(doc, { ...tx, ops: rest }), b);
    return { ...second, info: second.info ?? first.info, sheetsChanged: true, errors: [...first.errors, ...second.errors] };
  }

  /** Rewrites one v0.2 op into mock ops (and side-model changes). Other ops pass through. */
  private async rewriteV02(doc: DocId, op: Op, errors: OpError[], layout: Set<number>): Promise<Op[]> {
    const fail = (code: string, message: string): Op[] => {
      errors.push({ opIndex: 0, code, message });
      return [];
    };
    switch (op.type) {
      case "setCellInput": {
        const arr = this.ext.get(doc)?.sheets[op.sheet]?.arrays.find((a) => inside(a.range, op.r, op.c));
        if (arr) {
          if (arr.range.r0 === arr.range.r1 && arr.range.c0 === arr.range.c1) {
            const s = this.sheetExt(doc, op.sheet);
            s.arrays = s.arrays.filter((a) => a !== arr);
            return [op];
          }
          return fail("array_block", "You cannot change part of an array.");
        }
        return [op];
      }
      case "defineName": {
        const d = this.docExt(doc);
        const sheetName = op.scope === null ? null : `Sheet${op.scope}`;
        d.names = d.names.filter((n) => !(n.name.toLowerCase() === op.name.toLowerCase() && n.scope === op.scope));
        d.names.push({ name: op.name, scope: op.scope, refersTo: op.refersTo, sheetName });
        d.names.sort((a, b) => (a.scope ?? -1) - (b.scope ?? -1) || a.name.localeCompare(b.name));
        return [];
      }
      case "deleteName": {
        const d = this.docExt(doc);
        d.names = d.names.filter((n) => !(n.name.toLowerCase() === op.name.toLowerCase() && n.scope === op.scope));
        return [];
      }
      case "setArrayFormula": {
        const s = this.sheetExt(doc, op.sheet);
        s.arrays = s.arrays.filter((a) => !overlaps(a.range, op.range));
        s.arrays.push({ range: { ...op.range }, input: op.input });
        layout.add(op.sheet);
        return [
          { type: "clear", sheet: op.sheet, ranges: [{ ...op.range }], what: "contents" },
          { type: "setCellInput", sheet: op.sheet, r: op.range.r0, c: op.range.c0, input: op.input },
        ];
      }
      case "moveRange": {
        const snap = await this.snapshot(doc, op.sheet, op.from);
        const s = this.sheetExt(doc, op.sheet);
        const dr = op.to.r - op.from.r0;
        const dc = op.to.c - op.from.c0;
        for (const m of s.merges) if (contains(op.from, m)) Object.assign(m, { r0: m.r0 + dr, r1: m.r1 + dr, c0: m.c0 + dc, c1: m.c1 + dc });
        layout.add(op.sheet);
        return [
          { type: "clear", sheet: op.sheet, ranges: [{ ...op.from }], what: "all" },
          { type: "setRangeInputs", sheet: op.sheet, r: op.to.r, c: op.to.c, rows: snap.inputs },
        ];
      }
      case "mergeCells": {
        const s = this.sheetExt(doc, op.sheet);
        if (s.merges.some((m) => overlaps(m, op.range) && !contains(op.range, m))) return fail("merged", "That overlaps part of a merged area.");
        s.merges = s.merges.filter((m) => !contains(op.range, m));
        s.merges.push({ ...op.range });
        layout.add(op.sheet);
        const rest = withoutAnchor(op.range);
        return rest.length > 0 ? [{ type: "clear", sheet: op.sheet, ranges: rest, what: "contents" }] : [];
      }
      case "unmergeCells": {
        const s = this.sheetExt(doc, op.sheet);
        s.merges = s.merges.filter((m) => !overlaps(m, op.range));
        layout.add(op.sheet);
        return [];
      }
      case "setFrozen": {
        this.sheetExt(doc, op.sheet).frozen = op.rows === 0 && op.cols === 0 ? null : { r: op.rows, c: op.cols };
        layout.add(op.sheet);
        return [];
      }
      case "setRowsHidden":
      case "setColsHidden": {
        const s = this.sheetExt(doc, op.sheet);
        const key = op.type === "setRowsHidden" ? "hiddenRows" : "hiddenCols";
        const set = new Set(s[key]);
        for (let i = op.start; i <= op.end; i++) {
          if (op.hidden) set.add(i);
          else set.delete(i);
        }
        s[key] = [...set].sort((a, b) => a - b);
        layout.add(op.sheet);
        return [];
      }
      case "setAutoFilter": {
        this.sheetExt(doc, op.sheet).filter = op.range ? { range: { ...op.range }, columns: {} } : null;
        layout.add(op.sheet);
        return [];
      }
      case "setColumnFilter": {
        const f = this.sheetExt(doc, op.sheet).filter;
        if (!f || op.col < f.range.c0 || op.col > f.range.c1) return fail("invalid_argument", "That column is not in the AutoFilter.");
        if (op.filter) f.columns[op.col] = structuredClone(op.filter);
        else delete f.columns[op.col];
        layout.add(op.sheet);
        return [];
      }
      case "sortRange": {
        const body = op.hasHeader ? { ...op.range, r0: op.range.r0 + 1 } : { ...op.range };
        if (body.r1 < body.r0) return [];
        const snap = await this.snapshot(doc, op.sheet, body);
        const [block] = await this.cells(doc, op.sheet, [body]);
        const view = (r: number, c: number) => block?.cells.find((v) => v.r === r && v.c === c);
        const order = snap.inputs.map((_, i) => i);
        order.sort((a, b) => {
          for (const k of op.keys) {
            const d = compareValues(view(body.r0 + a, k.col), view(body.r0 + b, k.col));
            if (d !== 0) {
              const empty = (i: number) => !view(body.r0 + i, k.col);
              if (empty(a) !== empty(b)) return d; // empties last either way
              return k.ascending ? d : -d;
            }
          }
          return a - b;
        });
        return [{ type: "setRangeInputs", sheet: op.sheet, r: body.r0, c: body.c0, rows: order.map((i) => snap.inputs[i] ?? []) }];
      }
      case "replaceAll": {
        const q = op.query;
        const sheets = op.sheet === null ? (this.sheetOrder.get(doc) ?? [this.firstSheet(doc)]) : [op.sheet];
        const out: Op[] = [];
        const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const pattern = q.mode === "regex" ? q.text : esc(q.text);
        let re: RegExp;
        try {
          re = new RegExp(q.wholeCell ? `^(?:${pattern})$` : pattern, q.matchCase ? "g" : "gi");
        } catch {
          return fail("invalid_argument", "Invalid regular expression.");
        }
        for (const sheet of sheets) {
          const used = (await super.sheetLayout(doc, sheet)).used;
          if (!used) continue;
          const [block] = await this.cells(doc, sheet, [used]);
          for (const v of block?.cells ?? []) {
            const input = (await super.cellEditInfo(doc, sheet, v.r, v.c)).input;
            if (input === "") continue;
            re.lastIndex = 0;
            if (!re.test(input)) continue;
            re.lastIndex = 0;
            out.push({ type: "setCellInput", sheet, r: v.r, c: v.c, input: input.replace(re, op.replace) });
          }
        }
        return out;
      }
      case "pasteSpecial": {
        const o = op.options;
        let rows: string[][];
        if (op.payload.type === "internal") {
          const clip = this.clips.get(op.payload.token);
          if (!clip) return fail("not_found", "The clipboard has expired.");
          rows = o.valuesOnly ? clip.texts : clip.inputs;
        } else {
          rows = op.payload.text.replace(/\r?\n$/, "").split(/\r?\n/).map((l) => l.split("\t"));
        }
        if (o.formatsOnly) return [{ type: "applyStyle", sheet: op.sheet, ranges: [{ ...op.target }], patch: EMPTY_PATCH }];
        if (o.transpose) {
          const w = Math.max(0, ...rows.map((r) => r.length));
          rows = Array.from({ length: w }, (_, c) => rows.map((r) => r[c] ?? ""));
        }
        const out: Op[] = [];
        const [block] = await this.cells(doc, op.sheet, [{ r0: op.target.r0, c0: op.target.c0, r1: op.target.r0 + rows.length - 1, c1: op.target.c0 + Math.max(0, ...rows.map((r) => r.length)) - 1 }]);
        rows.forEach((row, i) =>
          row.forEach((input, j) => {
            if (o.skipEmpty && input === "") return;
            const r = op.target.r0 + i;
            const c = op.target.c0 + j;
            let next = input;
            if (o.operation !== "none") {
              const cur = block?.cells.find((v) => v.r === r && v.c === c);
              const a = cur?.num ?? (cur ? numberOf(cur.text) : 0) ?? 0;
              const b = numberOf(input);
              if (b === null) return;
              const n = o.operation === "add" ? a + b : o.operation === "subtract" ? a - b : o.operation === "multiply" ? a * b : b === 0 ? NaN : a / b;
              next = Number.isFinite(n) ? String(Number(n.toPrecision(15))) : "#DIV/0!";
            }
            out.push({ type: "setCellInput", sheet: op.sheet, r, c, input: next });
          }),
        );
        return out;
      }
      case "fillRange":
        return emulateFill({ client: this, doc, sheet: op.sheet }, op.source, op.target);
      case "applyStyle":
        return op.patch.borders ? this.borderOps(doc, op.sheet, op.ranges, op.patch) : [op];
      default:
        return [op];
    }
  }

  /**
   * `applyStyle` with `borders`: the mock copies `patch.borders` verbatim
   * into the style, so each cell gets its resulting `BordersDto` (grouped by
   * value) through its own applyStyle.
   */
  private async borderOps(doc: DocId, sheet: number, ranges: RangeDto[], patch: StylePatch): Promise<Op[]> {
    const bp = patch.borders as BordersPatch;
    const rest: StylePatch = { ...patch, borders: null };
    const ops: Op[] = [];
    if (Object.entries(rest).some(([k, v]) => k !== "borders" && v !== null)) ops.push({ type: "applyStyle", sheet, ranges, patch: rest });
    const groups = new Map<string, RangeDto[]>();
    for (const range of ranges) {
      const [block] = await this.cells(doc, sheet, [range]);
      const ids = [...new Set((block?.cells ?? []).map((v) => v.style))];
      const styles = new Map((await this.styles(doc, [0, ...ids])).map((s) => [s.id, s]));
      for (let r = range.r0; r <= range.r1; r++) {
        for (let c = range.c0; c <= range.c1; c++) {
          const cur = styles.get(block?.cells.find((v) => v.r === r && v.c === c)?.style ?? 0)?.borders ?? { top: null, bottom: null, left: null, right: null };
          const next: BordersDto = { ...cur };
          const set = (edge: keyof BordersDto, e: { line: BordersDto["top"] } | null) => {
            if (e) next[edge] = e.line ? { ...e.line } : null;
          };
          set("top", r === range.r0 ? bp.top : bp.innerH);
          set("bottom", r === range.r1 ? bp.bottom : bp.innerH);
          set("left", c === range.c0 ? bp.left : bp.innerV);
          set("right", c === range.c1 ? bp.right : bp.innerV);
          const key = JSON.stringify(next);
          if (key === JSON.stringify(cur)) continue;
          const list = groups.get(key) ?? [];
          list.push({ r0: r, c0: c, r1: r, c1: c });
          groups.set(key, list);
        }
      }
    }
    for (const [key, list] of groups) {
      ops.push({ type: "applyStyle", sheet, ranges: list, patch: { ...EMPTY_PATCH, borders: JSON.parse(key) as BordersPatch } });
    }
    return ops;
  }

  /** Rewrites one structural op into cell ops over the used range. */
  private async rewrite(doc: DocId, op: Structural): Promise<Op[]> {
    const sheet = op.sheet;
    const used = (await super.sheetLayout(doc, sheet)).used;
    if (!used || op.count <= 0) return [];
    const rows = op.type === "insertRows" || op.type === "deleteRows";
    const insert = op.type === "insertRows" || op.type === "insertCols";
    const end = rows ? used.r1 : used.c1;
    if (op.at > end) return [];
    // The block that moves, and by how much.
    const from = insert ? op.at : op.at + op.count;
    if (from > end) {
      // Deleting the tail: just clear it.
      const tail: RangeDto = rows ? { ...used, r0: op.at, r1: end } : { ...used, c0: op.at, c1: end };
      return [{ type: "clear", sheet, ranges: [tail], what: "all" }];
    }
    const block: RangeDto = rows ? { ...used, r0: from, r1: end } : { ...used, c0: from, c1: end };
    const shift = insert ? op.count : -op.count;
    const clearRange: RangeDto = rows ? { ...used, r0: Math.min(op.at, from), r1: end } : { ...used, c0: Math.min(op.at, from), c1: end };

    const [cells] = await this.cells(doc, sheet, [block]);
    const h = block.r1 - block.r0 + 1;
    const w = block.c1 - block.c0 + 1;
    const inputs: string[][] = Array.from({ length: h }, () => Array.from({ length: w }, () => ""));
    const byStyle = new Map<number, RangeDto[]>();
    for (const cell of cells?.cells ?? []) {
      const info = await super.cellEditInfo(doc, sheet, cell.r, cell.c);
      const row = inputs[cell.r - block.r0];
      if (row) row[cell.c - block.c0] = info.input;
      if (cell.style !== 0) {
        const r = rows ? cell.r + shift : cell.r;
        const c = rows ? cell.c : cell.c + shift;
        const list = byStyle.get(cell.style) ?? [];
        list.push({ r0: r, c0: c, r1: r, c1: c });
        byStyle.set(cell.style, list);
      }
    }
    const ops: Op[] = [
      { type: "clear", sheet, ranges: [clearRange], what: "all" },
      { type: "setRangeInputs", sheet, r: rows ? block.r0 + shift : block.r0, c: rows ? block.c0 : block.c0 + shift, rows: inputs },
    ];
    if (byStyle.size > 0) {
      const styles = await this.styles(doc, [...byStyle.keys()]);
      for (const st of styles) {
        const patch: StylePatch = {
          bold: st.bold,
          italic: st.italic,
          underline: st.underline,
          strike: st.strike,
          fontFamily: st.fontFamily,
          fontSizePt: st.fontSizePt,
          color: st.color,
          fill: st.fill,
          hAlign: st.hAlign,
          vAlign: st.vAlign,
          wrap: st.wrap,
          numberFormat: st.numberFormat,
          borders: st.borders as unknown as BordersPatch,
        };
        ops.push({ type: "applyStyle", sheet, ranges: byStyle.get(st.id) ?? [], patch });
      }
    }
    return ops;
  }
}

/** Overlays hidden (and filtered) indices on size runs. */
export function overlayRuns(runs: readonly DimRunPx[], defaultSize: number, hidden: ReadonlySet<number>, filtered: ReadonlySet<number>): DimRunPx[] {
  if (hidden.size === 0 && filtered.size === 0) return runs.map((r) => ({ ...r }));
  const sizeAt = (i: number): number => runs.find((r) => i >= r.start && i <= r.end)?.size ?? defaultSize;
  const special = [...new Set([...hidden, ...filtered])].sort((a, b) => a - b);
  const out: DimRunPx[] = [];
  const push = (run: DimRunPx) => {
    const last = out[out.length - 1];
    if (last && last.end + 1 === run.start && last.size === run.size && last.hidden === run.hidden && (last.filtered ?? null) === (run.filtered ?? null)) last.end = run.end;
    else out.push(run);
  };
  // Split the existing runs around the special indices.
  const points = new Set<number>(special);
  const all: DimRunPx[] = [];
  for (const r of runs) {
    let s = r.start;
    for (const p of special) {
      if (p < r.start || p > r.end) continue;
      if (p > s) all.push({ ...r, start: s, end: p - 1 });
      s = p + 1;
    }
    if (s <= r.end) all.push({ ...r, start: s, end: r.end });
  }
  for (const p of points) {
    const isFiltered = filtered.has(p);
    all.push({ start: p, end: p, size: sizeAt(p), hidden: true, filtered: isFiltered && !hidden.has(p) ? true : null });
  }
  all.sort((a, b) => a.start - b.start);
  for (const r of all) push(r);
  return out;
}
