/**
 * The v0.3 side model of the browser mock (`WebMockClient`): conditional
 * formats, validations, notes, links, protection, page setup, document
 * properties and the locale, kept in the mock's undoable `DocExt` (plain
 * JSON, snapshotted for undo/redo). Pure functions: `applyV3Op` changes
 * the model for one op (or refuses it), the checks implement protection
 * and validation the way docs/api.md describes the engine's.
 */
import type {
  CellPos,
  ConditionalFormatDto,
  DocPropertiesDto,
  NoteDto,
  Op,
  PageSetupDto,
  ProtectionOptions,
  RangeDto,
  ValidationDto,
} from "@tally/bridge";
import { defaultPageSetup } from "../v03";

export interface SheetProtection {
  password: string | null;
  options: ProtectionOptions;
}

export interface V3Ext {
  cfs: Record<number, ConditionalFormatDto[]>;
  validations: Record<number, ValidationDto[]>;
  notes: Record<number, Record<string, NoteDto>>;
  links: Record<number, Record<string, string>>;
  protection: Record<number, SheetProtection | null>;
  /** Unlocked ranges per sheet (cells are locked by default). */
  unlocked: Record<number, RangeDto[]>;
  structure: { password: string | null } | null;
  pages: Record<number, PageSetupDto>;
  props: DocPropertiesDto;
  locale: string;
  nextId: number;
}

export function emptyV3(): V3Ext {
  return {
    cfs: {},
    validations: {},
    notes: {},
    links: {},
    protection: {},
    unlocked: {},
    structure: null,
    pages: {},
    props: { title: null, subject: null, keywords: [], description: null, author: null, created: new Date().toISOString().slice(0, 19), modified: null, generator: "Tally (browser demo)" },
    locale: "en-US",
    nextId: 1,
  };
}

export const cellKey = (r: number, c: number): string => `${r},${c}`;

const inRange = (rg: RangeDto, r: number, c: number): boolean => r >= rg.r0 && r <= rg.r1 && c >= rg.c0 && c <= rg.c1;
const overlaps = (a: RangeDto, b: RangeDto): boolean => a.r0 <= b.r1 && b.r0 <= a.r1 && a.c0 <= b.c1 && b.c0 <= a.c1;

/** Subtracts `cut` from `rg` (up to four rectangles). */
export function subtract(rg: RangeDto, cut: RangeDto): RangeDto[] {
  if (!overlaps(rg, cut)) return [{ ...rg }];
  const out: RangeDto[] = [];
  if (cut.r0 > rg.r0) out.push({ ...rg, r1: cut.r0 - 1 });
  if (cut.r1 < rg.r1) out.push({ ...rg, r0: cut.r1 + 1 });
  const r0 = Math.max(rg.r0, cut.r0);
  const r1 = Math.min(rg.r1, cut.r1);
  if (cut.c0 > rg.c0) out.push({ r0, r1, c0: rg.c0, c1: cut.c0 - 1 });
  if (cut.c1 < rg.c1) out.push({ r0, r1, c0: cut.c1 + 1, c1: rg.c1 });
  return out;
}

/** Whether `cell` is locked on `sheet` (all cells are, unless unlocked). */
export function isLocked(x: V3Ext, sheet: number, cell: CellPos): boolean {
  return !(x.unlocked[sheet] ?? []).some((rg) => inRange(rg, cell.r, cell.c));
}

/** The cells an op writes (for protection), or `null` if it writes none. */
function writtenCells(op: Op): RangeDto[] | null {
  switch (op.type) {
    case "setCellInput":
    case "setNote":
    case "deleteNote":
    case "setLink":
      return [{ r0: op.r, c0: op.c, r1: op.r, c1: op.c }];
    case "setRangeInputs":
      return [{ r0: op.r, c0: op.c, r1: op.r + Math.max(0, op.rows.length - 1), c1: op.c + Math.max(0, ...op.rows.map((x) => x.length - 1)) }];
    case "clear":
    case "clearLink":
      return op.ranges;
    case "paste":
    case "pasteSpecial":
      return [op.target];
    case "fillRange":
      return [op.target];
    case "setArrayFormula":
      return [op.range];
    case "moveRange":
      return [op.from];
    default:
      return null;
  }
}

/**
 * Why a protected sheet or document refuses `op` (`null`: allowed). Edits
 * of locked cells fail; formatting needs `formatCells`; structure ops need
 * their option; CF, validations, merges and sizes are refused.
 */
export function protectionRefusal(x: V3Ext, op: Op): string | null {
  if ((op.type === "addSheet" || op.type === "deleteSheet" || op.type === "renameSheet" || op.type === "moveSheet") && x.structure) {
    return "The document structure is protected.";
  }
  const sheet = "sheet" in op && typeof op.sheet === "number" ? op.sheet : null;
  if (sheet === null) return null;
  const p = x.protection[sheet];
  if (!p) return null;
  const msg = "The cell is protected.";
  const written = writtenCells(op);
  if (written) {
    for (const rg of written) {
      for (let r = rg.r0; r <= Math.min(rg.r1, rg.r0 + 2000); r++) {
        for (let c = rg.c0; c <= Math.min(rg.c1, rg.c0 + 200); c++) if (isLocked(x, sheet, { r, c })) return msg;
      }
    }
    return null;
  }
  switch (op.type) {
    case "applyStyle":
    case "setNumberFormat":
      return p.options.formatCells ? null : msg;
    case "insertRows":
      return p.options.insertRows ? null : "The sheet is protected.";
    case "insertCols":
      return p.options.insertCols ? null : "The sheet is protected.";
    case "deleteRows":
      return p.options.deleteRows ? null : "The sheet is protected.";
    case "deleteCols":
      return p.options.deleteCols ? null : "The sheet is protected.";
    case "sortRange":
      return p.options.sort ? null : "The sheet is protected.";
    case "setAutoFilter":
    case "setColumnFilter":
      return p.options.autoFilter ? null : "The sheet is protected.";
    case "addConditionalFormat":
    case "updateConditionalFormat":
    case "deleteConditionalFormat":
    case "addValidation":
    case "updateValidation":
    case "deleteValidation":
    case "mergeCells":
    case "unmergeCells":
    case "setColWidth":
    case "setRowHeight":
    case "setCellProtection":
      return "The sheet is protected.";
    default:
      return null;
  }
}

/** The validation of a cell (the newest one covering it). */
export function validationAt(x: V3Ext, sheet: number, r: number, c: number): ValidationDto | null {
  const list = x.validations[sheet] ?? [];
  for (let i = list.length - 1; i >= 0; i--) if (list[i]!.ranges.some((rg) => inRange(rg, r, c))) return list[i]!;
  return null;
}

function num(s: string): number | null {
  const t = s.trim().replace(/[$,\s]/g, "");
  if (t === "") return null;
  const n = Number(t.replace(/%$/, ""));
  return Number.isFinite(n) ? (t.endsWith("%") ? n / 100 : n) : null;
}

function cmp(op: string, v: number, a: number, b: number): boolean {
  switch (op) {
    case "eq":
      return v === a;
    case "ne":
      return v !== a;
    case "gt":
      return v > a;
    case "ge":
      return v >= a;
    case "lt":
      return v < a;
    case "le":
      return v <= a;
    case "between":
      return v >= Math.min(a, b) && v <= Math.max(a, b);
    case "notBetween":
      return v < Math.min(a, b) || v > Math.max(a, b);
    default:
      return true;
  }
}

/**
 * Whether typed `input` passes `v` (list items as typed in the rule; a
 * range source and date, time and custom rules always pass here).
 */
export function passes(v: ValidationDto, input: string, listItems: readonly string[] | null): boolean {
  const t = input.trim();
  if (t === "") return v.allowBlank;
  const k = v.kind;
  switch (k.type) {
    case "any":
      return true;
    case "list": {
      const items = listItems ?? (k.source.type === "values" ? k.source.items : null);
      return items === null || items.some((i) => i.toLowerCase() === t.toLowerCase());
    }
    case "whole":
    case "decimal": {
      const n = num(t);
      if (n === null || (k.type === "whole" && !Number.isInteger(n))) return false;
      return cmp(k.op, n, num(k.formulas[0] ?? "") ?? -Infinity, num(k.formulas[1] ?? "") ?? Infinity);
    }
    case "textLength":
      return cmp(k.op, t.length, num(k.formulas[0] ?? "") ?? 0, num(k.formulas[1] ?? "") ?? Infinity);
    default:
      return true;
  }
}

export const DEFAULT_VALIDATION_MESSAGE = "The value you entered is not valid.";

/** Per-op outcome of the v0.3 side model. */
export type V3Outcome = { kind: "pass" } | { kind: "handled"; touched: RangeDto[]; layout: boolean; sheetsChanged?: boolean } | { kind: "error"; code: string; message: string };

/** Applies a v0.3 op to the side model, or `pass` for ops the mock itself applies. */
export function applyV3Op(x: V3Ext, op: Op, now: () => string = () => new Date().toISOString().slice(0, 19)): V3Outcome {
  const err = (code: string, message: string): V3Outcome => ({ kind: "error", code, message });
  switch (op.type) {
    case "addConditionalFormat": {
      if (op.rules.length === 0 || op.ranges.length === 0) return err("invalid_argument", "a conditional format needs a range and a rule");
      const list = x.cfs[op.sheet] ?? [];
      list.push({ id: x.nextId++, ranges: op.ranges, rules: op.rules });
      x.cfs[op.sheet] = list;
      return { kind: "handled", touched: op.ranges, layout: false };
    }
    case "updateConditionalFormat": {
      const list = x.cfs[op.sheet] ?? [];
      const i = list.findIndex((c) => c.id === op.id);
      if (i < 0) return err("not_found", `no conditional format ${op.id}`);
      const old = list[i]!;
      list[i] = { id: op.id, ranges: op.ranges, rules: op.rules };
      return { kind: "handled", touched: [...old.ranges, ...op.ranges], layout: false };
    }
    case "deleteConditionalFormat": {
      const list = x.cfs[op.sheet] ?? [];
      const old = list.find((c) => c.id === op.id);
      if (!old) return err("not_found", `no conditional format ${op.id}`);
      x.cfs[op.sheet] = list.filter((c) => c.id !== op.id);
      return { kind: "handled", touched: old.ranges, layout: false };
    }
    case "addValidation": {
      const list = (x.validations[op.sheet] ?? []).flatMap((v): ValidationDto[] => {
        // A new validation replaces older ones where they overlap.
        const ranges = v.ranges.flatMap((rg) => op.validation.ranges.reduce<RangeDto[]>((acc, cut) => acc.flatMap((a) => subtract(a, cut)), [rg]));
        return ranges.length > 0 ? [{ ...v, ranges }] : [];
      });
      list.push({ ...op.validation, id: x.nextId++ });
      x.validations[op.sheet] = list;
      return { kind: "handled", touched: op.validation.ranges, layout: false };
    }
    case "updateValidation": {
      const list = x.validations[op.sheet] ?? [];
      const i = list.findIndex((v) => v.id === op.id);
      if (i < 0) return err("not_found", `no validation ${op.id}`);
      const old = list[i]!;
      list[i] = { ...op.validation, id: op.id };
      return { kind: "handled", touched: [...old.ranges, ...op.validation.ranges], layout: false };
    }
    case "deleteValidation": {
      const list = x.validations[op.sheet] ?? [];
      const old = list.find((v) => v.id === op.id);
      if (!old) return err("not_found", `no validation ${op.id}`);
      x.validations[op.sheet] = list.filter((v) => v.id !== op.id);
      return { kind: "handled", touched: old.ranges, layout: false };
    }
    case "setNote": {
      const m = x.notes[op.sheet] ?? {};
      m[cellKey(op.r, op.c)] = { ...op.note, r: op.r, c: op.c, created: op.note.created ?? now() };
      x.notes[op.sheet] = m;
      return { kind: "handled", touched: [{ r0: op.r, c0: op.c, r1: op.r, c1: op.c }], layout: false };
    }
    case "deleteNote": {
      const m = x.notes[op.sheet];
      if (!m || !m[cellKey(op.r, op.c)]) return err("not_found", "the cell has no note");
      delete m[cellKey(op.r, op.c)];
      return { kind: "handled", touched: [{ r0: op.r, c0: op.c, r1: op.r, c1: op.c }], layout: false };
    }
    case "setLink": {
      if (/^\s*(javascript|data|vbscript):/i.test(op.url)) return err("invalid_argument", "this kind of link is not allowed");
      const m = x.links[op.sheet] ?? {};
      m[cellKey(op.r, op.c)] = op.url.trim();
      x.links[op.sheet] = m;
      return { kind: "handled", touched: [{ r0: op.r, c0: op.c, r1: op.r, c1: op.c }], layout: false };
    }
    case "clearLink": {
      const m = x.links[op.sheet] ?? {};
      for (const k of Object.keys(m)) {
        const [r, c] = k.split(",").map(Number) as [number, number];
        if (op.ranges.some((rg) => inRange(rg, r, c))) delete m[k];
      }
      return { kind: "handled", touched: op.ranges, layout: false };
    }
    case "protectSheet":
      if (x.protection[op.sheet]) return err("protected", "The sheet is already protected.");
      x.protection[op.sheet] = { password: op.password || null, options: op.options ?? { selectLocked: true, selectUnlocked: true, insertRows: false, insertCols: false, deleteRows: false, deleteCols: false, formatCells: false, sort: false, autoFilter: false } };
      return { kind: "handled", touched: [], layout: true };
    case "unprotectSheet": {
      const p = x.protection[op.sheet];
      if (!p) return { kind: "handled", touched: [], layout: false };
      if (p.password && p.password !== (op.password ?? "")) return err("protected", "The password is not right.");
      x.protection[op.sheet] = null;
      return { kind: "handled", touched: [], layout: true };
    }
    case "setCellProtection": {
      if (x.protection[op.sheet]) return err("protected", "The sheet is protected.");
      if (op.locked !== null) {
        let list = x.unlocked[op.sheet] ?? [];
        if (op.locked) list = list.flatMap((rg) => op.ranges.reduce<RangeDto[]>((acc, cut) => acc.flatMap((a) => subtract(a, cut)), [rg]));
        else list = [...list, ...op.ranges];
        x.unlocked[op.sheet] = list;
      }
      return { kind: "handled", touched: [], layout: false };
    }
    case "protectDocument":
      x.structure = { password: op.password || null };
      return { kind: "handled", touched: [], layout: false, sheetsChanged: true };
    case "unprotectDocument":
      if (x.structure?.password && x.structure.password !== (op.password ?? "")) return err("protected", "The password is not right.");
      x.structure = null;
      return { kind: "handled", touched: [], layout: false, sheetsChanged: true };
    case "setPageSetup": {
      const prev = x.pages[op.sheet] ?? defaultPageSetup();
      x.pages[op.sheet] = { ...op.setup, rowBreaks: prev.rowBreaks, colBreaks: prev.colBreaks };
      return { kind: "handled", touched: [], layout: true };
    }
    case "setPageBreak": {
      if (op.at === 0) return err("invalid_argument", "a page break goes after the first row or column");
      const p = x.pages[op.sheet] ?? defaultPageSetup();
      const list = (op.axis === "row" ? p.rowBreaks : p.colBreaks).filter((b) => b !== op.at);
      if (op.present) list.push(op.at);
      list.sort((a, b) => a - b);
      x.pages[op.sheet] = op.axis === "row" ? { ...p, rowBreaks: list } : { ...p, colBreaks: list };
      return { kind: "handled", touched: [], layout: true };
    }
    case "setDocumentProperties":
      x.props = { ...op.props, created: x.props.created, modified: x.props.modified, generator: x.props.generator };
      return { kind: "handled", touched: [], layout: false, sheetsChanged: true };
    default:
      return { kind: "pass" };
  }
}

/** The 6 locales of the UI (the engine lists its own). */
export const MOCK_LOCALES = [
  { tag: "en-US", decimalSep: ".", groupSep: ",", listSep: ",", currencySymbol: "$" },
  { tag: "en-GB", decimalSep: ".", groupSep: ",", listSep: ",", currencySymbol: "£" },
  { tag: "de-DE", decimalSep: ",", groupSep: ".", listSep: ";", currencySymbol: "€" },
  { tag: "fr-FR", decimalSep: ",", groupSep: " ", listSep: ";", currencySymbol: "€" },
  { tag: "es-ES", decimalSep: ",", groupSep: ".", listSep: ";", currencySymbol: "€" },
  { tag: "ja-JP", decimalSep: ".", groupSep: ",", listSep: ",", currencySymbol: "¥" },
];
