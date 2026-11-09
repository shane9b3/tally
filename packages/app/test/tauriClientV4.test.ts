/**
 * W5-0b: every `EngineClientV4` method has a Tauri command of the docs/api.md
 * name, `TauriClient` invokes it with camelCase arguments, and the Rust side
 * (`commands_v4.rs`, `lib.rs`) defines and registers each command.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { EngineClientV3, EngineClientV4 } from "@tally/bridge";
import { COMMANDS, TauriClient, V4_METHODS, type InvokeArgs, type TauriEvent } from "@tally/bridge/tauri";
import { describe, expect, it } from "vitest";

const SRC_TAURI = new URL("../../../apps/desktop/src-tauri/src/", import.meta.url);
const COMMANDS_V4_RS = readFileSync(fileURLToPath(new URL("commands_v4.rs", SRC_TAURI)), "utf8");
const LIB_RS = readFileSync(fileURLToPath(new URL("lib.rs", SRC_TAURI)), "utf8");

/** Compile-time check: `V4_METHODS` lists every method V4 adds over V3. */
type V4Only = Exclude<keyof EngineClientV4, keyof EngineClientV3>;
type Missing = Exclude<V4Only, (typeof V4_METHODS)[number]>;
const everyV4MethodListed: Missing extends never ? true : false = true;

function fake() {
  const calls: { cmd: string; args: InvokeArgs | undefined }[] = [];
  const client = new TauriClient({
    invoke: <T,>(cmd: string, args?: InvokeArgs): Promise<T> => {
      calls.push({ cmd, args });
      return Promise.resolve((cmd === COMMANDS.printJobPdf ? [37, 80, 68, 70] : null) as T);
    },
    listen: <T,>(_event: string, _h: (e: TauriEvent<T>) => void) => Promise.resolve(() => undefined),
  });
  return { client, calls };
}

describe("EngineClientV4 over Tauri (W5-0b seams)", () => {
  it("lists every V4 method with a snake_case command", () => {
    expect(everyV4MethodListed).toBe(true);
    expect(V4_METHODS.length).toBe(36);
    for (const m of V4_METHODS) {
      const cmd = COMMANDS[m];
      expect(cmd, m).toMatch(/^[a-z]+(_[a-z0-9]+)*$/);
    }
    expect(COMMANDS.exportPdf).toBe("wb_export_pdf");
    expect(COMMANDS.print).toBe("wb_print");
  });

  it("every command is defined in commands_v4.rs and registered in lib.rs", () => {
    const names = [...V4_METHODS.map((m) => COMMANDS[m]), COMMANDS.exportPdf, COMMANDS.print];
    for (const name of names) {
      expect(COMMANDS_V4_RS, name).toMatch(new RegExp(`#\\[tauri::command\\]\\s*pub async fn ${name}\\(`));
      expect(LIB_RS, name).toContain(`commands_v4::${name},`);
    }
    // The Rust constant mirrors the same list (docs and tests over there).
    const rustList = COMMANDS_V4_RS.match(/pub const COMMAND_NAMES: &\[&str\] = &\[([^\]]*)\]/)?.[1] ?? "";
    const rustNames = [...rustList.matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
    expect(rustNames.sort()).toEqual([...names].sort());
  });

  it("invokes each command with the docs/api.md argument names", async () => {
    const { client, calls } = fake();
    const sel = { r0: 0, c0: 0, r1: 5, c1: 2 };
    const opts = { sheets: { type: "active" as const }, selection: null, suppressEmptyPages: true, pageFrom: null, pageTo: null };
    const source = { type: "range" as const, sheet: 2, range: sel };
    const req = {
      objective: { sheet: 2, r: 0, c: 0 },
      goal: { type: "maximize" as const },
      variables: [],
      constraints: [],
      options: { assumeNonNegative: true, integer: false, epsilon: 1e-6, maxIterations: 100, timeoutMs: 1000 },
    };
    await client.charts(1, 2);
    await client.chartData(1, 2, 3);
    await client.suggestChart(1, 2, sel, "column");
    await client.images(1, 2);
    await client.pivots(1);
    await client.pivotFieldItems(1, source);
    await client.pivotAt(1, 2, 3, 4);
    await client.autocomplete(1, 2, 3, 4, "Ap");
    await client.cellStyles(1);
    await client.scenarios(1, 2);
    await client.databaseRanges(1);
    await client.navigator(1);
    await client.documentStatistics(1);
    await client.printLayout(1, opts);
    await client.pageBreaks(1, 2);
    await client.preparePrintJob(1, opts, []);
    await client.printJobPagePng(7, 0, 96);
    const pdf = await client.printJobPdf(7);
    await client.calcSettings(1);
    await client.solver(1, req);
    await client.precedents(1, 2, 3, 4);
    await client.dependents(1, 2, 3, 4);
    await client.detective(1, 2, 3, 4, "tracePrecedents");
    await client.detectiveArrows(1, 2);
    await client.setDetectiveAuto(1, false);
    await client.setViewZoom(1, 20);
    await client.undoSteps(1, 3);
    await client.redoSteps(1, 2);
    await client.undoHistory(1);
    await client.setUndoDepth(250);
    await client.imageData(1, 2, 3);
    await client.viewOptions(1, 2);
    await client.invalidCells(1, 2, null);
    await client.customSortLists(1);
    await client.cellsWith(1, 2, [sel], { showFormulas: true, valueHighlighting: false, hideZeroValues: true });
    await client.previewFormula(1, 2, 3, 4, "=SUM(A1:A3)");
    await client.exportPdf(1, 7);
    await client.print(1, 7);
    expect(pdf).toBeInstanceOf(Uint8Array);
    expect([...pdf]).toEqual([37, 80, 68, 70]);
    expect(calls).toEqual([
      { cmd: "charts", args: { doc: 1, sheet: 2 } },
      { cmd: "chart_data", args: { doc: 1, sheet: 2, id: 3 } },
      { cmd: "suggest_chart", args: { doc: 1, sheet: 2, sel, kind: "column" } },
      { cmd: "images", args: { doc: 1, sheet: 2 } },
      { cmd: "pivots", args: { doc: 1 } },
      { cmd: "pivot_field_items", args: { doc: 1, source } },
      { cmd: "pivot_at", args: { doc: 1, sheet: 2, r: 3, c: 4 } },
      { cmd: "autocomplete", args: { doc: 1, sheet: 2, r: 3, c: 4, prefix: "Ap" } },
      { cmd: "cell_styles", args: { doc: 1 } },
      { cmd: "scenarios", args: { doc: 1, sheet: 2 } },
      { cmd: "database_ranges", args: { doc: 1 } },
      { cmd: "navigator", args: { doc: 1 } },
      { cmd: "document_statistics", args: { doc: 1 } },
      { cmd: "print_layout", args: { doc: 1, opts } },
      { cmd: "page_breaks", args: { doc: 1, sheet: 2 } },
      { cmd: "prepare_print_job", args: { doc: 1, opts, charts: [] } },
      { cmd: "print_job_page_png", args: { job: 7, page: 0, dpi: 96 } },
      { cmd: "print_job_pdf", args: { job: 7 } },
      { cmd: "calc_settings", args: { doc: 1 } },
      { cmd: "solver", args: { doc: 1, req } },
      { cmd: "precedents", args: { doc: 1, sheet: 2, r: 3, c: 4 } },
      { cmd: "dependents", args: { doc: 1, sheet: 2, r: 3, c: 4 } },
      { cmd: "detective", args: { doc: 1, sheet: 2, r: 3, c: 4, action: "tracePrecedents" } },
      { cmd: "detective_arrows", args: { doc: 1, sheet: 2 } },
      { cmd: "set_detective_auto", args: { doc: 1, on: false } },
      { cmd: "set_view_zoom", args: { doc: 1, zoom: 20 } },
      { cmd: "undo_steps", args: { doc: 1, count: 3 } },
      { cmd: "redo_steps", args: { doc: 1, count: 2 } },
      { cmd: "undo_history", args: { doc: 1 } },
      { cmd: "set_undo_depth", args: { depth: 250 } },
      { cmd: "image_data", args: { doc: 1, sheet: 2, id: 3 } },
      { cmd: "view_options", args: { doc: 1, sheet: 2 } },
      { cmd: "invalid_cells", args: { doc: 1, sheet: 2, range: null } },
      { cmd: "custom_sort_lists", args: { doc: 1 } },
      { cmd: "cells_with", args: { doc: 1, sheet: 2, ranges: [sel], opts: { showFormulas: true, valueHighlighting: false, hideZeroValues: true } } },
      { cmd: "preview_formula", args: { doc: 1, sheet: 2, r: 3, c: 4, text: "=SUM(A1:A3)" } },
      { cmd: "wb_export_pdf", args: { doc: 1, job: 7 } },
      { cmd: "wb_print", args: { doc: 1, job: 7 } },
    ]);
    // Every V4 method was exercised above (keeps the list honest).
    const invoked = new Set(calls.map((c) => c.cmd));
    for (const m of V4_METHODS) expect(invoked.has(COMMANDS[m]), m).toBe(true);
  });
});
