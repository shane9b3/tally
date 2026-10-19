/**
 * `WebMockClient`: the browser demo/e2e engine. It is @tally/bridge's
 * `MockClient` plus emulated insert/delete of rows and columns (which the
 * mock rejects as `unsupported`): each structural op is rewritten, through
 * the public client API, into clear + `setRangeInputs` + `applyStyle` ops
 * that shift the cells, so the op applies as one undo step.
 *
 * Demo only: formulas are NOT re-anchored and row/column sizes do not
 * move. The real engine implements these ops natively. Remove this file
 * once MockClient supports structural ops.
 */
import { MockClient, type ApplyResult, type DocId, type Op, type RangeDto, type StylePatch, type Transaction } from "@tally/bridge";

type Structural = Extract<Op, { type: "insertRows" | "deleteRows" | "insertCols" | "deleteCols" }>;

function isStructural(op: Op): op is Structural {
  return op.type === "insertRows" || op.type === "deleteRows" || op.type === "insertCols" || op.type === "deleteCols";
}

export class WebMockClient extends MockClient {
  override async apply(doc: DocId, tx: Transaction): Promise<ApplyResult> {
    if (!tx.ops.some(isStructural)) return super.apply(doc, tx);
    if (tx.ops.length === 1) return super.apply(doc, { ...tx, ops: await this.rewrite(doc, tx.ops[0] as Structural) });
    // Mixed transactions: one step per op (later ops see the shifted cells).
    let last: ApplyResult | null = null;
    for (const op of tx.ops) {
      const ops = isStructural(op) ? await this.rewrite(doc, op) : [op];
      last = await super.apply(doc, { ...tx, ops });
    }
    return last as ApplyResult;
  }

  /** Rewrites one structural op into cell ops over the used range. */
  private async rewrite(doc: DocId, op: Structural): Promise<Op[]> {
    const sheet = op.sheet;
    const used = (await this.sheetLayout(doc, sheet)).used;
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
      const info = await this.cellEditInfo(doc, sheet, cell.r, cell.c);
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
        };
        ops.push({ type: "applyStyle", sheet, ranges: byStyle.get(st.id) ?? [], patch });
      }
    }
    return ops;
  }
}
