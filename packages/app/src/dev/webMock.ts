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
      d = { sheets: {}, names: [] };
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

  // -- queries ---------------------------------------------------------------------

  override async sheetLayout(doc: DocId, sheet: number): Promise<SheetLayout> {
    const base = await super.sheetLayout(doc, sheet);
    const s = this.ext.get(doc)?.sheets[sheet];
    if (!s) return base;
    const hidden = new Set(s.hiddenRows);
    const filtered = new Set(s.filteredRows);
    return {
      ...base,
      merges: s.merges.map((m) => ({ ...m })),
      frozen: s.frozen ? { ...s.frozen } : null,
      rowRuns: overlayRuns(base.rowRuns, base.defaultRowHeightPx, hidden, filtered),
      colRuns: overlayRuns(base.colRuns, base.defaultColWidthPx, new Set(s.hiddenCols), new Set()),
    };
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
    return info;
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
    if (tx.ops.some(isStructural)) {
      // One step per op (later ops see the shifted cells).
      let last: ApplyResult | null = null;
      for (const op of tx.ops) {
        const before = JSON.stringify(this.docExt(doc));
        const ops = isStructural(op) ? await this.rewrite(doc, op) : await this.rewriteV02(doc, op, [], new Set());
        last = this.track(doc, await super.apply(doc, { ...tx, ops }), before);
      }
      return last as ApplyResult;
    }
    const before = JSON.stringify(this.docExt(doc));
    const errors: OpError[] = [];
    const layout = new Set<number>();
    const ops: Op[] = [];
    for (const [i, op] of tx.ops.entries()) {
      const errs: OpError[] = [];
      ops.push(...(await this.rewriteV02(doc, op, errs, layout)));
      errors.push(...errs.map((e) => ({ ...e, opIndex: i })));
    }
    const changedExt = JSON.stringify(this.docExt(doc)) !== before;
    // A side-model-only change still needs an undo step in the mock: a no-op style touch.
    if (changedExt && ops.length === 0) {
      const sheet = [...layout][0] ?? this.firstSheet(doc);
      ops.push({ type: "applyStyle", sheet, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], patch: EMPTY_PATCH });
    }
    if (ops.length === 0) {
      const res = await super.apply(doc, { ...tx, ops: [] });
      return this.track(doc, { ...res, errors: [...errors, ...res.errors] });
    }
    const res = await super.apply(doc, { ...tx, ops });
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
    if (res.info) this.sheetOrder.set(doc, res.info.sheets.map((s) => s.id));
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
