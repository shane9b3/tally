/**
 * `SyntheticSource`: a deterministic fake data source for the demo and perf
 * tests. Every cell is generated on demand from its coordinates (no
 * per-row storage), so a 1,048,576-row sheet costs nothing until scrolled.
 *
 * Columns: Item, Region, Date (a number shown as MM/DD/YYYY), Quantity,
 * Price, Total (negative = refund, red), Paid (bool), Status (chip), Notes
 * (long text that overflows), an empty column, then numeric metrics with
 * varying magnitudes (to exercise General shrink-to-fit) and an error column.
 *
 * With `latencyMs > 0` the source behaves like an async engine: tiles are
 * empty until `ensureRanges` "loads" them after the delay.
 */
import type { CellView, RangeDto, SheetLayout, StyleDto } from "@tally/bridge";
import { generalText } from "./fitNumber";
import { TILE_COLS, TILE_ROWS } from "./tiles";
import type { DirtySet, GridDataSource } from "./types";

export interface SyntheticOptions {
  /** Simulated fetch latency; 0 (default) = everything is always loaded. */
  latencyMs?: number;
}

const HEADERS = ["Item", "Region", "Date", "Quantity", "Price", "Total", "Paid", "Status", "Notes", ""];
const REGIONS = ["North", "South", "East", "West", "Central"];
const NOTES = [
  "Follow up with the customer about the delivery window next week",
  "Discount approved by finance",
  "Shipped in two parcels because of stock levels at the warehouse",
];
const RED = 0xdc2626ff;

const blank = (): StyleDto => ({
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
});

/** Style ids used by the synthetic data. */
export const SYN_STYLE = { normal: 0, bold: 1, italic: 2, header: 3, paid: 4, due: 5, overdue: 6, date: 7, money: 8 } as const;

const STYLES: StyleDto[] = [
  blank(),
  { ...blank(), id: 1, bold: true },
  { ...blank(), id: 2, italic: true, color: 0x6b7280ff },
  { ...blank(), id: 3, bold: true, fill: 0xf9fafbff, borders: { top: null, left: null, right: null, bottom: { style: "solid", widthPx: 1, color: 0xd1d5dbff } } },
  { ...blank(), id: 4, display: { type: "chip", bg: 0xdcfce7ff, fg: 0x166534ff } },
  { ...blank(), id: 5, display: { type: "chip", bg: 0xfef3c7ff, fg: 0x92400eff } },
  { ...blank(), id: 6, display: { type: "chip", bg: 0xfee2e2ff, fg: 0x991b1bff } },
  { ...blank(), id: 7, numberFormat: "MM/DD/YYYY" },
  { ...blank(), id: 8, numberFormat: "#,##0.00" },
];

/** Fast deterministic hash of (r, c) to [0, 1). */
function hash(r: number, c: number): number {
  let h = (r * 374761393 + c * 668265263) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}

function dateText(serial: number): string {
  // Serial 0 = 1899-12-30.
  const d = new Date(Date.UTC(1899, 11, 30) + serial * 86400000);
  const mm = String(d.getUTCMonth() + 1).padStart(2, "0");
  const dd = String(d.getUTCDate()).padStart(2, "0");
  return `${mm}/${dd}/${d.getUTCFullYear()}`;
}

function money(n: number): string {
  const s = Math.abs(n).toFixed(2).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return n < 0 ? `-${s}` : s;
}

export class SyntheticSource implements GridDataSource {
  readonly rows: number;
  readonly cols: number;
  private readonly latencyMs: number;
  private readonly layout: SheetLayout;
  private readonly listeners = new Set<(d: DirtySet) => void>();
  private readonly overrides = new Map<number, CellView | null>();
  private readonly loaded = new Set<number>();
  private readonly loading = new Set<number>();
  /** Small row cache so a frame's getCell calls do not allocate repeatedly. */
  private readonly rowCache = new Map<number, (CellView | null | undefined)[]>();
  /** Counters for tests and the demo. */
  ensureCalls = 0;

  constructor(rows = 1_048_576, cols = 100, opts: SyntheticOptions = {}) {
    this.rows = rows;
    this.cols = cols;
    this.latencyMs = opts.latencyMs ?? 0;
    this.layout = {
      sheet: 1,
      used: { r0: 0, c0: 0, r1: rows - 1, c1: cols - 1 },
      defaultColWidthPx: 100,
      defaultRowHeightPx: 24,
      colRuns: [
        { start: 0, end: 0, size: 120, hidden: false },
        { start: 7, end: 7, size: 96, hidden: false },
        { start: 8, end: 8, size: 140, hidden: false },
        { start: 9, end: 9, size: 72, hidden: false },
      ],
      rowRuns: [],
      frozen: null,
      merges: [],
    };
  }

  getLayout(_sheet: number): SheetLayout {
    return this.layout;
  }

  getStyle(id: number): StyleDto | undefined {
    return STYLES[id];
  }

  getCell(r: number, c: number): CellView | undefined {
    if (r < 0 || c < 0 || r >= this.rows || c >= this.cols) return undefined;
    if (this.latencyMs > 0 && !this.loaded.has(this.tileId(r, c))) return undefined;
    const k = r * this.cols + c;
    if (this.overrides.has(k)) return this.overrides.get(k) ?? undefined;
    let row = this.rowCache.get(r);
    if (!row) {
      row = new Array<CellView | null | undefined>(this.cols);
      if (this.rowCache.size > 512) this.rowCache.clear();
      this.rowCache.set(r, row);
    }
    let v = row[c];
    if (v === undefined) {
      v = this.generate(r, c);
      row[c] = v;
    }
    return v ?? undefined;
  }

  ensureRanges(ranges: RangeDto[]): void {
    this.ensureCalls++;
    if (this.latencyMs <= 0) return;
    const fresh: number[] = [];
    for (const rg of ranges) {
      for (let tr = Math.floor(rg.r0 / TILE_ROWS); tr <= Math.floor(rg.r1 / TILE_ROWS); tr++) {
        for (let tc = Math.floor(rg.c0 / TILE_COLS); tc <= Math.floor(rg.c1 / TILE_COLS); tc++) {
          const id = tr * 1024 + tc;
          if (this.loaded.has(id) || this.loading.has(id)) continue;
          this.loading.add(id);
          fresh.push(id);
        }
      }
    }
    if (fresh.length === 0) return;
    setTimeout(() => {
      const dirty: RangeDto[] = [];
      for (const id of fresh) {
        this.loading.delete(id);
        this.loaded.add(id);
        const tr = Math.floor(id / 1024);
        const tc = id % 1024;
        dirty.push({ r0: tr * TILE_ROWS, c0: tc * TILE_COLS, r1: tr * TILE_ROWS + TILE_ROWS - 1, c1: tc * TILE_COLS + TILE_COLS - 1 });
      }
      this.emit(dirty);
    }, this.latencyMs);
  }

  subscribe(fn: (dirty: DirtySet) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Demo editing: replaces a cell with typed text (numbers recognised). */
  setInput(r: number, c: number, input: string): void {
    const k = r * this.cols + c;
    if (input === "") this.overrides.set(k, null);
    else {
      const n = Number(input);
      const isNum = input.trim() !== "" && Number.isFinite(n);
      this.overrides.set(k, {
        r,
        c,
        text: isNum ? generalText(n) : input,
        kind: isNum ? "number" : "text",
        num: isNum ? n : null,
        style: 0,
        color: isNum && n < 0 ? RED : null,
        flags: 0,
      });
    }
    this.emit([{ r0: r, c0: c, r1: r, c1: c }]);
  }

  /** Demo: clears a range (bounded to 100k cells). */
  clear(range: RangeDto): void {
    let n = 0;
    for (let r = range.r0; r <= Math.min(range.r1, this.rows - 1); r++) {
      for (let c = range.c0; c <= Math.min(range.c1, this.cols - 1); c++) {
        this.overrides.set(r * this.cols + c, null);
        if (++n > 100_000) break;
      }
    }
    this.emit([range]);
  }

  /** The text a user would edit (the display text here). */
  inputOf(r: number, c: number): string {
    return this.getCell(r, c)?.text ?? "";
  }

  /** Ctrl+arrow edge jump over this data (dense, so mostly the sheet edge). */
  navigateEdge(from: { r: number; c: number }, dir: "up" | "down" | "left" | "right"): { r: number; c: number } {
    const step = { up: [-1, 0], down: [1, 0], left: [0, -1], right: [0, 1] }[dir];
    const dr = step[0] ?? 0;
    const dc = step[1] ?? 0;
    const filled = (r: number, c: number): boolean => {
      const v = this.getCell(r, c);
      return v !== undefined && v.text !== "";
    };
    let { r, c } = from;
    const inside = (rr: number, cc: number): boolean => rr >= 0 && cc >= 0 && rr < this.rows && cc < this.cols;
    if (!inside(r + dr, c + dc)) return { r, c };
    if (filled(r, c) && filled(r + dr, c + dc)) {
      while (inside(r + dr, c + dc) && filled(r + dr, c + dc)) {
        r += dr;
        c += dc;
      }
      return { r, c };
    }
    r += dr;
    c += dc;
    while (inside(r, c) && !filled(r, c) && inside(r + dr, c + dc)) {
      r += dr;
      c += dc;
    }
    return { r, c };
  }

  private tileId(r: number, c: number): number {
    return Math.floor(r / TILE_ROWS) * 1024 + Math.floor(c / TILE_COLS);
  }

  private emit(d: DirtySet): void {
    for (const fn of this.listeners) fn(d);
  }

  private generate(r: number, c: number): CellView | null {
    const cell = (text: string, kind: CellView["kind"], num: number | null, style = 0, color: number | null = null): CellView => ({
      r,
      c,
      text,
      kind,
      num,
      style,
      color,
      flags: 0,
    });
    if (r === 0) {
      const label = c < HEADERS.length ? (HEADERS[c] ?? "") : c === this.cols - 1 ? "Error" : `Metric ${c - HEADERS.length + 1}`;
      return label === "" ? null : cell(label, "text", null, SYN_STYLE.header);
    }
    const h = hash(r, c);
    const rowStyle = r % 50 === 0 ? SYN_STYLE.bold : 0;
    switch (c) {
      case 0:
        return cell(`Item ${r}`, "text", null, r % 100 === 0 ? SYN_STYLE.italic : rowStyle);
      case 1:
        return cell(REGIONS[Math.floor(h * REGIONS.length)] ?? "North", "text", null, rowStyle);
      case 2: {
        const serial = 44927 + (r % 1460);
        return cell(dateText(serial), "number", serial, SYN_STYLE.date);
      }
      case 3: {
        const q = 1 + Math.floor(h * 500);
        return cell(String(q), "number", q, rowStyle);
      }
      case 4: {
        const p = Math.round(h * 100000) / 100;
        return cell(money(p), "number", p, SYN_STYLE.money);
      }
      case 5: {
        const q = 1 + Math.floor(hash(r, 3) * 500);
        const p = Math.round(hash(r, 4) * 100000) / 100;
        const total = (r % 13 === 0 ? -1 : 1) * Math.round(q * p * 100) / 100;
        return cell(money(total), "number", total, SYN_STYLE.money, total < 0 ? RED : null);
      }
      case 6: {
        const b = h > 0.4;
        return cell(b ? "TRUE" : "FALSE", "bool", null);
      }
      case 7: {
        const k = Math.floor(h * 3);
        const label = ["Paid", "Due", "Overdue"][k] ?? "Paid";
        return cell(label, "text", null, SYN_STYLE.paid + k);
      }
      case 8:
        return r % 9 === 0 ? cell(NOTES[r % NOTES.length] ?? "", "text", null) : null;
      case 9:
        return null;
      default: {
        if (c === this.cols - 1) return r % 37 === 0 ? cell("#DIV/0!", "error", null) : null;
        if (h < 0.15) return null; // some gaps
        const mag = (c + r) % 7;
        const num =
          mag === 0 ? h * 1e12 : mag === 1 ? h * 3.14159265358979 : mag === 2 ? -h * 1000 : mag === 3 ? h * 0.0001 : Math.round(h * 100000) / 100;
        return cell(generalText(num), "number", num, rowStyle, num < 0 ? RED : null);
      }
    }
  }
}
