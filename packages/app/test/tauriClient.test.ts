import type { Transaction } from "@tally/bridge";
import { describe, expect, it, vi } from "vitest";
import { COMMANDS, TauriClient, type InvokeArgs, type TauriEvent } from "@tally/bridge/tauri";

function fake(result: unknown = null) {
  const calls: { cmd: string; args: InvokeArgs | undefined }[] = [];
  const handlers = new Map<string, (e: TauriEvent<unknown>) => void>();
  const unlisten = vi.fn();
  const client = new TauriClient({
    invoke: <T,>(cmd: string, args?: InvokeArgs): Promise<T> => {
      calls.push({ cmd, args });
      return result instanceof Error || (typeof result === "object" && result !== null && "reject" in result)
        ? Promise.reject((result as { reject: unknown }).reject ?? result)
        : Promise.resolve(result as T);
    },
    listen: <T,>(event: string, h: (e: TauriEvent<T>) => void) => {
      handlers.set(event, h as (e: TauriEvent<unknown>) => void);
      return Promise.resolve(unlisten);
    },
  });
  return { client, calls, handlers, unlisten };
}

describe("TauriClient", () => {
  it("uses the docs/api.md command names and camelCase arguments", async () => {
    const { client, calls } = fake();
    const sel = { sheet: 1, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 2 }], active: { r: 0, c: 0 } };
    const tx: Transaction = { ops: [], label: "x", selectionBefore: null, selectionAfter: null };
    await client.newWorkbook();
    await client.open("/a.ods");
    await client.save(1, null, "ods");
    await client.save(1, "/b.csv", "csv");
    await client.close(1);
    await client.sheetLayout(1, 2);
    await client.cells(1, 2, [{ r0: 0, c0: 0, r1: 63, c1: 31 }]);
    await client.styles(1, [0, 4]);
    await client.cellEditInfo(1, 2, 3, 4);
    await client.apply(1, tx);
    await client.undo(1);
    await client.redo(1);
    await client.selectionStats(1, sel);
    await client.copy(1, sel);
    await client.navigateEdge(1, 2, { r: 0, c: 0 }, "down");
    await client.resolveNameBox(1, "B12");
    await client.listFunctions();
    await client.assistSuggest(1, "total", sel);
    await client.openDialog();
    await client.saveWithDialog(1, true, null);
    await client.recentFiles();
    await client.listTemplates();
    const query = { text: "Paid", sheet: null, inFormulas: false, matchCase: false, wholeCell: true, mode: "literal" as const, from: null };
    await client.names(1);
    await client.filterInfo(1, 2);
    await client.find(1, query);
    await client.detectHeader(1, 2, { r0: 0, c0: 0, r1: 9, c1: 3 });
    await client.recalcAll(1);
    const req = { formulaSheet: 2, formulaCell: { r: 1, c: 5 }, target: 100, variableSheet: 2, variableCell: { r: 1, c: 3 } };
    await client.conditionalFormats(1, 2);
    await client.validations(1, 2);
    await client.validationAt(1, 2, 3, 4);
    await client.note(1, 2, 3, 4);
    await client.notes(1, 2);
    await client.protectionInfo(1, 2);
    await client.pageSetup(1, 2);
    await client.documentProperties(1);
    await client.goalSeek(1, req);
    await client.listLocales();
    await client.setLocale(1, "de-DE");
    await client.resolveNameBoxIn(1, 2, "Totals");
    await client.openUrl("https://example.com");
    await client.userName();
    expect(calls).toEqual([
      { cmd: "wb_new", args: { template: null } },
      { cmd: "wb_open", args: { path: "/a.ods" } },
      { cmd: "wb_save", args: { doc: 1, saveAs: false, format: "ods" } },
      { cmd: "wb_save", args: { doc: 1, saveAs: true, format: "csv" } },
      { cmd: "wb_close", args: { doc: 1 } },
      { cmd: "sheet_layout", args: { doc: 1, sheet: 2 } },
      { cmd: "get_cells", args: { doc: 1, sheet: 2, ranges: [{ r0: 0, c0: 0, r1: 63, c1: 31 }] } },
      { cmd: "get_styles", args: { doc: 1, ids: [0, 4] } },
      { cmd: "cell_edit_info", args: { doc: 1, sheet: 2, r: 3, c: 4 } },
      { cmd: "apply", args: { doc: 1, tx } },
      { cmd: "undo", args: { doc: 1 } },
      { cmd: "redo", args: { doc: 1 } },
      { cmd: "selection_stats", args: { doc: 1, sel } },
      { cmd: "clipboard_copy", args: { doc: 1, sel } },
      { cmd: "navigate_edge", args: { doc: 1, sheet: 2, from: { r: 0, c: 0 }, dir: "down" } },
      { cmd: "resolve_name_box", args: { doc: 1, text: "B12" } },
      { cmd: "list_functions", args: undefined },
      { cmd: "assist_suggest", args: { doc: 1, query: "total", sel } },
      { cmd: "wb_open_dialog", args: undefined },
      { cmd: "wb_save", args: { doc: 1, saveAs: true, format: null } },
      { cmd: "recent_files", args: undefined },
      { cmd: "list_templates", args: undefined },
      { cmd: "names", args: { doc: 1 } },
      { cmd: "filter_info", args: { doc: 1, sheet: 2 } },
      { cmd: "find", args: { doc: 1, query } },
      { cmd: "detect_header", args: { doc: 1, sheet: 2, range: { r0: 0, c0: 0, r1: 9, c1: 3 } } },
      { cmd: "recalc_all", args: { doc: 1 } },
      { cmd: "conditional_formats", args: { doc: 1, sheet: 2 } },
      { cmd: "validations", args: { doc: 1, sheet: 2 } },
      { cmd: "validation_at", args: { doc: 1, sheet: 2, r: 3, c: 4 } },
      { cmd: "note", args: { doc: 1, sheet: 2, r: 3, c: 4 } },
      { cmd: "notes", args: { doc: 1, sheet: 2 } },
      { cmd: "protection_info", args: { doc: 1, sheet: 2 } },
      { cmd: "page_setup", args: { doc: 1, sheet: 2 } },
      { cmd: "document_properties", args: { doc: 1 } },
      { cmd: "goal_seek", args: { doc: 1, req } },
      { cmd: "list_locales", args: undefined },
      { cmd: "set_locale", args: { doc: 1, locale: "de-DE" } },
      { cmd: "resolve_name_box_in", args: { doc: 1, sheet: 2, text: "Totals" } },
      { cmd: "open_url", args: { url: "https://example.com" } },
      { cmd: "user_name", args: undefined },
    ]);
    expect(new Set(calls.map((c) => c.cmd))).toEqual(new Set(Object.values(COMMANDS)));
  });

  it("rejects with ApiError objects", async () => {
    await expect(fake({ reject: { code: "not_found", message: "no doc" } }).client.undo(9)).rejects.toEqual({
      code: "not_found",
      message: "no doc",
    });
    await expect(fake({ reject: "boom" }).client.undo(9)).rejects.toEqual({ code: "internal", message: "boom" });
    await expect(fake(new Error("bad")).client.undo(9)).rejects.toEqual({ code: "internal", message: "bad" });
  });

  it("forwards event payloads and unlistens", async () => {
    const { client, handlers, unlisten } = fake();
    const seen: string[] = [];
    const off = client.onMenuAction((e) => seen.push(e.id));
    await Promise.resolve();
    handlers.get("menu-action")?.({ payload: { id: "edit.undo" } });
    expect(seen).toEqual(["edit.undo"]);
    expect(handlers.has("doc-changed")).toBe(false);
    off();
    expect(unlisten).toHaveBeenCalledTimes(1);
    client.onDocChanged(() => undefined);
    expect(handlers.has("doc-changed")).toBe(true);
  });
});
