/**
 * `MockClient`: an in-memory {@link EngineClient} for building and testing
 * the grid and the app before the Rust engine exists, and for browser e2e.
 *
 * It is deliberately tiny and is NOT a spreadsheet engine:
 * - inputs are recognised as number, TRUE/FALSE, formula or text;
 * - formulas support only `=ref`, `=a OP b` (OP one of + - * /, operands a
 *   cell reference or a number) and `=SUM(range)`; anything else is `#NAME?`;
 * - formulas are evaluated on read, so dependents are always fresh;
 * - styles support the full `StylePatch` (display only; number formats are
 *   stored but not applied);
 * - undo/redo restores whole-document snapshots;
 * - structural ops (insert/delete rows/cols) report an `unsupported` OpError;
 * - v0.3 (conditional formats, validations, notes, links, protection, page
 *   setup, properties) is stored per document and returned by the queries,
 *   but not evaluated, enforced or undoable; goal seek is unsupported.
 *
 * Seeded in WP0; owned by WP7 from here on (WP6 may extend it via WP7 review).
 */
import type { ApiError } from "./generated/ApiError";
import type { ApplyResult } from "./generated/ApplyResult";
import type { CellBlock } from "./generated/CellBlock";
import type { CellEditInfo } from "./generated/CellEditInfo";
import type { CellPos } from "./generated/CellPos";
import type { CellView } from "./generated/CellView";
import type { ClipboardOut } from "./generated/ClipboardOut";
import type { DimRunPx } from "./generated/DimRunPx";
import type { Direction } from "./generated/Direction";
import type { DocChangedEvent } from "./generated/DocChangedEvent";
import type { DocId } from "./generated/DocId";
import type { FileFormat } from "./generated/FileFormat";
import type { FunctionInfo } from "./generated/FunctionInfo";
import type { MenuActionEvent } from "./generated/MenuActionEvent";
import type { Op } from "./generated/Op";
import type { OpError } from "./generated/OpError";
import type { RangeDto } from "./generated/RangeDto";
import type { SelectionDto } from "./generated/SelectionDto";
import type { SelectionStats } from "./generated/SelectionStats";
import type { SheetLayout } from "./generated/SheetLayout";
import type { SheetRanges } from "./generated/SheetRanges";
import type { StyleDto } from "./generated/StyleDto";
import type { StylePatch } from "./generated/StylePatch";
import type { Suggestion } from "./generated/Suggestion";
import type { Transaction } from "./generated/Transaction";
import type { ValueKind } from "./generated/ValueKind";
import type { WorkbookInfo } from "./generated/WorkbookInfo";
import type { AutoFilterInfo } from "./generated/AutoFilterInfo";
import type { FindQuery } from "./generated/FindQuery";
import type { FindResult } from "./generated/FindResult";
import type { NameInfo } from "./generated/NameInfo";
import type { ConditionalFormatDto } from "./generated/ConditionalFormatDto";
import type { DocPropertiesDto } from "./generated/DocPropertiesDto";
import type { GoalSeekRequest } from "./generated/GoalSeekRequest";
import type { GoalSeekResult } from "./generated/GoalSeekResult";
import type { LocaleInfo } from "./generated/LocaleInfo";
import type { NoteDto } from "./generated/NoteDto";
import type { PageSetupDto } from "./generated/PageSetupDto";
import type { ProtectionInfo } from "./generated/ProtectionInfo";
import type { ValidationDto } from "./generated/ValidationDto";
import type { ValidationInfo } from "./generated/ValidationInfo";
import type { AutocompleteDto } from "./generated/AutocompleteDto";
import type { CalcSettingsDto } from "./generated/CalcSettingsDto";
import type { CellStyleDto } from "./generated/CellStyleDto";
import type { ChartDataDto } from "./generated/ChartDataDto";
import type { ChartDto } from "./generated/ChartDto";
import type { ChartKind } from "./generated/ChartKind";
import type { ChartSvgDto } from "./generated/ChartSvgDto";
import type { DatabaseRangeDto } from "./generated/DatabaseRangeDto";
import type { DocumentStatsDto } from "./generated/DocumentStatsDto";
import type { ImageDto } from "./generated/ImageDto";
import type { NavigatorDto } from "./generated/NavigatorDto";
import type { PageBreaksDto } from "./generated/PageBreaksDto";
import type { PivotDto } from "./generated/PivotDto";
import type { PivotFieldItemsDto } from "./generated/PivotFieldItemsDto";
import type { PivotHit } from "./generated/PivotHit";
import type { PivotSource } from "./generated/PivotSource";
import type { PrintJobInfo } from "./generated/PrintJobInfo";
import type { PrintLayoutDto } from "./generated/PrintLayoutDto";
import type { PrintOptionsDto } from "./generated/PrintOptionsDto";
import type { ScenarioDto } from "./generated/ScenarioDto";
import type { SolverRequest } from "./generated/SolverRequest";
import type { SolverResult } from "./generated/SolverResult";
import type { TraceDto } from "./generated/TraceDto";
import type { EngineClientV4, SheetId, Unsubscribe } from "./client";
import { mockTemplate } from "./mockTemplates";
import { BUILTIN_TEMPLATES, type ShellClient, type TemplateInfo } from "./shell";

/** Grid limits, matching `tally_types::{MAX_ROWS, MAX_COLS}`. */
export const MAX_ROWS = 1_048_576;
export const MAX_COLS = 16_384;
/** Default column width in CSS pixels. */
export const DEFAULT_COL_WIDTH_PX = 100;
/** Default row height in CSS pixels. */
export const DEFAULT_ROW_HEIGHT_PX = 24;
/** Undo depth, as in the engine. */
const UNDO_DEPTH = 100;
/** Refuse ops touching more cells than this (the mock stores cells densely). */
const MAX_CELLS_PER_OP = 100_000;

// ---------------------------------------------------------------------------
// Internal model
// ---------------------------------------------------------------------------

interface MockCell {
  input: string;
  style: number;
}

interface MockSheet {
  id: SheetId;
  name: string;
  /** Keyed by {@link key}. */
  cells: Map<string, MockCell>;
  colWidths: Map<number, number>;
  rowHeights: Map<number, number>;
}

interface DocState {
  sheets: MockSheet[];
  activeSheet: SheetId;
  /** Index = style id; 0 is the default style. */
  styles: StyleDto[];
  nextSheetId: number;
}

interface UndoEntry {
  state: DocState;
  label: string | null;
  selectionBefore: SelectionDto | null;
  selectionAfter: SelectionDto | null;
}

/** v0.3 state of a mock document (not part of undo). */
interface MockV3 {
  cfs: Map<SheetId, ConditionalFormatDto[]>;
  validations: Map<SheetId, ValidationDto[]>;
  notes: Map<SheetId, Map<string, NoteDto>>;
  links: Map<SheetId, Map<string, string>>;
  protected: Map<SheetId, boolean>;
  structureProtected: boolean;
  pages: Map<SheetId, PageSetupDto>;
  props: DocPropertiesDto;
  nextId: number;
}

function newV3(): MockV3 {
  return {
    cfs: new Map(),
    validations: new Map(),
    notes: new Map(),
    links: new Map(),
    protected: new Map(),
    structureProtected: false,
    pages: new Map(),
    props: { title: null, subject: null, keywords: [], description: null, author: null, created: null, modified: null, generator: null },
    nextId: 1,
  };
}

/** The default page setup (LibreOffice's Default page style). */
export function defaultPageSetup(): PageSetupDto {
  return {
    orientation: "portrait",
    paper: { type: "a4" },
    margins: { topMm: 20, bottomMm: 20, leftMm: 20, rightMm: 20, headerMm: 7.5, footerMm: 7.5 },
    scale: { type: "percent", percent: 100 },
    printRange: null,
    repeatRows: null,
    repeatCols: null,
    header: { left: "", center: "&[SHEET]", right: "" },
    footer: { left: "", center: "Page &[PAGE]", right: "" },
    printGridlines: false,
    printHeaders: false,
    centerH: false,
    centerV: false,
    pageOrder: "topToBottom",
    firstPageNumber: null,
    rowBreaks: [],
    colBreaks: [],
  };
}

interface MockDoc {
  v3: MockV3;
  id: DocId;
  title: string;
  path: string | null;
  dirty: boolean;
  version: number;
  state: DocState;
  undo: UndoEntry[];
  redo: UndoEntry[];
}

/** A computed value. */
type Scalar = number | string | boolean | null | { error: string };

const key = (r: number, c: number): string => `${r},${c}`;

function parseKey(k: string): [number, number] {
  const i = k.indexOf(",");
  return [Number(k.slice(0, i)), Number(k.slice(i + 1))];
}

function apiError(code: string, message: string): ApiError {
  return { code, message };
}

function defaultStyle(): StyleDto {
  return {
    id: 0,
    bold: false,
    italic: false,
    underline: false,
    strike: false,
    fontFamily: null,
    fontSizePt: 10,
    color: null,
    fill: null,
    hAlign: "general",
    vAlign: "bottom",
    wrap: false,
    indent: 0,
    borders: { top: null, bottom: null, left: null, right: null },
    numberFormat: "General",
    display: null,
  };
}

function cloneState(s: DocState): DocState {
  return {
    activeSheet: s.activeSheet,
    nextSheetId: s.nextSheetId,
    styles: s.styles.map((st) => structuredClone(st)),
    sheets: s.sheets.map((sh) => ({
      id: sh.id,
      name: sh.name,
      cells: new Map([...sh.cells].map(([k, v]) => [k, { ...v }])),
      colWidths: new Map(sh.colWidths),
      rowHeights: new Map(sh.rowHeights),
    })),
  };
}

function newSheet(id: SheetId, name: string): MockSheet {
  return { id, name, cells: new Map(), colWidths: new Map(), rowHeights: new Map() };
}

// ---------------------------------------------------------------------------
// A1 helpers
// ---------------------------------------------------------------------------

/** 0-based column index to letters (0 → "A"). */
export function colToLetters(col: number): string {
  let n = col + 1;
  let s = "";
  while (n > 0) {
    const rem = (n - 1) % 26;
    s = String.fromCharCode(65 + rem) + s;
    n = Math.floor((n - 1) / 26);
  }
  return s;
}

/** Parses `A1` / `$A$1` (case-insensitive) into a 0-based position. */
export function parseA1(text: string): CellPos | null {
  const m = /^\$?([A-Za-z]{1,3})\$?([0-9]{1,7})$/.exec(text.trim());
  if (!m || m[1] === undefined || m[2] === undefined) return null;
  let col = 0;
  for (const ch of m[1].toUpperCase()) col = col * 26 + (ch.charCodeAt(0) - 64);
  col -= 1;
  const row = Number(m[2]) - 1;
  if (col >= MAX_COLS || row < 0 || row >= MAX_ROWS) return null;
  return { r: row, c: col };
}

/** Parses `A1` or `A1:C5` into a normalised range. */
export function parseRange(text: string): RangeDto | null {
  const parts = text.split(":");
  if (parts.length > 2) return null;
  const a = parseA1(parts[0] ?? "");
  const b = parts.length === 2 ? parseA1(parts[1] ?? "") : a;
  if (!a || !b) return null;
  return {
    r0: Math.min(a.r, b.r),
    c0: Math.min(a.c, b.c),
    r1: Math.max(a.r, b.r),
    c1: Math.max(a.c, b.c),
  };
}

function cellCount(r: RangeDto): number {
  return (r.r1 - r.r0 + 1) * (r.c1 - r.c0 + 1);
}

function inRange(r: RangeDto, row: number, col: number): boolean {
  return row >= r.r0 && row <= r.r1 && col >= r.c0 && col <= r.c1;
}

function validRange(r: RangeDto): boolean {
  return r.r0 <= r.r1 && r.c0 <= r.c1 && r.r1 < MAX_ROWS && r.c1 < MAX_COLS;
}

// ---------------------------------------------------------------------------
// Values and the tiny evaluator
// ---------------------------------------------------------------------------

const NUMBER_RE = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;

/** Recognises a non-formula input. */
function recognize(input: string): Scalar {
  if (input === "") return null;
  const t = input.trim();
  if (NUMBER_RE.test(t)) return Number(t);
  const upper = t.toUpperCase();
  if (upper === "TRUE") return true;
  if (upper === "FALSE") return false;
  return input;
}

function isError(v: Scalar): v is { error: string } {
  return typeof v === "object" && v !== null;
}

function formatNumber(n: number): string {
  if (!Number.isFinite(n)) return "#NUM!";
  return String(Number(n.toPrecision(15)));
}

function displayText(v: Scalar): string {
  if (v === null) return "";
  if (typeof v === "number") return formatNumber(v);
  if (typeof v === "boolean") return v ? "TRUE" : "FALSE";
  if (typeof v === "string") return v;
  return v.error;
}

function kindOf(v: Scalar): ValueKind {
  if (v === null) return "empty";
  if (typeof v === "number") return "number";
  if (typeof v === "boolean") return "bool";
  if (typeof v === "string") return "text";
  return "error";
}

const OPERAND = String.raw`(\$?[A-Za-z]{1,3}\$?\d{1,7}|[+-]?(?:\d+\.?\d*|\.\d+))`;
const BINARY_RE = new RegExp(String.raw`^${OPERAND}\s*([-+*/])\s*${OPERAND}$`);
const SINGLE_RE = new RegExp(String.raw`^${OPERAND}$`);
const SUM_RE = /^SUM\(\s*([$A-Za-z0-9]+(?::[$A-Za-z0-9]+)?)\s*\)$/i;

class Evaluator {
  private readonly visiting = new Set<string>();

  constructor(private readonly sheet: MockSheet) {}

  value(r: number, c: number): Scalar {
    const cell = this.sheet.cells.get(key(r, c));
    if (!cell || cell.input === "") return null;
    if (!cell.input.startsWith("=") || cell.input.length === 1) return recognize(cell.input);
    const k = key(r, c);
    if (this.visiting.has(k)) return { error: "Err:522" };
    this.visiting.add(k);
    try {
      return this.formula(cell.input.slice(1).trim());
    } finally {
      this.visiting.delete(k);
    }
  }

  private operand(text: string): Scalar {
    const pos = parseA1(text);
    if (pos) return this.value(pos.r, pos.c);
    if (NUMBER_RE.test(text)) return Number(text);
    return { error: "#NAME?" };
  }

  private toNumber(v: Scalar): number | { error: string } {
    if (v === null) return 0;
    if (typeof v === "number") return v;
    if (typeof v === "boolean") return v ? 1 : 0;
    if (typeof v === "string") return NUMBER_RE.test(v.trim()) ? Number(v.trim()) : { error: "#VALUE!" };
    return v;
  }

  private formula(src: string): Scalar {
    const sum = SUM_RE.exec(src);
    if (sum?.[1] !== undefined) {
      const range = parseRange(sum[1]);
      if (!range) return { error: "#NAME?" };
      let total = 0;
      for (const [k, cell] of this.sheet.cells) {
        if (cell.input === "") continue;
        const [r, c] = parseKey(k);
        if (!inRange(range, r, c)) continue;
        const v = this.value(r, c);
        if (isError(v)) return v;
        if (typeof v === "number") total += v; // text and bools in ranges are skipped
      }
      return total;
    }
    const single = SINGLE_RE.exec(src);
    if (single?.[1] !== undefined) return this.operand(single[1]);
    const bin = BINARY_RE.exec(src);
    if (bin?.[1] !== undefined && bin[2] !== undefined && bin[3] !== undefined) {
      const a = this.toNumber(this.operand(bin[1]));
      if (typeof a !== "number") return a;
      const b = this.toNumber(this.operand(bin[3]));
      if (typeof b !== "number") return b;
      switch (bin[2]) {
        case "+":
          return a + b;
        case "-":
          return a - b;
        case "*":
          return a * b;
        default:
          return b === 0 ? { error: "#DIV/0!" } : a / b;
      }
    }
    return { error: "#NAME?" };
  }
}

/** Plain-English hint for the supported formula shapes. */
function hintFor(input: string): string | null {
  if (!input.startsWith("=")) return null;
  const src = input.slice(1).trim();
  const sum = SUM_RE.exec(src);
  if (sum?.[1] !== undefined) return `Sum of ${sum[1].toUpperCase()}`;
  const bin = BINARY_RE.exec(src);
  if (bin?.[1] !== undefined && bin[2] !== undefined && bin[3] !== undefined) {
    const sym: Record<string, string> = { "+": "+", "-": "−", "*": "×", "/": "÷" };
    return `${bin[1].toUpperCase()} ${sym[bin[2]] ?? bin[2]} ${bin[3].toUpperCase()}`;
  }
  return null;
}

// ---------------------------------------------------------------------------
// MockClient
// ---------------------------------------------------------------------------

/** In-memory {@link EngineClient}. Each instance is an isolated "engine". */
/** Currency format the mock applies to template money columns. */
const MOCK_CURRENCY = "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00";

export class MockClient implements EngineClientV4, ShellClient {
  private readonly docs = new Map<DocId, MockDoc>();
  private nextDocId = 1;
  private nextClipToken = 1;
  private readonly clipboard = new Map<string, string[][]>();
  private readonly docListeners = new Set<(e: DocChangedEvent) => void>();
  private readonly menuListeners = new Set<(e: MenuActionEvent) => void>();

  // -- documents ------------------------------------------------------------

  newWorkbook(template?: string | null): Promise<WorkbookInfo> {
    const tpl = template ? mockTemplate(template) : undefined;
    if (template && !tpl) return Promise.reject(apiError("not_found", `no template "${template}"`));
    const id = this.nextDocId++;
    const doc: MockDoc = {
      id,
      title: tpl ? tpl.title : `Untitled ${id}`,
      path: null,
      dirty: false,
      version: 0,
      state: { sheets: [newSheet(1, tpl ? tpl.sheet : "Sheet1")], activeSheet: 1, styles: [defaultStyle()], nextSheetId: 2 },
      v3: newV3(),
      undo: [],
      redo: [],
    };
    if (tpl) this.fillTemplate(doc, tpl.rows, tpl.currencyCols, tpl.widths);
    this.docs.set(id, doc);
    return Promise.resolve(this.info(doc));
  }

  /** Writes a template's cells and styles (not an undo step). */
  private fillTemplate(d: MockDoc, rows: string[][], currencyCols: number[], widths: number[]): void {
    const t = new Touched();
    const width = Math.max(0, ...rows.map((r) => r.length));
    const last = rows.length - 1;
    this.applyOp(d, { type: "setRangeInputs", sheet: 1, r: 0, c: 0, rows }, t);
    if (width > 0) {
      this.applyOp(d, { type: "applyStyle", sheet: 1, ranges: [{ r0: 0, c0: 0, r1: 0, c1: width - 1 }], patch: { ...emptyPatch(), bold: true } }, t);
    }
    if (last >= 1 && currencyCols.length > 0) {
      const ranges = currencyCols.map((c) => ({ r0: 1, c0: c, r1: last, c1: c }));
      this.applyOp(d, { type: "setNumberFormat", sheet: 1, ranges, code: MOCK_CURRENCY }, t);
    }
    widths.forEach((px, c) => this.applyOp(d, { type: "setColWidth", sheet: 1, start: c, end: c, px }, t));
  }

  open(path: string): Promise<WorkbookInfo> {
    return Promise.reject(apiError("unsupported", `MockClient cannot open files (${path})`));
  }

  save(doc: DocId, path: string | null, _format: FileFormat): Promise<WorkbookInfo> {
    return this.run(() => {
      const d = this.doc(doc);
      const target = path ?? d.path;
      if (target === null) throw apiError("invalid_argument", "document has no path; pass one");
      d.path = target;
      d.title = target.split(/[\\/]/).pop() ?? target;
      d.dirty = false;
      return this.info(d);
    });
  }

  close(doc: DocId): Promise<void> {
    return this.run(() => {
      this.doc(doc);
      this.docs.delete(doc);
    });
  }

  // -- shell (ShellClient) --------------------------------------------------

  /** The browser has no native dialog. */
  openDialog(): Promise<WorkbookInfo | null> {
    return Promise.reject(apiError("unsupported", "Opening files needs the desktop app."));
  }

  /** "Saves" in memory under the document title with the format's extension. */
  saveWithDialog(doc: DocId, saveAs: boolean, format: FileFormat | null): Promise<WorkbookInfo> {
    const d = this.docs.get(doc);
    if (!d) return Promise.reject(apiError("not_found", `no document ${doc}`));
    const fmt = format ?? "ods";
    if (d.path !== null && !saveAs && format === null) return this.save(doc, null, fmt);
    const title = d.title;
    const name = /\.[a-z]+$/i.test(title) ? title.replace(/\.[a-z]+$/i, `.${fmt}`) : `${title}.${fmt}`;
    return this.save(doc, name, fmt);
  }

  recentFiles(): Promise<string[]> {
    return Promise.resolve([]);
  }

  listTemplates(): Promise<TemplateInfo[]> {
    return Promise.resolve(BUILTIN_TEMPLATES.map((t) => ({ ...t })));
  }

  // -- queries --------------------------------------------------------------

  sheetLayout(doc: DocId, sheet: SheetId): Promise<SheetLayout> {
    return this.run(() => {
      const sh = this.sheet(this.doc(doc), sheet);
      return {
        sheet,
        used: usedRange(sh),
        defaultColWidthPx: DEFAULT_COL_WIDTH_PX,
        defaultRowHeightPx: DEFAULT_ROW_HEIGHT_PX,
        colRuns: toRuns(sh.colWidths),
        rowRuns: toRuns(sh.rowHeights),
        frozen: null,
        merges: [],
      };
    });
  }

  cells(doc: DocId, sheet: SheetId, ranges: RangeDto[]): Promise<CellBlock[]> {
    return this.run(() => {
      const d = this.doc(doc);
      const sh = this.sheet(d, sheet);
      const ev = new Evaluator(sh);
      return ranges.map((range) => {
        const cells: CellView[] = [];
        for (const [k, cell] of sh.cells) {
          const [r, c] = parseKey(k);
          if (!inRange(range, r, c)) continue;
          cells.push(viewOf(r, c, cell, ev));
        }
        // Notes (flag 8) and links (flag 32), adding views for empty cells.
        const decorate = (k: string, flag: number, link?: string) => {
          const [r, c] = parseKey(k);
          if (!inRange(range, r, c)) return;
          let v = cells.find((x) => x.r === r && x.c === c);
          if (!v) {
            v = { r, c, text: "", kind: "empty", num: null, style: 0, color: null, flags: 0 };
            cells.push(v);
          }
          v.flags |= flag;
          if (link !== undefined) v.link = link;
        };
        for (const k of d.v3.notes.get(sheet)?.keys() ?? []) decorate(k, 8);
        for (const [k, url] of d.v3.links.get(sheet) ?? []) decorate(k, 32, url);
        cells.sort((a, b) => a.r - b.r || a.c - b.c);
        return { sheet, range, version: d.version, cells };
      });
    });
  }

  styles(doc: DocId, ids: number[]): Promise<StyleDto[]> {
    return this.run(() => {
      const st = this.doc(doc).state.styles;
      return ids.flatMap((id) => {
        const s = st[id];
        return s ? [structuredClone(s)] : [];
      });
    });
  }

  cellEditInfo(doc: DocId, sheet: SheetId, r: number, c: number): Promise<CellEditInfo> {
    return this.run(() => {
      const input = this.sheet(this.doc(doc), sheet).cells.get(key(r, c))?.input ?? "";
      return { input, hint: hintFor(input) };
    });
  }

  selectionStats(doc: DocId, sel: SelectionDto): Promise<SelectionStats> {
    return this.run(() => {
      const sh = this.sheet(this.doc(doc), sel.sheet);
      const ev = new Evaluator(sh);
      let count = 0;
      let countNumbers = 0;
      let sum = 0;
      let min = Infinity;
      let max = -Infinity;
      for (const [k, cell] of sh.cells) {
        if (cell.input === "") continue;
        const [r, c] = parseKey(k);
        if (!sel.ranges.some((rg) => inRange(rg, r, c))) continue;
        count++;
        const v = ev.value(r, c);
        if (typeof v === "number") {
          countNumbers++;
          sum += v;
          min = Math.min(min, v);
          max = Math.max(max, v);
        }
      }
      const has = countNumbers > 0;
      return {
        sum: has ? sum : null,
        count,
        countNumbers,
        average: has ? sum / countNumbers : null,
        min: has ? min : null,
        max: has ? max : null,
        formattedSum: has ? formatNumber(sum) : null,
      };
    });
  }

  navigateEdge(doc: DocId, sheet: SheetId, from: CellPos, dir: Direction): Promise<CellPos> {
    return this.run(() => navigate(this.sheet(this.doc(doc), sheet), from, dir));
  }

  resolveNameBox(doc: DocId, text: string): Promise<SelectionDto> {
    return this.run(() => {
      const d = this.doc(doc);
      let sheetId = d.state.activeSheet;
      let ref = text.trim();
      const sep = Math.max(ref.lastIndexOf("."), ref.lastIndexOf("!"));
      if (sep > 0) {
        const name = ref.slice(0, sep).replace(/^\$/, "").replace(/^'(.*)'$/, "$1");
        const sh = d.state.sheets.find((s) => s.name.toLowerCase() === name.toLowerCase());
        if (!sh) throw apiError("not_found", `no sheet named ${name}`);
        sheetId = sh.id;
        ref = ref.slice(sep + 1);
      }
      const range = parseRange(ref);
      if (!range) throw apiError("invalid_argument", `cannot resolve "${text}"`);
      return { sheet: sheetId, ranges: [range], active: { r: range.r0, c: range.c0 } };
    });
  }

  listFunctions(): Promise<FunctionInfo[]> {
    return Promise.resolve([
      {
        name: "SUM",
        category: "Mathematical",
        signature: "SUM(number1, [number2], …)",
        description: "Adds all the numbers in a range of cells.",
      },
    ]);
  }

  assistSuggest(doc: DocId, _query: string, _sel: SelectionDto): Promise<Suggestion[]> {
    return this.run(() => {
      this.doc(doc);
      return [];
    });
  }

  // -- v0.2 stubs (the mock has no names, filters or search) -----------------

  names(doc: DocId): Promise<NameInfo[]> {
    return this.run(() => {
      this.doc(doc);
      return [];
    });
  }

  filterInfo(doc: DocId, sheet: SheetId): Promise<AutoFilterInfo> {
    return this.run(() => {
      this.sheet(this.doc(doc), sheet);
      return { range: null, columns: [] };
    });
  }

  find(doc: DocId, _query: FindQuery): Promise<FindResult> {
    return this.run(() => {
      this.doc(doc);
      return { hits: [], truncated: false };
    });
  }

  detectHeader(doc: DocId, sheet: SheetId, _range: RangeDto): Promise<boolean> {
    return this.run(() => {
      this.sheet(this.doc(doc), sheet);
      return false;
    });
  }

  recalcAll(doc: DocId): Promise<ApplyResult> {
    return this.run(() => {
      const d = this.doc(doc);
      return this.result(d, new Touched(), [], null);
    });
  }

  // -- v0.3 stubs (stored, not evaluated) ------------------------------------

  conditionalFormats(doc: DocId, sheet: SheetId): Promise<ConditionalFormatDto[]> {
    return this.run(() => {
      const d = this.doc(doc);
      this.sheet(d, sheet);
      return [...(d.v3.cfs.get(sheet) ?? [])];
    });
  }

  validations(doc: DocId, sheet: SheetId): Promise<ValidationDto[]> {
    return this.run(() => {
      const d = this.doc(doc);
      this.sheet(d, sheet);
      return [...(d.v3.validations.get(sheet) ?? [])];
    });
  }

  validationAt(doc: DocId, sheet: SheetId, r: number, c: number): Promise<ValidationInfo | null> {
    return this.run(() => {
      const d = this.doc(doc);
      this.sheet(d, sheet);
      const v = (d.v3.validations.get(sheet) ?? []).find((x) => x.ranges.some((rg) => inRange(rg, r, c)));
      if (!v) return null;
      const items = v.kind.type === "list" && v.kind.source.type === "values" ? v.kind.source.items : [];
      return { id: v.id, listItems: items, showDropdown: v.showDropdown && v.kind.type === "list", inputHelp: v.inputHelp, error: v.error, kind: v.kind };
    });
  }

  note(doc: DocId, sheet: SheetId, r: number, c: number): Promise<NoteDto | null> {
    return this.run(() => {
      const d = this.doc(doc);
      this.sheet(d, sheet);
      return d.v3.notes.get(sheet)?.get(key(r, c)) ?? null;
    });
  }

  notes(doc: DocId, sheet: SheetId): Promise<NoteDto[]> {
    return this.run(() => {
      const d = this.doc(doc);
      this.sheet(d, sheet);
      return [...(d.v3.notes.get(sheet)?.values() ?? [])].sort((a, b) => a.r - b.r || a.c - b.c);
    });
  }

  protectionInfo(doc: DocId, sheet: SheetId): Promise<ProtectionInfo> {
    return this.run(() => {
      const d = this.doc(doc);
      this.sheet(d, sheet);
      return {
        sheetProtected: d.v3.protected.get(sheet) ?? false,
        sheetHasPassword: false,
        options: {
          selectLocked: true,
          selectUnlocked: true,
          insertRows: false,
          insertCols: false,
          deleteRows: false,
          deleteCols: false,
          formatCells: false,
          sort: false,
          autoFilter: false,
        },
        structureProtected: d.v3.structureProtected,
        structureHasPassword: false,
      };
    });
  }

  pageSetup(doc: DocId, sheet: SheetId): Promise<PageSetupDto> {
    return this.run(() => {
      const d = this.doc(doc);
      this.sheet(d, sheet);
      return d.v3.pages.get(sheet) ?? defaultPageSetup();
    });
  }

  documentProperties(doc: DocId): Promise<DocPropertiesDto> {
    return this.run(() => ({ ...this.doc(doc).v3.props }));
  }

  goalSeek(doc: DocId, _req: GoalSeekRequest): Promise<GoalSeekResult> {
    return this.run(() => {
      this.doc(doc);
      throw apiError("unsupported", "MockClient does not support goal seek");
    });
  }

  listLocales(): Promise<LocaleInfo[]> {
    return Promise.resolve([{ tag: "en-US", decimalSep: ".", groupSep: ",", listSep: ",", currencySymbol: "$" }]);
  }

  setLocale(doc: DocId, locale: string): Promise<ApplyResult> {
    return this.run(() => {
      const d = this.doc(doc);
      if (locale !== "en-US") throw apiError("invalid_argument", `unknown locale "${locale}"`);
      return this.result(d, new Touched(), [], null);
    });
  }

  resolveNameBoxIn(doc: DocId, sheet: SheetId, text: string): Promise<SelectionDto> {
    const d = this.docs.get(doc);
    if (d && d.state.sheets.some((s) => s.id === sheet)) {
      const prev = d.state.activeSheet;
      d.state.activeSheet = sheet;
      return this.resolveNameBox(doc, text).finally(() => {
        d.state.activeSheet = prev;
      });
    }
    return this.resolveNameBox(doc, text);
  }

  // -- v0.4–v0.6 stubs (wave 5) ----------------------------------------------
  // List queries answer empty lists so panels render; the rest is
  // `unsupported` until the engine work packages land (the browser build's
  // `WebMockClient` adds emulation where the e2e flows need it).

  charts(doc: DocId, sheet: SheetId): Promise<ChartDto[]> {
    return this.run(() => {
      this.sheet(this.doc(doc), sheet);
      return [];
    });
  }

  chartData(doc: DocId, sheet: SheetId, id: number): Promise<ChartDataDto> {
    return this.unsupportedV4(doc, `chart ${id} on sheet ${sheet}`);
  }

  suggestChart(doc: DocId, sheet: SheetId, _sel: RangeDto, kind: ChartKind): Promise<ChartDto> {
    return this.unsupportedV4(doc, `${kind} chart suggestions on sheet ${sheet}`);
  }

  images(doc: DocId, sheet: SheetId): Promise<ImageDto[]> {
    return this.run(() => {
      this.sheet(this.doc(doc), sheet);
      return [];
    });
  }

  pivots(doc: DocId): Promise<PivotDto[]> {
    return this.run(() => {
      this.doc(doc);
      return [];
    });
  }

  pivotFieldItems(doc: DocId, _source: PivotSource): Promise<PivotFieldItemsDto[]> {
    return this.unsupportedV4(doc, "pivot tables");
  }

  pivotAt(doc: DocId, sheet: SheetId, _r: number, _c: number): Promise<PivotHit | null> {
    return this.run(() => {
      this.sheet(this.doc(doc), sheet);
      return null;
    });
  }

  autocomplete(doc: DocId, sheet: SheetId, _r: number, _c: number, _prefix: string): Promise<AutocompleteDto> {
    return this.run(() => {
      this.sheet(this.doc(doc), sheet);
      return { completion: null, items: [] };
    });
  }

  cellStyles(doc: DocId): Promise<CellStyleDto[]> {
    return this.run(() => {
      const style = this.doc(doc).state.styles[0];
      return style ? [{ name: "Default", parent: null, builtin: true, style }] : [];
    });
  }

  scenarios(doc: DocId, sheet: SheetId): Promise<ScenarioDto[]> {
    return this.run(() => {
      this.sheet(this.doc(doc), sheet);
      return [];
    });
  }

  databaseRanges(doc: DocId): Promise<DatabaseRangeDto[]> {
    return this.run(() => {
      this.doc(doc);
      return [];
    });
  }

  navigator(doc: DocId): Promise<NavigatorDto> {
    return this.run(() => {
      const d = this.doc(doc);
      const sheets = d.state.sheets.map((s) => ({ title: s.name, sheet: s.id, range: null, id: null }));
      const notes = d.state.sheets.flatMap((s) =>
        [...(d.v3.notes.get(s.id)?.values() ?? [])].map((n) => ({
          title: n.text,
          sheet: s.id,
          range: { r0: n.r, c0: n.c, r1: n.r, c1: n.c },
          id: null,
        })),
      );
      return { sheets, names: [], databaseRanges: [], charts: [], images: [], pivots: [], notes, scenarios: [] };
    });
  }

  documentStatistics(doc: DocId): Promise<DocumentStatsDto> {
    return this.run(() => {
      const d = this.doc(doc);
      let cells = 0;
      let formulas = 0;
      for (const s of d.state.sheets) {
        for (const cell of s.cells.values()) {
          if (cell.input === "") continue;
          cells += 1;
          if (cell.input.startsWith("=")) formulas += 1;
        }
      }
      const notes = d.state.sheets.reduce((n, s) => n + (d.v3.notes.get(s.id)?.size ?? 0), 0);
      return { sheets: d.state.sheets.length, cells, formulas, pages: null, charts: 0, notes, names: 0 };
    });
  }

  printLayout(doc: DocId, _opts: PrintOptionsDto): Promise<PrintLayoutDto> {
    return this.unsupportedV4(doc, "printing");
  }

  pageBreaks(doc: DocId, sheet: SheetId): Promise<PageBreaksDto> {
    return this.unsupportedV4(doc, `page breaks on sheet ${sheet}`);
  }

  preparePrintJob(doc: DocId, _opts: PrintOptionsDto, _charts: ChartSvgDto[]): Promise<PrintJobInfo> {
    return this.unsupportedV4(doc, "printing");
  }

  printJobPagePng(_job: number, _page: number, _dpi: number): Promise<string> {
    return Promise.reject(apiError("unsupported", "MockClient does not support printing"));
  }

  printJobPdf(_job: number): Promise<Uint8Array> {
    return Promise.reject(apiError("unsupported", "MockClient does not support PDF export"));
  }

  calcSettings(doc: DocId): Promise<CalcSettingsDto> {
    return this.run(() => {
      this.doc(doc);
      return {
        iterations: null,
        dateEpoch: "d1899",
        caseSensitive: false,
        precisionAsShown: false,
        wholeCellMatch: true,
        criteriaMode: "wildcards",
        textToNumber: "convertUnambiguous",
      };
    });
  }

  solver(doc: DocId, _req: SolverRequest): Promise<SolverResult> {
    return this.unsupportedV4(doc, "the solver");
  }

  precedents(doc: DocId, sheet: SheetId, _r: number, _c: number): Promise<TraceDto[]> {
    return this.run(() => {
      this.sheet(this.doc(doc), sheet);
      return [];
    });
  }

  dependents(doc: DocId, sheet: SheetId, _r: number, _c: number): Promise<TraceDto[]> {
    return this.run(() => {
      this.sheet(this.doc(doc), sheet);
      return [];
    });
  }

  private unsupportedV4<T>(doc: DocId, what: string): Promise<T> {
    return this.run(() => {
      this.doc(doc);
      throw apiError("unsupported", `MockClient does not support ${what}`);
    });
  }

  copy(doc: DocId, sel: SelectionDto): Promise<ClipboardOut> {
    return this.run(() => {
      const sh = this.sheet(this.doc(doc), sel.sheet);
      const range = sel.ranges[0];
      if (!range || !validRange(range)) throw apiError("invalid_argument", "empty selection");
      if (cellCount(range) > MAX_CELLS_PER_OP) throw apiError("invalid_argument", "selection too large");
      const ev = new Evaluator(sh);
      const inputs: string[][] = [];
      const texts: string[][] = [];
      for (let r = range.r0; r <= range.r1; r++) {
        const inRow: string[] = [];
        const textRow: string[] = [];
        for (let c = range.c0; c <= range.c1; c++) {
          inRow.push(sh.cells.get(key(r, c))?.input ?? "");
          textRow.push(displayText(ev.value(r, c)));
        }
        inputs.push(inRow);
        texts.push(textRow);
      }
      const token = `mock-${this.nextClipToken++}`;
      this.clipboard.set(token, inputs);
      const tsv = texts.map((row) => row.join("\t")).join("\n");
      const rows = texts.map((row) => `<tr>${row.map((t) => `<td>${escapeHtml(t)}</td>`).join("")}</tr>`);
      const html = `<table data-tally-clip="${token}">${rows.join("")}</table>`;
      return { tsv, html, token };
    });
  }

  // -- mutations ------------------------------------------------------------

  apply(doc: DocId, tx: Transaction): Promise<ApplyResult> {
    return this.run(() => {
      const d = this.doc(doc);
      const before = cloneState(d.state);
      const touched = new Touched();
      const errors: OpError[] = [];
      tx.ops.forEach((op, i) => {
        try {
          this.applyOp(d, op, touched);
        } catch (e) {
          const err = e as Partial<ApiError>;
          errors.push({
            opIndex: i,
            code: err.code ?? "internal",
            message: err.message ?? String(e),
          });
        }
      });
      if (touched.any()) {
        d.undo.push({
          state: before,
          label: tx.label,
          selectionBefore: tx.selectionBefore,
          selectionAfter: tx.selectionAfter,
        });
        if (d.undo.length > UNDO_DEPTH) d.undo.shift();
        d.redo = [];
        d.dirty = true;
        d.version++;
      }
      return this.result(d, touched, errors, tx.selectionAfter);
    });
  }

  undo(doc: DocId): Promise<ApplyResult> {
    return this.run(() => {
      const d = this.doc(doc);
      const entry = d.undo.pop();
      if (!entry) throw apiError("invalid_argument", "nothing to undo");
      d.redo.push({ ...entry, state: d.state });
      return this.restore(d, entry.state, entry.selectionBefore);
    });
  }

  redo(doc: DocId): Promise<ApplyResult> {
    return this.run(() => {
      const d = this.doc(doc);
      const entry = d.redo.pop();
      if (!entry) throw apiError("invalid_argument", "nothing to redo");
      d.undo.push({ ...entry, state: d.state });
      return this.restore(d, entry.state, entry.selectionAfter);
    });
  }

  // -- events ---------------------------------------------------------------

  onDocChanged(cb: (event: DocChangedEvent) => void): Unsubscribe {
    this.docListeners.add(cb);
    return () => this.docListeners.delete(cb);
  }

  onMenuAction(cb: (event: MenuActionEvent) => void): Unsubscribe {
    this.menuListeners.add(cb);
    return () => this.menuListeners.delete(cb);
  }

  /** Test hook: simulates a native menu selection. */
  emitMenuAction(id: string): void {
    for (const cb of this.menuListeners) cb({ id });
  }

  /** Test hook: simulates an engine-initiated change (e.g. volatile recalc). */
  emitDocChanged(event: DocChangedEvent): void {
    for (const cb of this.docListeners) cb(event);
  }

  // -- internals ------------------------------------------------------------

  /** Runs `f`, turning synchronous throws into Promise rejections. */
  private run<T>(f: () => T): Promise<T> {
    try {
      return Promise.resolve(f());
    } catch (e) {
      return Promise.reject(e);
    }
  }

  private doc(id: DocId): MockDoc {
    const d = this.docs.get(id);
    if (!d) throw apiError("not_found", `no document ${id}`);
    return d;
  }

  private sheet(d: MockDoc, id: SheetId): MockSheet {
    const s = d.state.sheets.find((sh) => sh.id === id);
    if (!s) throw apiError("not_found", `no sheet ${id}`);
    return s;
  }

  private info(d: MockDoc): WorkbookInfo {
    return {
      doc: d.id,
      title: d.title,
      path: d.path,
      sheets: d.state.sheets.map((s) => ({ id: s.id, name: s.name, tabColor: null, hidden: false })),
      activeSheet: d.state.activeSheet,
      dirty: d.dirty,
      version: d.version,
      warnings: [],
    };
  }

  private restore(d: MockDoc, state: DocState, selection: SelectionDto | null): ApplyResult {
    d.state = state;
    d.version++;
    d.dirty = true;
    const touched = new Touched();
    touched.sheetsChanged = true;
    for (const sh of state.sheets) {
      touched.layout.add(sh.id);
      const used = usedRange(sh);
      if (used) touched.cell(sh.id, used);
    }
    return this.result(d, touched, [], selection);
  }

  private result(d: MockDoc, t: Touched, errors: OpError[], selection: SelectionDto | null): ApplyResult {
    const last = (stack: UndoEntry[]): string | null => stack[stack.length - 1]?.label ?? null;
    return {
      version: d.version,
      changed: t.changedRanges(d.state),
      layoutChanged: [...t.layout],
      sheetsChanged: t.sheetsChanged,
      errors,
      undo: {
        canUndo: d.undo.length > 0,
        canRedo: d.redo.length > 0,
        undoLabel: last(d.undo),
        redoLabel: last(d.redo),
      },
      selection,
      info: t.sheetsChanged ? this.info(d) : null,
    };
  }

  private applyOp(d: MockDoc, op: Op, t: Touched): void {
    const st = d.state;
    switch (op.type) {
      case "setCellInput": {
        const sh = this.sheet(d, op.sheet);
        checkPos(op.r, op.c);
        setInput(sh, op.r, op.c, op.input);
        t.cell(sh.id, { r0: op.r, c0: op.c, r1: op.r, c1: op.c });
        return;
      }
      case "setRangeInputs": {
        const sh = this.sheet(d, op.sheet);
        this.writeBlock(sh, op.r, op.c, op.rows, t);
        return;
      }
      case "clear": {
        const sh = this.sheet(d, op.sheet);
        for (const range of op.ranges) {
          checkRange(range);
          for (const [k, cell] of [...sh.cells]) {
            const [r, c] = parseKey(k);
            if (!inRange(range, r, c)) continue;
            if (op.what === "all") sh.cells.delete(k);
            else if (op.what === "contents") cell.input = "";
            else cell.style = 0;
            if (cell.input === "" && cell.style === 0) sh.cells.delete(k);
          }
          t.cell(sh.id, range);
        }
        return;
      }
      case "applyStyle":
      case "setNumberFormat": {
        const sh = this.sheet(d, op.sheet);
        const patch: StylePatch =
          op.type === "applyStyle" ? op.patch : { ...emptyPatch(), numberFormat: op.code };
        for (const range of op.ranges) {
          checkRange(range);
          if (cellCount(range) > MAX_CELLS_PER_OP) throw apiError("unsupported", "range too large for MockClient");
          for (let r = range.r0; r <= range.r1; r++) {
            for (let c = range.c0; c <= range.c1; c++) {
              const k = key(r, c);
              const cell = sh.cells.get(k) ?? { input: "", style: 0 };
              cell.style = styleWith(st.styles, cell.style, patch);
              if (cell.input === "" && cell.style === 0) sh.cells.delete(k);
              else sh.cells.set(k, cell);
            }
          }
          t.cell(sh.id, range);
        }
        return;
      }
      case "setColWidth":
      case "setRowHeight": {
        const sh = this.sheet(d, op.sheet);
        if (op.start > op.end || !(op.px >= 0)) throw apiError("invalid_argument", "bad size run");
        const map = op.type === "setColWidth" ? sh.colWidths : sh.rowHeights;
        for (let i = op.start; i <= op.end; i++) map.set(i, op.px);
        t.layout.add(sh.id);
        return;
      }
      case "addSheet": {
        const name = op.name ?? nextSheetName(st.sheets);
        checkSheetName(st.sheets, name, null);
        const sh = newSheet(st.nextSheetId++, name);
        const index = Math.min(op.index ?? st.sheets.length, st.sheets.length);
        st.sheets.splice(index, 0, sh);
        st.activeSheet = sh.id;
        t.sheetsChanged = true;
        return;
      }
      case "renameSheet": {
        const sh = this.sheet(d, op.sheet);
        checkSheetName(st.sheets, op.name, sh.id);
        sh.name = op.name;
        t.sheetsChanged = true;
        return;
      }
      case "deleteSheet": {
        const sh = this.sheet(d, op.sheet);
        if (st.sheets.length === 1) throw apiError("invalid_argument", "cannot delete the last sheet");
        const index = st.sheets.indexOf(sh);
        st.sheets.splice(index, 1);
        if (st.activeSheet === sh.id) {
          const next = st.sheets[Math.min(index, st.sheets.length - 1)];
          if (next) st.activeSheet = next.id;
        }
        t.sheetsChanged = true;
        return;
      }
      case "moveSheet": {
        const sh = this.sheet(d, op.sheet);
        st.sheets.splice(st.sheets.indexOf(sh), 1);
        st.sheets.splice(Math.min(op.to, st.sheets.length), 0, sh);
        t.sheetsChanged = true;
        return;
      }
      case "paste": {
        const sh = this.sheet(d, op.sheet);
        checkRange(op.target);
        let rows: string[][];
        if (op.payload.type === "internal") {
          const snap = this.clipboard.get(op.payload.token);
          if (!snap) throw apiError("not_found", "clipboard token expired");
          rows = snap;
        } else {
          rows = op.payload.text.replace(/\r?\n$/, "").split(/\r?\n/).map((line) => line.split("\t"));
        }
        this.writeBlock(sh, op.target.r0, op.target.c0, rows, t);
        return;
      }
      case "addConditionalFormat":
      case "updateConditionalFormat": {
        this.sheet(d, op.sheet);
        const list = d.v3.cfs.get(op.sheet) ?? [];
        if (op.type === "addConditionalFormat") {
          list.push({ id: d.v3.nextId++, ranges: op.ranges, rules: op.rules });
        } else {
          const i = list.findIndex((x) => x.id === op.id);
          if (i < 0) throw apiError("not_found", `no conditional format ${op.id}`);
          list[i] = { id: op.id, ranges: op.ranges, rules: op.rules };
        }
        d.v3.cfs.set(op.sheet, list);
        t.layout.add(op.sheet);
        return;
      }
      case "deleteConditionalFormat": {
        const list = d.v3.cfs.get(op.sheet) ?? [];
        if (!list.some((x) => x.id === op.id)) throw apiError("not_found", `no conditional format ${op.id}`);
        d.v3.cfs.set(op.sheet, list.filter((x) => x.id !== op.id));
        t.layout.add(op.sheet);
        return;
      }
      case "addValidation":
      case "updateValidation": {
        this.sheet(d, op.sheet);
        const list = d.v3.validations.get(op.sheet) ?? [];
        if (op.type === "addValidation") {
          list.push({ ...op.validation, id: d.v3.nextId++ });
        } else {
          const i = list.findIndex((x) => x.id === op.id);
          if (i < 0) throw apiError("not_found", `no validation ${op.id}`);
          list[i] = { ...op.validation, id: op.id };
        }
        d.v3.validations.set(op.sheet, list);
        t.layout.add(op.sheet);
        return;
      }
      case "deleteValidation": {
        const list = d.v3.validations.get(op.sheet) ?? [];
        if (!list.some((x) => x.id === op.id)) throw apiError("not_found", `no validation ${op.id}`);
        d.v3.validations.set(op.sheet, list.filter((x) => x.id !== op.id));
        t.layout.add(op.sheet);
        return;
      }
      case "setNote": {
        this.sheet(d, op.sheet);
        checkPos(op.r, op.c);
        const m = d.v3.notes.get(op.sheet) ?? new Map<string, NoteDto>();
        m.set(key(op.r, op.c), { ...op.note, r: op.r, c: op.c, created: op.note.created ?? new Date().toISOString().slice(0, 19) });
        d.v3.notes.set(op.sheet, m);
        t.cell(op.sheet, { r0: op.r, c0: op.c, r1: op.r, c1: op.c });
        return;
      }
      case "deleteNote": {
        const m = d.v3.notes.get(op.sheet);
        if (!m?.delete(key(op.r, op.c))) throw apiError("not_found", "the cell has no note");
        t.cell(op.sheet, { r0: op.r, c0: op.c, r1: op.r, c1: op.c });
        return;
      }
      case "setLink": {
        this.sheet(d, op.sheet);
        checkPos(op.r, op.c);
        if (/^\s*(javascript|data|vbscript):/i.test(op.url)) throw apiError("invalid_argument", "this kind of link is not allowed");
        const m = d.v3.links.get(op.sheet) ?? new Map<string, string>();
        m.set(key(op.r, op.c), op.url.trim());
        d.v3.links.set(op.sheet, m);
        t.cell(op.sheet, { r0: op.r, c0: op.c, r1: op.r, c1: op.c });
        return;
      }
      case "clearLink": {
        const m = d.v3.links.get(op.sheet);
        if (m) {
          for (const k of [...m.keys()]) {
            const [r, c] = parseKey(k);
            if (op.ranges.some((rg) => inRange(rg, r, c))) m.delete(k);
          }
        }
        for (const rg of op.ranges) t.cell(op.sheet, rg);
        return;
      }
      case "protectSheet":
      case "unprotectSheet":
        this.sheet(d, op.sheet);
        d.v3.protected.set(op.sheet, op.type === "protectSheet");
        t.layout.add(op.sheet);
        return;
      case "protectDocument":
      case "unprotectDocument":
        d.v3.structureProtected = op.type === "protectDocument";
        t.sheetsChanged = true;
        return;
      case "setPageSetup":
        this.sheet(d, op.sheet);
        d.v3.pages.set(op.sheet, { ...op.setup, rowBreaks: d.v3.pages.get(op.sheet)?.rowBreaks ?? [], colBreaks: d.v3.pages.get(op.sheet)?.colBreaks ?? [] });
        t.layout.add(op.sheet);
        return;
      case "setPageBreak": {
        this.sheet(d, op.sheet);
        const p = d.v3.pages.get(op.sheet) ?? defaultPageSetup();
        const list = op.axis === "row" ? p.rowBreaks : p.colBreaks;
        const next = list.filter((b) => b !== op.at);
        if (op.present) next.push(op.at);
        next.sort((a, b) => a - b);
        d.v3.pages.set(op.sheet, op.axis === "row" ? { ...p, rowBreaks: next } : { ...p, colBreaks: next });
        t.layout.add(op.sheet);
        return;
      }
      case "setDocumentProperties":
        d.v3.props = { ...d.v3.props, ...op.props, created: d.v3.props.created, modified: d.v3.props.modified, generator: d.v3.props.generator };
        t.sheetsChanged = true;
        return;
      case "refreshFilter":
      case "setCellProtection":
      case "autoFitRows":
        this.sheet(d, op.sheet);
        t.layout.add(op.sheet);
        return;
      case "insertRows":
      case "deleteRows":
      case "insertCols":
      case "deleteCols":
      case "defineName":
      case "deleteName":
      case "setArrayFormula":
      case "moveRange":
      case "mergeCells":
      case "unmergeCells":
      case "setFrozen":
      case "sortRange":
      case "setAutoFilter":
      case "setColumnFilter":
      case "setRowsHidden":
      case "setColsHidden":
      case "replaceAll":
      case "pasteSpecial":
      case "fillRange":
        throw apiError("unsupported", `MockClient does not support ${op.type}`);
    }
  }

  private writeBlock(sh: MockSheet, r0: number, c0: number, rows: string[][], t: Touched): void {
    const width = Math.max(0, ...rows.map((row) => row.length));
    if (rows.length === 0 || width === 0) return;
    const range = { r0, c0, r1: r0 + rows.length - 1, c1: c0 + width - 1 };
    checkRange(range);
    if (cellCount(range) > MAX_CELLS_PER_OP) throw apiError("unsupported", "block too large for MockClient");
    rows.forEach((row, i) => row.forEach((input, j) => setInput(sh, r0 + i, c0 + j, input)));
    t.cell(sh.id, range);
  }
}

// ---------------------------------------------------------------------------
// Helpers used by MockClient
// ---------------------------------------------------------------------------

/** Collects what an operation touched. */
class Touched {
  readonly cells = new Map<SheetId, RangeDto[]>();
  readonly layout = new Set<SheetId>();
  sheetsChanged = false;

  cell(sheet: SheetId, range: RangeDto): void {
    const list = this.cells.get(sheet) ?? [];
    list.push(range);
    this.cells.set(sheet, list);
  }

  any(): boolean {
    return this.cells.size > 0 || this.layout.size > 0 || this.sheetsChanged;
  }

  /** Touched ranges plus every formula cell (formulas are evaluated on read). */
  changedRanges(state: DocState): SheetRanges[] {
    if (this.cells.size === 0) return [];
    const out: SheetRanges[] = [];
    for (const sh of state.sheets) {
      const ranges = [...(this.cells.get(sh.id) ?? [])];
      for (const [k, cell] of sh.cells) {
        if (!cell.input.startsWith("=")) continue;
        const [r, c] = parseKey(k);
        if (!ranges.some((rg) => inRange(rg, r, c))) ranges.push({ r0: r, c0: c, r1: r, c1: c });
      }
      if (this.cells.has(sh.id) || ranges.length > 0) out.push({ sheet: sh.id, ranges });
    }
    return out.filter((s) => s.ranges.length > 0);
  }
}

function checkPos(r: number, c: number): void {
  if (!Number.isInteger(r) || !Number.isInteger(c) || r < 0 || c < 0 || r >= MAX_ROWS || c >= MAX_COLS) {
    throw apiError("invalid_argument", `cell (${r}, ${c}) is outside the grid`);
  }
}

function checkRange(r: RangeDto): void {
  if (!validRange(r) || r.r0 < 0 || r.c0 < 0) throw apiError("invalid_argument", "range is outside the grid");
}

function setInput(sh: MockSheet, r: number, c: number, input: string): void {
  const k = key(r, c);
  const cell = sh.cells.get(k);
  if (cell) {
    cell.input = input;
    if (input === "" && cell.style === 0) sh.cells.delete(k);
  } else if (input !== "") {
    sh.cells.set(k, { input, style: 0 });
  }
}

function viewOf(r: number, c: number, cell: MockCell, ev: Evaluator): CellView {
  const v = ev.value(r, c);
  const formula = cell.input.startsWith("=") && cell.input.length > 1;
  const flags = (formula ? 1 : 0) | (isError(v) ? 2 : 0);
  return {
    r,
    c,
    text: displayText(v),
    kind: kindOf(v),
    num: typeof v === "number" ? v : null,
    style: cell.style,
    color: null,
    flags,
  };
}

function usedRange(sh: MockSheet): RangeDto | null {
  let used: RangeDto | null = null;
  for (const [k, cell] of sh.cells) {
    if (cell.input === "") continue;
    const [r, c] = parseKey(k);
    used = used
      ? { r0: Math.min(used.r0, r), c0: Math.min(used.c0, c), r1: Math.max(used.r1, r), c1: Math.max(used.c1, c) }
      : { r0: r, c0: c, r1: r, c1: c };
  }
  return used;
}

/** Coalesces per-index sizes into sorted runs. */
function toRuns(sizes: Map<number, number>): DimRunPx[] {
  const runs: DimRunPx[] = [];
  for (const i of [...sizes.keys()].sort((a, b) => a - b)) {
    const size = sizes.get(i) ?? 0;
    const last = runs[runs.length - 1];
    if (last && last.end === i - 1 && last.size === size) last.end = i;
    else runs.push({ start: i, end: i, size, hidden: size === 0 });
  }
  return runs;
}

function emptyPatch(): StylePatch {
  return {
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
}

/** Returns the id of `base` with `patch` applied, adding a style if new. */
function styleWith(styles: StyleDto[], baseId: number, patch: StylePatch): number {
  const base = styles[baseId] ?? defaultStyle();
  const next: StyleDto = { ...structuredClone(base), id: 0 };
  for (const [k, v] of Object.entries(patch) as [keyof StylePatch, unknown][]) {
    if (v !== null && v !== undefined) (next as unknown as Record<string, unknown>)[k] = v;
  }
  const sig = JSON.stringify(next);
  const found = styles.findIndex((s) => JSON.stringify({ ...s, id: 0 }) === sig);
  if (found >= 0) return found;
  next.id = styles.length;
  styles.push(next);
  return next.id;
}

function nextSheetName(sheets: MockSheet[]): string {
  const taken = new Set(sheets.map((s) => s.name.toLowerCase()));
  for (let n = sheets.length + 1; ; n++) {
    const name = `Sheet${n}`;
    if (!taken.has(name.toLowerCase())) return name;
  }
}

function checkSheetName(sheets: MockSheet[], name: string, self: SheetId | null): void {
  if (name.trim() === "" || /[[\]*?:/\\]/.test(name)) {
    throw apiError("invalid_argument", `invalid sheet name "${name}"`);
  }
  if (sheets.some((s) => s.id !== self && s.name.toLowerCase() === name.toLowerCase())) {
    throw apiError("invalid_argument", `a sheet named "${name}" already exists`);
  }
}

/** Ctrl+arrow: jump to the edge of the current data block or the next one. */
function navigate(sh: MockSheet, from: CellPos, dir: Direction): CellPos {
  const steps: Record<Direction, readonly [number, number]> = {
    up: [-1, 0],
    down: [1, 0],
    left: [0, -1],
    right: [0, 1],
  };
  const [dr, dc] = steps[dir];
  const filled = (r: number, c: number): boolean => (sh.cells.get(key(r, c))?.input ?? "") !== "";
  const inside = (r: number, c: number): boolean => r >= 0 && c >= 0 && r < MAX_ROWS && c < MAX_COLS;
  let r = from.r;
  let c = from.c;
  if (!inside(r + dr, c + dc)) return { r, c };
  if (filled(r, c) && filled(r + dr, c + dc)) {
    // Inside a block: go to its last filled cell.
    while (inside(r + dr, c + dc) && filled(r + dr, c + dc)) {
      r += dr;
      c += dc;
    }
    return { r, c };
  }
  // Otherwise: go to the next filled cell, or the grid edge.
  do {
    r += dr;
    c += dc;
  } while (inside(r + dr, c + dc) && !filled(r, c));
  return { r, c };
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
