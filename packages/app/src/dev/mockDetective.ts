/**
 * The browser mock's Tools ▸ Detective: a port of the engine's
 * `detective.rs` (AOO `ScDetectiveFunc`) over a snapshot of the mock's
 * formulas, so the web build and e2e see the same levels, removals,
 * Refresh Traces and AutoRefresh as the desktop app. Operations do not
 * move with inserted rows here (the engine moves them).
 */
import type { DetectiveAction, DetectiveArrowDto, DetectiveState, RangeDto } from "@tally/bridge";

export type DetOpKind = "addPred" | "delPred" | "addSucc" | "delSucc" | "addError";

export interface DetOp {
  r: number;
  c: number;
  kind: DetOpKind;
}

export interface SheetDet {
  ops: DetOp[];
  arrows: DetectiveArrowDto[];
}

/** One sheet's formulas and error cells, read from the mock. */
export interface DetSnapshot {
  /** Sheet ids in tab order. */
  sheets: number[];
  /** `sheet:r:c` → the formula's references. */
  formulas: Map<string, { sheet: number; range: RangeDto }[]>;
  /** `sheet:r:c` of cells showing an error. */
  errors: Set<string>;
}

type Ins = "continue" | "inserted" | "empty" | "circular";

const k = (s: number, r: number, c: number): string => `${s}:${r}:${c}`;
const unkey = (key: string): [number, number, number] => {
  const [s = 0, r = 0, c = 0] = key.split(":").map(Number);
  return [s, r, c];
};
const single = (r: number, c: number): RangeDto => ({ r0: r, c0: c, r1: r, c1: c });
const isSingle = (g: RangeDto): boolean => g.r0 === g.r1 && g.c0 === g.c1;
const hits = (a: RangeDto, b: RangeDto): boolean => a.r0 <= b.r1 && b.r0 <= a.r1 && a.c0 <= b.c1 && b.c0 <= a.c1;
const arrowKey = (a: DetectiveArrowDto): string => `${a.from ? `${a.from.r0},${a.from.c0}` : "*"}>${a.to ? `${a.to.r},${a.to.c}` : "*"}`;

function merge(acc: Ins, sub: Ins): Ins {
  if (sub === "inserted") return "inserted";
  if (sub === "continue" && acc !== "inserted") return "continue";
  if (sub === "circular" && acc === "empty") return "circular";
  return acc;
}

class Page {
  keys: Set<string>;
  running = new Set<string>();
  maxLevel = 0;
  work = 200_000;

  constructor(
    private readonly snap: DetSnapshot,
    private readonly sheet: number,
    public arrows: DetectiveArrowDto[],
  ) {
    this.keys = new Set(arrows.map(arrowKey));
  }

  private spent(): boolean {
    if (this.work <= 0) return true;
    this.work--;
    return false;
  }

  private refs(r: number, c: number): { sheet: number; range: RangeDto }[] | null {
    return this.snap.formulas.get(k(this.sheet, r, c)) ?? null;
  }

  private hasError(sheet: number, g: RangeDto): boolean {
    return this.firstError(sheet, g) !== null;
  }

  private firstError(sheet: number, g: RangeDto): { r: number; c: number } | null {
    for (const key of this.snap.errors) {
      const [s, r, c] = unkey(key);
      if (s === sheet && r >= g.r0 && r <= g.r1 && c >= g.c0 && c <= g.c1) return { r, c };
    }
    return null;
  }

  private push(a: DetectiveArrowDto): boolean {
    this.keys.add(arrowKey(a));
    this.arrows.push(a);
    return true;
  }

  private drawEntry(r: number, c: number, ref: { sheet: number; range: RangeDto }): boolean {
    const alien = ref.sheet !== this.sheet;
    const a: DetectiveArrowDto = { from: alien ? null : { ...ref.range }, to: { r, c }, error: false };
    if (this.keys.has(arrowKey(a))) return false;
    a.error = this.hasError(ref.sheet, ref.range);
    return this.push(a);
  }

  private drawAlien(ref: RangeDto): boolean {
    const a: DetectiveArrowDto = { from: { ...ref }, to: null, error: false };
    if (this.keys.has(arrowKey(a))) return false;
    a.error = this.hasError(this.sheet, ref);
    return this.push(a);
  }

  private deleteAt(r: number, c: number, dest: boolean): void {
    this.arrows = this.arrows.filter((a) => (dest ? !(a.to && a.to.r === r && a.to.c === c) : !(a.from && a.from.r0 === r && a.from.c0 === c)));
    this.keys = new Set(this.arrows.map(arrowKey));
  }

  private formulaCellsIn(g: RangeDto): { r: number; c: number }[] {
    const out: { r: number; c: number }[] = [];
    for (const key of this.snap.formulas.keys()) {
      const [s, r, c] = unkey(key);
      if (s === this.sheet && r >= g.r0 && r <= g.r1 && c >= g.c0 && c <= g.c1) out.push({ r, c });
    }
    return out.sort((a, b) => a.c - b.c || a.r - b.r);
  }

  private readers(g: RangeDto, localOnly: boolean): { sheet: number; r: number; c: number }[] {
    const out: { sheet: number; r: number; c: number }[] = [];
    for (const [key, refs] of this.snap.formulas) {
      const [s, r, c] = unkey(key);
      if (localOnly && s !== this.sheet) continue;
      if (refs.some((x) => x.sheet === this.sheet && hits(x.range, g))) out.push({ sheet: s, r, c });
    }
    const tab = (s: number): number => this.snap.sheets.indexOf(s);
    return out.sort((a, b) => tab(a.sheet) - tab(b.sheet) || a.c - b.c || a.r - b.r);
  }

  insertPred(r: number, c: number, level: number): Ins {
    if (this.spent()) return "empty";
    const refs = this.refs(r, c);
    if (!refs) return "empty";
    const id = k(this.sheet, r, c);
    if (this.running.has(id)) return "circular";
    this.running.add(id);
    let result: Ins = "empty";
    for (const ref of refs) {
      if (this.drawEntry(r, c, ref)) result = "inserted";
      else if (level < this.maxLevel) {
        let sub: Ins = "empty";
        if (ref.sheet === this.sheet) {
          if (isSingle(ref.range)) sub = this.insertPred(ref.range.r0, ref.range.c0, level + 1);
          else for (const f of this.formulaCellsIn(ref.range)) sub = merge(sub, this.insertPred(f.r, f.c, level + 1));
        }
        result = merge(result, sub);
      } else if (result !== "inserted") result = "continue";
    }
    this.running.delete(id);
    return result;
  }

  findPred(r: number, c: number, level: number, del: number): number {
    if (this.spent()) return level;
    const refs = this.refs(r, c);
    if (!refs) return level;
    const id = k(this.sheet, r, c);
    if (this.running.has(id)) return level;
    this.running.add(id);
    let result = level;
    if (del !== 0 && level + 1 === del) this.deleteAt(r, c, true);
    else {
      for (const ref of refs) {
        if (ref.sheet !== this.sheet) {
          if (this.keys.has(`*>${r},${c}`)) result = Math.max(result, level + 1);
          continue;
        }
        if (this.keys.has(`${ref.range.r0},${ref.range.c0}>${r},${c}`)) {
          if (isSingle(ref.range)) result = Math.max(result, this.findPred(ref.range.r0, ref.range.c0, level + 1, del));
          else {
            result = Math.max(result, level + 1);
            for (const f of this.formulaCellsIn(ref.range)) result = Math.max(result, this.findPred(f.r, f.c, level + 1, del));
          }
        }
      }
    }
    this.running.delete(id);
    return result;
  }

  insertSucc(g: RangeDto, level: number): Ins {
    let result: Ins = "empty";
    for (const f of this.readers(g, false)) {
      if (this.spent()) break;
      const id = k(f.sheet, f.r, f.c);
      const wasRunning = this.running.has(id);
      this.running.add(id);
      for (const ref of this.snap.formulas.get(id) ?? []) {
        if (ref.sheet !== this.sheet || !hits(ref.range, g)) continue;
        const alien = f.sheet !== this.sheet;
        const drawn = alien ? this.drawAlien(ref.range) : this.drawEntry(f.r, f.c, ref);
        if (drawn) result = "inserted";
        else if (wasRunning) {
          if (result === "empty") result = "circular";
        } else if (level < this.maxLevel) result = merge(result, alien ? "empty" : this.insertSucc(single(f.r, f.c), level + 1));
        else if (result !== "inserted") result = "continue";
      }
      if (!wasRunning) this.running.delete(id);
    }
    return result;
  }

  findSucc(g: RangeDto, level: number, del: number): number {
    let result = level;
    const deleting = del !== 0 && level + 1 === del;
    for (const f of this.readers(g, true)) {
      if (this.spent()) break;
      const id = k(f.sheet, f.r, f.c);
      const wasRunning = this.running.has(id);
      this.running.add(id);
      for (const ref of this.snap.formulas.get(id) ?? []) {
        if (ref.sheet !== this.sheet || !hits(ref.range, g)) continue;
        if (deleting) this.deleteAt(ref.range.r0, ref.range.c0, false);
        else if (!wasRunning && this.keys.has(`${ref.range.r0},${ref.range.c0}>${f.r},${f.c}`)) result = Math.max(result, this.findSucc(single(f.r, f.c), level + 1, del));
      }
      if (!wasRunning) this.running.delete(id);
    }
    return result;
  }

  insertError(r: number, c: number, level: number): Ins {
    if (this.spent()) return "empty";
    const refs = this.refs(r, c);
    if (!refs) return "empty";
    const id = k(this.sheet, r, c);
    if (this.running.has(id)) return "circular";
    this.running.add(id);
    let result: Ins = "empty";
    let has = false;
    for (const ref of refs) {
      const err = this.firstError(ref.sheet, ref.range);
      if (!err) continue;
      has = true;
      if (this.drawEntry(r, c, { sheet: ref.sheet, range: single(err.r, err.c) })) result = "inserted";
      if (level < this.maxLevel && ref.sheet === this.sheet && this.insertError(err.r, err.c, level + 1) === "inserted") result = "inserted";
    }
    this.running.delete(id);
    if (!has && this.insertPred(r, c, this.maxLevel) === "inserted") result = "inserted";
    return result;
  }

  apply(op: DetOp): boolean {
    const deepen = (f: () => Ins): boolean => {
      let result: Ins = "continue";
      for (let level = 0; result === "continue" && level < 1000 && this.work > 0; level++) {
        this.maxLevel = level;
        result = f();
      }
      return result === "inserted";
    };
    const removeLevel = (find: (del: number) => number): boolean => {
      const n = find(0);
      if (n > 0) find(n);
      return n !== 0;
    };
    switch (op.kind) {
      case "addPred":
        return deepen(() => this.insertPred(op.r, op.c, 0));
      case "addSucc":
        return deepen(() => this.insertSucc(single(op.r, op.c), 0));
      case "delPred":
        return removeLevel((del) => this.findPred(op.r, op.c, 0, del));
      case "delSucc":
        return removeLevel((del) => this.findSucc(single(op.r, op.c), 0, del));
      case "addError":
        if (!this.snap.errors.has(k(this.sheet, op.r, op.c))) return false;
        this.maxLevel = 1000;
        return this.insertError(op.r, op.c, 0) === "inserted";
    }
  }
}

const KIND: Partial<Record<DetectiveAction, DetOpKind>> = {
  tracePrecedents: "addPred",
  removePrecedents: "delPred",
  traceDependents: "addSucc",
  removeDependents: "delSucc",
  traceError: "addError",
};

/** A document's detective state in the mock. */
export class MockDetective {
  readonly sheets = new Map<number, SheetDet>();
  auto = true;

  private of(sheet: number): SheetDet {
    let d = this.sheets.get(sheet);
    if (!d) {
      d = { ops: [], arrows: [] };
      this.sheets.set(sheet, d);
    }
    return d;
  }

  hasOps(): boolean {
    return [...this.sheets.values()].some((d) => d.ops.length > 0);
  }

  state(sheet: number, done: boolean): DetectiveState {
    return { done, arrows: this.of(sheet).arrows.map((a) => ({ ...a })), hasOps: this.hasOps(), dirty: false, incomplete: false };
  }

  run(snap: DetSnapshot, sheet: number, r: number, c: number, action: DetectiveAction): boolean {
    if (action === "refresh") return this.refresh(snap);
    if (action === "removeAll") {
      const had = this.of(sheet).arrows.length > 0;
      if (!had && !this.hasOps()) return false;
      for (const d of this.sheets.values()) d.ops = [];
      this.of(sheet).arrows = [];
      return true;
    }
    const kind = KIND[action];
    if (!kind) return false;
    const d = this.of(sheet);
    const page = new Page(snap, sheet, d.arrows);
    const op = { r, c, kind };
    const done = page.apply(op);
    d.arrows = page.arrows;
    if (done) d.ops.push(op);
    return done;
  }

  /** Refresh Traces: every sheet's arrows from its operations (when the list is not empty). */
  refresh(snap: DetSnapshot): boolean {
    if (!this.hasOps()) return false;
    for (const [sheet, d] of this.sheets) {
      const page = new Page(snap, sheet, []);
      for (const op of d.ops) page.apply(op);
      d.arrows = page.arrows;
    }
    return true;
  }
}
