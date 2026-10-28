/**
 * Multi-sheet selection ("grouped sheets") for the mock engines, mirroring
 * the engine's `group.rs`: when a transaction carries a `group` of more than
 * one sheet, every whitelisted op aimed at `group[0]` is applied to each
 * group sheet at the same addresses, in group order. Other ops run once.
 * The caller makes the whole transaction all-or-nothing and one undo step;
 * errors keep the index of the op in the ORIGINAL transaction.
 */
import type { Op } from "./generated/Op";
import type { Transaction } from "./generated/Transaction";

/** Sheet id (the engine's `SheetId`). */
type SheetId = number;

/** Op `type` tags repeated on every grouped sheet (the engine's `GROUP_OPS`). */
export const GROUP_OPS: readonly string[] = [
  "setCellInput",
  "setRangeInputs",
  "setArrayFormula",
  "clear",
  "deleteContents",
  "applyStyle",
  "setNumberFormat",
  "applyCellStyle",
  "mergeCells",
  "unmergeCells",
  "setCellProtection",
  "addConditionalFormat",
  "addValidation",
  "insertRows",
  "deleteRows",
  "insertCols",
  "deleteCols",
  "insertCells",
  "deleteCells",
  "setColWidth",
  "setRowHeight",
  "setRowsHidden",
  "setColsHidden",
  "autoFitRows",
  "paste",
  "pasteSpecial",
  "fillRange",
  "fillSeries",
  "replaceAll",
];

/** The `fillSheets` op (Edit ▸ Fill ▸ Sheets). */
export type FillSheetsOp = Extract<Op, { type: "fillSheets" }>;

/** An op of an expanded transaction with its index in the original one. */
export interface IndexedOp {
  op: Op;
  index: number;
}

/** The op's target sheet, when it has a numeric `sheet`. */
function sheetOf(op: Op): SheetId | null {
  const s = (op as { sheet?: unknown }).sheet;
  return typeof s === "number" ? s : null;
}

/** `op` aimed at `sheet` instead (same addresses). */
export function retarget(op: Op, sheet: SheetId): Op {
  if (op.type === "replaceAll") return { ...op, sheet, query: { ...op.query, sheet } };
  return { ...structuredClone(op), sheet } as Op;
}

/** The group of a transaction when it has more than one sheet, else null. */
export function groupOf(tx: Transaction): SheetId[] | null {
  const g = tx.group;
  return g && g.length > 1 ? g : null;
}

/** A sheet as `groupSheets` sees it (`null`: no such sheet). */
export type GroupSheetLookup = (id: SheetId) => { hidden: boolean } | null;

/**
 * The engine's `group_sheets`: `null` without a group of more than one
 * sheet (the transaction is then not atomic); otherwise the group cleaned
 * up — duplicates dropped, hidden sheets dropped except `group[0]` — which
 * may come down to one sheet (still atomic, nothing repeats). An unknown
 * id throws `not_found` (reported at op index 0, before anything runs).
 */
export function groupSheets(tx: Transaction, sheet: GroupSheetLookup): SheetId[] | null {
  const group = groupOf(tx);
  if (!group) return null;
  const out: SheetId[] = [];
  group.forEach((id, i) => {
    const s = sheet(id);
    if (!s) throw { code: "not_found", message: `no sheet ${id}` };
    if (out.includes(id) || (i > 0 && s.hidden)) return;
    out.push(id);
  });
  return out;
}

/**
 * The ops of `tx` with grouped ops repeated on every sheet of `group`
 * (the result of {@link groupSheets}; in group order), each with its index
 * in `tx.ops`. Without a group of two or more the ops pass as is.
 */
export function expandGroup(tx: Transaction, group: readonly SheetId[] | null = groupOf(tx)): IndexedOp[] {
  const g = group && group.length > 1 ? group : null;
  const out: IndexedOp[] = [];
  tx.ops.forEach((op, index) => {
    out.push({ op, index });
    if (!g || !GROUP_OPS.includes(op.type) || sheetOf(op) !== g[0]) return;
    for (const sheet of g.slice(1)) out.push({ op: retarget(op, sheet), index });
  });
  return out;
}
