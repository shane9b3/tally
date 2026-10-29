import { describe, expect, it } from "vitest";
import { BUILTIN_TEMPLATES, CSV_OPTIONS, isApiError, MockClient, type Op, type SelectionDto, type ShellClient, type Transaction } from "../src/index";

const tx = (...ops: Op[]): Transaction => ({ ops, label: null, selectionBefore: null, selectionAfter: null });
const set = (sheet: number, r: number, c: number, input: string): Op => ({ type: "setCellInput", sheet, r, c, input });
const sel = (sheet: number, r0: number, c0: number, r1: number, c1: number): SelectionDto => ({
  sheet,
  ranges: [{ r0, c0, r1, c1 }],
  active: { r: r0, c: c0 },
});

async function setup() {
  const client = new MockClient();
  const info = await client.newWorkbook();
  const sheet = info.activeSheet;
  return { client, doc: info.doc, sheet };
}

async function text(client: MockClient, doc: number, sheet: number, r: number, c: number) {
  const [block] = await client.cells(doc, sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
  return block?.cells[0]?.text ?? "";
}

describe("MockClient", () => {
  it("creates a workbook with one sheet", async () => {
    const client = new MockClient();
    const info = await client.newWorkbook();
    expect(info.sheets).toHaveLength(1);
    expect(info.sheets[0]?.name).toBe("Sheet1");
    expect(info.dirty).toBe(false);
  });

  it("evaluates the supported formulas", async () => {
    const { client, doc, sheet } = await setup();
    const res = await client.apply(
      doc,
      tx(set(sheet, 0, 0, "2"), set(sheet, 0, 1, "3"), set(sheet, 0, 2, "=A1*B1"), set(sheet, 0, 3, "=a1+b1")),
    );
    expect(res.errors).toEqual([]);
    expect(await text(client, doc, sheet, 0, 2)).toBe("6");
    expect(await text(client, doc, sheet, 0, 3)).toBe("5");
    await client.apply(doc, tx(set(sheet, 1, 0, "=SUM(A1:D1)"), set(sheet, 1, 1, "=A1/0"), set(sheet, 1, 2, "=FOO()")));
    expect(await text(client, doc, sheet, 1, 0)).toBe("16");
    expect(await text(client, doc, sheet, 1, 1)).toBe("#DIV/0!");
    expect(await text(client, doc, sheet, 1, 2)).toBe("#NAME?");
    // Dependents are fresh after an edit.
    await client.apply(doc, tx(set(sheet, 0, 0, "10")));
    expect(await text(client, doc, sheet, 0, 2)).toBe("30");
  });

  it("evaluates aggregates over sheet and 3D references in tab order", async () => {
    const { client, doc, sheet } = await setup();
    const added = await client.apply(
      doc,
      tx({ type: "addSheet", name: null, index: null }, { type: "addSheet", name: "My Sheet", index: null }),
    );
    const [, s2, s3] = added.info?.sheets.map((s) => s.id) ?? [];
    if (s2 === undefined || s3 === undefined) throw new Error("sheets");
    await client.apply(doc, tx(set(sheet, 0, 0, "10"), set(s2, 0, 0, "20"), set(s2, 1, 0, "x"), set(s3, 0, 0, "30")));
    await client.apply(
      doc,
      tx(
        set(sheet, 0, 1, "=SUM(Sheet1.A1:'My Sheet'.A2)"),
        set(sheet, 0, 2, "=COUNT($Sheet1.A1:$Sheet2.A2)"),
        set(sheet, 0, 3, "=AVERAGE('My Sheet'.A1:Sheet1.A1)"),
        set(sheet, 0, 4, "=MIN(Sheet2.A1:A2)"),
        set(sheet, 0, 5, "=MAX(Sheet1.A1:Nope.A1)"),
      ),
    );
    expect(await text(client, doc, sheet, 0, 1)).toBe("60");
    expect(await text(client, doc, sheet, 0, 2)).toBe("2");
    expect(await text(client, doc, sheet, 0, 3)).toBe("20");
    expect(await text(client, doc, sheet, 0, 4)).toBe("20");
    expect(await text(client, doc, sheet, 0, 5)).toBe("#NAME?");
    // Moving the last sheet first narrows the span to Sheet1 only... and back.
    await client.apply(doc, tx({ type: "moveSheet", sheet: s3, to: 0 }));
    expect(await text(client, doc, sheet, 0, 1)).toBe("40");
    // A cycle across sheets is caught.
    await client.apply(doc, tx(set(s2, 2, 0, "=SUM(Sheet1.G1)"), set(sheet, 0, 6, "=SUM(Sheet2.A3)")));
    expect(await text(client, doc, sheet, 0, 6)).toBe("Err:522");
  });

  it("detects cycles and text in arithmetic", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "=B1+1"), set(sheet, 0, 1, "=A1+1"), set(sheet, 1, 0, "x"), set(sheet, 1, 1, "=A2*2")));
    expect(await text(client, doc, sheet, 0, 0)).toBe("Err:522");
    expect(await text(client, doc, sheet, 1, 1)).toBe("#VALUE!");
  });

  it("reports cell views with kinds, flags and edit info", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "1.5"), set(sheet, 0, 1, "hello"), set(sheet, 0, 2, "true"), set(sheet, 0, 3, "=A1*A1")));
    const [block] = await client.cells(doc, sheet, [{ r0: 0, c0: 0, r1: 10, c1: 10 }]);
    expect(block?.cells.map((c) => c.kind)).toEqual(["number", "text", "bool", "number"]);
    expect(block?.cells[3]?.flags).toBe(1);
    expect(block?.cells[3]?.num).toBe(2.25);
    expect(await client.cellEditInfo(doc, sheet, 0, 3)).toEqual({ input: "=A1*A1", hint: "A1 × A1" });
  });

  it("applies bold and italic as deduplicated styles", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "x")));
    const ranges = [{ r0: 0, c0: 0, r1: 0, c1: 1 }];
    const patch = { bold: true, italic: null, underline: null, strike: null, fontFamily: null, fontSizePt: null, color: null, fill: null, hAlign: null, vAlign: null, wrap: null, numberFormat: null };
    await client.apply(doc, tx({ type: "applyStyle", sheet, ranges, patch }));
    const [block] = await client.cells(doc, sheet, ranges);
    expect(block?.cells).toHaveLength(2); // styled empty cell B1 is included
    const ids = [...new Set(block?.cells.map((c) => c.style))];
    expect(ids).toHaveLength(1);
    const [style] = await client.styles(doc, ids);
    expect(style?.bold).toBe(true);
    expect(style?.italic).toBe(false);
    await client.apply(doc, tx({ type: "applyStyle", sheet, ranges, patch: { ...patch, bold: null, italic: true } }));
    const [after] = await client.cells(doc, sheet, ranges);
    const [both] = await client.styles(doc, [after?.cells[0]?.style ?? -1]);
    expect(both?.bold && both.italic).toBe(true);
  });

  it("adds, renames, moves and deletes sheets", async () => {
    const { client, doc, sheet } = await setup();
    const added = await client.apply(doc, tx({ type: "addSheet", name: null, index: null }));
    expect(added.sheetsChanged).toBe(true);
    expect(added.info?.sheets.map((s) => s.name)).toEqual(["Sheet1", "Sheet2"]);
    const second = added.info?.sheets[1]?.id ?? -1;
    const renamed = await client.apply(doc, tx({ type: "renameSheet", sheet: second, name: "Data" }));
    expect(renamed.info?.sheets.map((s) => s.name)).toEqual(["Sheet1", "Data"]);
    const dup = await client.apply(doc, tx({ type: "renameSheet", sheet, name: "data" }));
    expect(dup.errors[0]?.code).toBe("invalid_argument");
    const moved = await client.apply(doc, tx({ type: "moveSheet", sheet: second, to: 0 }));
    expect(moved.info?.sheets.map((s) => s.name)).toEqual(["Data", "Sheet1"]);
    const deleted = await client.apply(doc, tx({ type: "deleteSheet", sheet: second }));
    expect(deleted.info?.sheets.map((s) => s.name)).toEqual(["Sheet1"]);
    const last = await client.apply(doc, tx({ type: "deleteSheet", sheet }));
    expect(last.errors).toHaveLength(1);
  });

  it("undoes and redoes transactions", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, { ...tx(set(sheet, 0, 0, "1")), label: "Type" });
    const r2 = await client.apply(doc, { ...tx(set(sheet, 0, 0, "2")), label: "Type again" });
    expect(r2.undo).toEqual({ canUndo: true, canRedo: false, undoLabel: "Type again", redoLabel: null });
    const u = await client.undo(doc);
    expect(u.undo.canRedo).toBe(true);
    expect(await text(client, doc, sheet, 0, 0)).toBe("1");
    await client.undo(doc);
    expect(await text(client, doc, sheet, 0, 0)).toBe("");
    await expect(client.undo(doc)).rejects.toMatchObject({ code: "invalid_argument" });
    await client.redo(doc);
    await client.redo(doc);
    expect(await text(client, doc, sheet, 0, 0)).toBe("2");
    // A new edit clears the redo stack.
    await client.undo(doc);
    const r = await client.apply(doc, tx(set(sheet, 5, 5, "z")));
    expect(r.undo.canRedo).toBe(false);
  });

  it("computes selection stats", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "2"), set(sheet, 0, 1, "3"), set(sheet, 0, 2, "=A1*B1"), set(sheet, 0, 3, "note")));
    const stats = await client.selectionStats(doc, sel(sheet, 0, 0, 0, 3));
    expect(stats).toEqual({ sum: 11, count: 4, countNumbers: 3, average: 11 / 3, min: 2, max: 6, formattedSum: "11" });
    const empty = await client.selectionStats(doc, sel(sheet, 9, 9, 9, 9));
    expect(empty.sum).toBeNull();
    expect(empty.count).toBe(0);
  });

  it("returns a default layout and records column widths", async () => {
    const { client, doc, sheet } = await setup();
    let layout = await client.sheetLayout(doc, sheet);
    expect(layout.defaultColWidthPx).toBeGreaterThan(0);
    expect(layout.used).toBeNull();
    const res = await client.apply(doc, tx({ type: "setColWidth", sheet, start: 2, end: 4, px: 150 }, set(sheet, 3, 1, "x")));
    expect(res.layoutChanged).toEqual([sheet]);
    layout = await client.sheetLayout(doc, sheet);
    expect(layout.colRuns).toEqual([{ start: 2, end: 4, size: 150, hidden: false }]);
    expect(layout.used).toEqual({ r0: 3, c0: 1, r1: 3, c1: 1 });
  });

  it("navigates edges, resolves the name box, copies and pastes", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "1"), set(sheet, 1, 0, "2"), set(sheet, 2, 0, "3"), set(sheet, 9, 0, "4")));
    expect(await client.navigateEdge(doc, sheet, { r: 0, c: 0 }, "down")).toEqual({ r: 2, c: 0 });
    expect(await client.navigateEdge(doc, sheet, { r: 2, c: 0 }, "down")).toEqual({ r: 9, c: 0 });
    expect(await client.navigateEdge(doc, sheet, { r: 0, c: 0 }, "up")).toEqual({ r: 0, c: 0 });
    expect(await client.navigateEdge(doc, sheet, { r: 0, c: 0 }, "right")).toEqual({ r: 0, c: 16383 });
    expect((await client.resolveNameBox(doc, "b2:a1")).ranges).toEqual([{ r0: 0, c0: 0, r1: 1, c1: 1 }]);
    expect((await client.resolveNameBox(doc, "Sheet1.C3")).active).toEqual({ r: 2, c: 2 });
    await expect(client.resolveNameBox(doc, "nope")).rejects.toSatisfy(isApiError);
    const clip = await client.copy(doc, sel(sheet, 0, 0, 1, 0));
    expect(clip.tsv).toBe("1\n2");
    await client.apply(doc, tx({ type: "paste", sheet, target: { r0: 0, c0: 5, r1: 0, c1: 5 }, payload: { type: "internal", token: clip.token } }));
    await client.apply(doc, tx({ type: "paste", sheet, target: { r0: 0, c0: 6, r1: 0, c1: 6 }, payload: { type: "tsv", text: "7\t8\n" } }));
    expect(await text(client, doc, sheet, 1, 5)).toBe("2");
    expect(await text(client, doc, sheet, 0, 7)).toBe("8");
  });

  it("rejects unknown documents and unsupported ops with ApiError", async () => {
    const { client, doc, sheet } = await setup();
    await expect(client.sheetLayout(999, sheet)).rejects.toMatchObject({ code: "not_found" });
    const res = await client.apply(doc, tx({ type: "insertRows", sheet, at: 0, count: 1 }));
    expect(res.errors[0]).toMatchObject({ opIndex: 0, code: "unsupported" });
    expect(await client.assistSuggest(doc, "sum", sel(sheet, 0, 0, 0, 0))).toEqual([]);
    expect((await client.listFunctions()).map((f) => f.name)).toContain("SUM");
    await client.close(doc);
    await expect(client.close(doc)).rejects.toSatisfy(isApiError);
  });

  it("delivers events to subscribers until unsubscribed", async () => {
    const client = new MockClient();
    const seen: string[] = [];
    const off = client.onMenuAction((e) => seen.push(e.id));
    client.emitMenuAction("edit.undo");
    off();
    client.emitMenuAction("edit.redo");
    expect(seen).toEqual(["edit.undo"]);
  });

  it("builds the engine's templates by id and implements ShellClient", async () => {
    const client = new MockClient();
    const shell: ShellClient = client;
    expect((await shell.listTemplates()).map((t) => [t.id, t.title])).toEqual([
      ["invoices", "Invoice tracker"],
      ["budget", "Monthly budget"],
      ["stock", "Stock count"],
    ]);
    expect(BUILTIN_TEMPLATES.map((t) => t.id)).toEqual(["invoices", "budget", "stock"]);
    const inv = await client.newWorkbook("invoices");
    expect(inv.title).toBe("Invoices");
    expect(inv.sheets[0]?.name).toBe("Orders");
    expect(inv.dirty).toBe(false);
    expect(await text(client, inv.doc, inv.activeSheet, 0, 5)).toBe("Total");
    expect(await text(client, inv.doc, inv.activeSheet, 1, 5)).toBe("54");
    const [style] = await client.styles(inv.doc, [(await client.cells(inv.doc, inv.activeSheet, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]))[0]?.cells[0]?.style ?? 0]);
    expect(style?.bold).toBe(true);
    const stock = await client.newWorkbook("stock");
    expect(await text(client, stock.doc, stock.activeSheet, 3, 6)).toBe("Low");
    const err: unknown = await client.newWorkbook("nope").catch((e: unknown) => e);
    expect(isApiError(err) && err.code).toBe("not_found");
    expect(await shell.recentFiles()).toEqual([]);
    const openErr: unknown = await shell.openDialog().catch((e: unknown) => e);
    expect(isApiError(openErr) && openErr.code).toBe("unsupported");
    // Like the shell, a picked CSV file waits for the Text Export options.
    const pending: unknown = await shell.saveWithDialog(inv.doc, true, "csv").catch((e: unknown) => e);
    expect(isApiError(pending) && pending.code).toBe(CSV_OPTIONS);
    expect(isApiError(pending) && pending.message).toBe("Invoices.csv");
    const csv = { fieldDelimiter: ";", textDelimiter: '"', quoteAllText: true, asShown: false, charset: "windows1252" as const };
    const saved = await shell.saveCsv(inv.doc, csv);
    expect(saved?.title).toBe("Invoices.csv");
    expect(saved?.dirty).toBe(false);
    expect(client.savedCsv.at(-1)).toEqual({ doc: inv.doc, title: "Invoices.csv", csv });
    // Cancelled options: nothing is saved.
    await shell.saveWithDialog(inv.doc, true, "csv").catch(() => null);
    expect(await shell.saveCsv(inv.doc, null)).toBeNull();
    expect(client.savedCsv).toHaveLength(1);
    // Options given up front save at once.
    const direct = await shell.saveWithDialog(inv.doc, true, "csv", csv);
    expect(direct.title).toBe("Invoices.csv");
  });
});

describe("MockClient v0.3 stubs", () => {
  it("stores notes, links, validations and conditional formats", async () => {
    const { client, doc, sheet } = await setup();
    const res = await client.apply(
      doc,
      tx(
        { type: "setNote", sheet, r: 1, c: 1, note: { r: 0, c: 0, text: "hi", author: "Ana", created: null, shown: false } },
        { type: "setLink", sheet, r: 0, c: 0, url: "https://example.com" },
        {
          type: "addValidation",
          sheet,
          validation: {
            id: 0,
            ranges: [{ r0: 0, c0: 2, r1: 9, c1: 2 }],
            kind: { type: "list", source: { type: "values", items: ["a", "b"] } },
            allowBlank: true,
            showDropdown: true,
            inputHelp: null,
            error: null,
          },
        },
        {
          type: "addConditionalFormat",
          sheet,
          ranges: [{ r0: 0, c0: 0, r1: 9, c1: 0 }],
          rules: [
            {
              kind: { type: "cellValue", op: "gt", formulas: ["5"] },
              style: { bold: true, italic: null, underline: null, strike: null, color: null, fill: null, borders: null, numberFormat: null, display: null },
              stopIfTrue: false,
            },
          ],
        },
      ),
    );
    expect(res.errors).toEqual([]);
    expect((await client.note(doc, sheet, 1, 1))?.text).toBe("hi");
    expect(await client.notes(doc, sheet)).toHaveLength(1);
    const [block] = await client.cells(doc, sheet, [{ r0: 0, c0: 0, r1: 1, c1: 1 }]);
    expect(block?.cells.find((c) => c.r === 0 && c.c === 0)?.link).toBe("https://example.com");
    expect((block?.cells.find((c) => c.r === 1 && c.c === 1)?.flags ?? 0) & 8).toBe(8);
    expect((await client.validationAt(doc, sheet, 3, 2))?.listItems).toEqual(["a", "b"]);
    expect(await client.conditionalFormats(doc, sheet)).toHaveLength(1);
    expect((await client.pageSetup(doc, sheet)).paper).toEqual({ type: "a4" });
    expect((await client.protectionInfo(doc, sheet)).sheetProtected).toBe(false);
  });
});
