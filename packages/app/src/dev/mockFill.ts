/**
 * AutoFill emulation for the browser demo engine (`WebMockClient`), which
 * does not implement the `fillRange` op. The desktop app sends `fillRange`
 * to the real engine (../fill.ts); this file only keeps the browser build
 * and its e2e tests behaving like Calc's AutoFill, as ONE transaction:
 *   1. `client.copy(source)` + `Paste{Internal{token}}` into each block of
 *      the extension, so formats travel with the values;
 *   2. per line (a column when filling up/down, a row when filling
 *      left/right) whose source is a recognised series, `SetRangeInputs`
 *      overwrites the pasted copies with the continued series:
 *      numbers (1 cell → ±1; 2+ → constant difference), date-formatted
 *      numbers (+1 day, constant day step or month step), text dates,
 *      weekday and month names, "Item 1"-style text (and 1st/2nd…).
 */
import type { CellView, DocId, EngineClient, Op, RangeDto } from "@tally/bridge";
import { fillDirection, type FillDir } from "@tally/grid";
import { extensionOf } from "../fill";
import { isDateCode } from "../numfmtLite";

/** Above this many blocks a single repeating Paste is used instead of one per block. */
const MAX_BLOCK_PASTES = 200;

export interface FillContext {
  client: EngineClient;
  doc: DocId;
  sheet: number;
}

/**
 * Existing ops that emulate `FillRange` over `target` (which contains
 * `source` and extends it along one axis).
 */
export async function emulateFill(ctx: FillContext, source: RangeDto, target: RangeDto): Promise<Op[]> {
  const dir = fillDirection(source, target);
  if (!dir) return [];
  const { client, doc, sheet } = ctx;
  const vertical = dir === "down" || dir === "up";
  const ext = extensionOf(source, target, dir);
  const L = vertical ? source.r1 - source.r0 + 1 : source.c1 - source.c0 + 1;
  const N = vertical ? ext.r1 - ext.r0 + 1 : ext.c1 - ext.c0 + 1;

  // 1. Copy the pattern into the extension, block by block.
  const blocks = pasteBlocks(source, ext, dir, L, N);
  const tokens = new Map<string, string>();
  const tokenFor = async (src: RangeDto): Promise<string> => {
    const k = `${src.r0},${src.c0},${src.r1},${src.c1}`;
    let t = tokens.get(k);
    if (t === undefined) {
      t = (await client.copy(doc, { sheet, ranges: [src], active: { r: src.r0, c: src.c0 } })).token;
      tokens.set(k, t);
    }
    return t;
  };
  const ops: Op[] = [];
  if (blocks.length <= MAX_BLOCK_PASTES) {
    for (const b of blocks) ops.push({ type: "paste", sheet, target: b.target, payload: { type: "internal", token: await tokenFor(b.src) } });
  } else {
    // Huge fills: one paste that repeats over the whole extension (a partial last block is left to the engine).
    ops.push({ type: "paste", sheet, target: ext, payload: { type: "internal", token: await tokenFor(source) } });
  }

  // 2. Continue recognised series line by line.
  const [block] = await client.cells(doc, sheet, [source]);
  const byPos = new Map<string, CellView>();
  for (const c of block?.cells ?? []) byPos.set(`${c.r},${c.c}`, c);
  const styleIds = [...new Set([...byPos.values()].map((c) => c.style))];
  const formats = new Map<number, string>();
  if (styleIds.length > 0) for (const st of await client.styles(doc, styleIds)) formats.set(st.id, st.numberFormat);

  const lines = vertical ? range(source.c0, source.c1) : range(source.r0, source.r1);
  for (const line of lines) {
    const values: SourceValue[] = range(0, L - 1).map((i) => {
      const r = vertical ? source.r0 + i : line;
      const c = vertical ? line : source.c0 + i;
      const cell = byPos.get(`${r},${c}`);
      return toSourceValue(cell, cell ? (formats.get(cell.style) ?? "General") : "General");
    });
    const gen = planSeries(values);
    if (!gen) continue;
    // Offsets of the extension cells relative to the source start.
    const offsets = dir === "down" || dir === "right" ? range(L, L + N - 1) : range(-N, -1);
    const inputs = offsets.map(gen);
    if (vertical) {
      ops.push({ type: "setRangeInputs", sheet, r: ext.r0, c: line, rows: inputs.map((v) => [v]) });
    } else {
      ops.push({ type: "setRangeInputs", sheet, r: line, c: ext.c0, rows: [inputs] });
    }
  }
  return ops;
}

interface PasteBlock {
  src: RangeDto;
  target: RangeDto;
}

/** Source sub-ranges and where they go, tiling the extension away from the source. */
export function pasteBlocks(source: RangeDto, ext: RangeDto, dir: FillDir, L: number, N: number): PasteBlock[] {
  const out: PasteBlock[] = [];
  for (let k = 0; k * L < N; k++) {
    const m = Math.min(L, N - k * L); // cells in this block along the axis
    switch (dir) {
      case "down": {
        const r0 = ext.r0 + k * L;
        out.push({ src: { ...source, r1: source.r0 + m - 1 }, target: { ...ext, r0, r1: r0 + m - 1 } });
        break;
      }
      case "right": {
        const c0 = ext.c0 + k * L;
        out.push({ src: { ...source, c1: source.c0 + m - 1 }, target: { ...ext, c0, c1: c0 + m - 1 } });
        break;
      }
      case "up": {
        const r1 = ext.r1 - k * L;
        out.push({ src: { ...source, r0: source.r1 - m + 1 }, target: { ...ext, r0: r1 - m + 1, r1 } });
        break;
      }
      case "left": {
        const c1 = ext.c1 - k * L;
        out.push({ src: { ...source, c0: source.c1 - m + 1 }, target: { ...ext, c0: c1 - m + 1, c1 } });
        break;
      }
    }
  }
  return out;
}

function range(a: number, b: number): number[] {
  const out: number[] = [];
  for (let i = a; i <= b; i++) out.push(i);
  return out;
}

/** What the series planner needs to know about a source cell. */
export type SourceValue =
  | { kind: "number"; num: number; date: boolean }
  | { kind: "text"; text: string }
  | { kind: "empty" }
  | { kind: "other" };

/** CellView flag bit: the cell holds a formula. */
const FLAG_FORMULA = 1;

export function toSourceValue(cell: CellView | undefined, numberFormat: string): SourceValue {
  if (!cell || cell.kind === "empty") return { kind: "empty" };
  if ((cell.flags & FLAG_FORMULA) !== 0) return { kind: "other" };
  if (cell.kind === "number" && cell.num !== null) return { kind: "number", num: cell.num, date: isDateCode(numberFormat) };
  if (cell.kind === "text") return { kind: "text", text: cell.text };
  return { kind: "other" };
}

/** Input text for the cell at offset `i` from the source start (negative = before it). */
export type SeriesGen = (i: number) => string;

const EPS = 1e-9;

function numText(v: number): string {
  return String(Number(v.toPrecision(15)));
}

/** A constant step across `xs`, or `null`. */
function constantStep(xs: readonly number[]): number | null {
  if (xs.length < 2) return null;
  const d = (xs[1] ?? 0) - (xs[0] ?? 0);
  for (let i = 2; i < xs.length; i++) {
    const di = (xs[i] ?? 0) - (xs[i - 1] ?? 0);
    if (Math.abs(di - d) > EPS * Math.max(1, Math.abs(d))) return null;
  }
  return d;
}

/**
 * Continues a source line, or returns `null` to keep the copied pattern.
 * Exported for tests.
 */
export function planSeries(values: readonly SourceValue[]): SeriesGen | null {
  if (values.length === 0 || values.some((v) => v.kind === "empty" || v.kind === "other")) return null;
  if (values.every((v) => v.kind === "number")) {
    const nums = values.map((v) => (v as { num: number }).num);
    const dates = values.every((v) => (v as { date: boolean }).date);
    if (dates) return dateSeries(nums);
    const step = nums.length === 1 ? 1 : constantStep(nums);
    if (step === null) return null;
    const base = nums[0] ?? 0;
    return (i) => numText(base + step * i);
  }
  if (values.every((v) => v.kind === "text")) {
    const texts = values.map((v) => (v as { text: string }).text);
    return listSeries(texts) ?? textDateSeries(texts) ?? numberedTextSeries(texts);
  }
  return null;
}

// -- dates (serial numbers) ------------------------------------------------

const EPOCH = Date.UTC(1899, 11, 30);
const DAY_MS = 86_400_000;

function serialToYmd(serial: number): { y: number; m: number; d: number; frac: number } {
  const days = Math.floor(serial);
  const dt = new Date(EPOCH + days * DAY_MS);
  return { y: dt.getUTCFullYear(), m: dt.getUTCMonth(), d: dt.getUTCDate(), frac: serial - days };
}

function ymdToSerial(y: number, m: number, d: number): number {
  return Math.round((Date.UTC(y, m, d) - EPOCH) / DAY_MS);
}

/** Adds months, clamping the day to the target month's length. */
function addMonths(y: number, m: number, d: number, k: number): number {
  const total = y * 12 + m + k;
  const ny = Math.floor(total / 12);
  const nm = total - ny * 12;
  const last = new Date(Date.UTC(ny, nm + 1, 0)).getUTCDate();
  return ymdToSerial(ny, nm, Math.min(d, last));
}

function dateSeries(serials: readonly number[]): SeriesGen | null {
  const base = serials[0] ?? 0;
  if (serials.length === 1) return (i) => numText(base + i);
  const ymd = serials.map(serialToYmd);
  const first = ymd[0]!;
  const sameDay = ymd.every((p) => p.d === first.d);
  if (sameDay) {
    const months = constantStep(ymd.map((p) => p.y * 12 + p.m));
    if (months !== null && months !== 0) return (i) => numText(addMonths(first.y, first.m, first.d, months * i) + first.frac);
  }
  const step = constantStep(serials);
  if (step === null) return null;
  return (i) => numText(base + step * i);
}

// -- text: day and month names ----------------------------------------------

const LISTS: readonly (readonly string[])[] = [
  ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
  ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
  ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
  ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
];

function matchCase(word: string, like: string): string {
  if (like === like.toUpperCase() && like !== like.toLowerCase()) return word.toUpperCase();
  if (like === like.toLowerCase()) return word.toLowerCase();
  return word;
}

function listSeries(texts: readonly string[]): SeriesGen | null {
  for (const list of LISTS) {
    const idx = texts.map((t) => list.findIndex((w) => w.toLowerCase() === t.trim().toLowerCase()));
    if (idx.some((i) => i < 0)) continue;
    const n = list.length;
    let step = 1;
    if (idx.length >= 2) {
      step = (((idx[1] ?? 0) - (idx[0] ?? 0)) % n + n) % n;
      for (let k = 2; k < idx.length; k++) {
        if (((((idx[k] ?? 0) - (idx[k - 1] ?? 0)) % n) + n) % n !== step) return null;
      }
      if (step === 0) return null;
    }
    const base = idx[0] ?? 0;
    const like = texts[0] ?? "";
    return (i) => matchCase(list[(((base + step * i) % n) + n) % n] ?? "", like);
  }
  return null;
}

// -- text dates (MM/DD/YYYY, YYYY-MM-DD) ------------------------------------

interface TextDate {
  serial: number;
  fmt: (serial: number) => string;
}

function parseTextDate(t: string): TextDate | null {
  const us = /^(\d{1,2})\/(\d{1,2})\/(\d{4})$/.exec(t.trim());
  const pad = (n: number, w: number, on: boolean): string => (on ? String(n).padStart(w, "0") : String(n));
  if (us) {
    const [mm, dd, yy] = [Number(us[1]), Number(us[2]), Number(us[3])];
    if (mm < 1 || mm > 12 || dd < 1 || dd > 31) return null;
    const padM = (us[1] ?? "").length === 2;
    const padD = (us[2] ?? "").length === 2;
    return {
      serial: ymdToSerial(yy, mm - 1, dd),
      fmt: (s) => {
        const p = serialToYmd(s);
        return `${pad(p.m + 1, 2, padM)}/${pad(p.d, 2, padD)}/${p.y}`;
      },
    };
  }
  const iso = /^(\d{4})-(\d{2})-(\d{2})$/.exec(t.trim());
  if (iso) {
    const [yy, mm, dd] = [Number(iso[1]), Number(iso[2]), Number(iso[3])];
    if (mm < 1 || mm > 12 || dd < 1 || dd > 31) return null;
    return {
      serial: ymdToSerial(yy, mm - 1, dd),
      fmt: (s) => {
        const p = serialToYmd(s);
        return `${p.y}-${pad(p.m + 1, 2, true)}-${pad(p.d, 2, true)}`;
      },
    };
  }
  return null;
}

function textDateSeries(texts: readonly string[]): SeriesGen | null {
  const parsed = texts.map(parseTextDate);
  if (parsed.some((p) => p === null)) return null;
  const serials = parsed.map((p) => p!.serial);
  const gen = dateSeries(serials);
  const fmt = parsed[0]!.fmt;
  return gen ? (i) => fmt(Number(gen(i))) : null;
}

// -- "Item 1", "Q3", "1st" ----------------------------------------------------

function ordinal(n: number): string {
  const a = Math.abs(n) % 100;
  if (a >= 11 && a <= 13) return "th";
  return ["th", "st", "nd", "rd"][Math.abs(n) % 10] ?? "th";
}

function numberedTextSeries(texts: readonly string[]): SeriesGen | null {
  type Parts = { pre: string; num: number; digits: number; post: string; ord: boolean };
  const parse = (t: string): Parts | null => {
    const trailing = /^(.*?)(\d+)(\D*)$/.exec(t);
    if (!trailing) return null;
    const post = trailing[3] ?? "";
    const ord = /^(st|nd|rd|th)$/i.test(post) && (trailing[1] ?? "") === "";
    // Only a number at the very start or end counts ("Item 1", "1st", "Q3"), not "A1B".
    if (post !== "" && !ord && (trailing[1] ?? "") !== "") return null;
    return { pre: trailing[1] ?? "", num: Number(trailing[2]), digits: (trailing[2] ?? "").length, post, ord };
  };
  const parts = texts.map(parse);
  if (parts.some((p) => p === null)) return null;
  const ps = parts as Parts[];
  const head = ps[0]!;
  if (ps.some((p) => p.pre !== head.pre || (p.ord ? "" : p.post) !== (head.ord ? "" : head.post) || p.ord !== head.ord)) return null;
  const step = ps.length === 1 ? 1 : constantStep(ps.map((p) => p.num));
  if (step === null) return null;
  const width = head.digits > 1 && String(head.num).length < head.digits ? head.digits : 0;
  const upper = head.post === head.post.toUpperCase() && head.post !== head.post.toLowerCase();
  return (i) => {
    const n = head.num + step * i;
    const digits = n < 0 ? `-${String(-n).padStart(width, "0")}` : String(n).padStart(width, "0");
    if (head.ord) {
      const suf = ordinal(n);
      return `${head.pre}${digits}${upper ? suf.toUpperCase() : suf}`;
    }
    return `${head.pre}${digits}${head.post}`;
  };
}

