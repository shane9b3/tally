/**
 * Load Styles: the Tauri commands (templates.rs / dialogs.rs, ACL
 * manifest), the dialog's validity rule, the browser mock's plan (the
 * engine's `loadstyles.rs` semantics: overwrite, parents, page style) and
 * the mock's one-step undo.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { CellStyleDto, PageSetupDto, StyleDto } from "@tally/bridge";
import { COMMANDS, LOAD_STYLES_SHELL_COMMANDS, TauriClient, type InvokeArgs, type TauriEvent } from "@tally/bridge/tauri";
import { describe, expect, it } from "vitest";
import { WebMockClient } from "../src/dev/webMock";
import { loadStylesValid, pageStyleOn, planLoadStyles, rebaseStyle } from "../src/styles/loadStyles";
import { defaultPageSetup } from "../src/v03";

const SRC = new URL("../../../apps/desktop/src-tauri/", import.meta.url);
const read = (p: string) => readFileSync(fileURLToPath(new URL(p, SRC)), "utf8");

const BASE: StyleDto = { id: 0, bold: false, italic: false, underline: false, strike: false, fontFamily: null, fontSizePt: 11, color: null, fill: null, hAlign: "general", vAlign: "bottom", wrap: false, indent: 0, borders: { top: null, bottom: null, left: null, right: null }, numberFormat: "General", display: null };
const style = (name: string, parent: string | null, patch: Partial<StyleDto> = {}, builtin = false): CellStyleDto => ({ name, parent, builtin, style: { ...BASE, ...patch } });

describe("Load Styles commands", () => {
  it("are registered, defined and granted", () => {
    const lib = read("src/lib.rs");
    const acl = read("app_commands.rs");
    const caps = read("capabilities/default.json");
    const modules: Record<string, string> = { load_styles_from_template: "src/templates.rs", load_styles_dialog: "src/dialogs.rs" };
    for (const m of LOAD_STYLES_SHELL_COMMANDS) {
      const name = COMMANDS[m];
      expect(lib, name).toMatch(new RegExp(`::${name},`));
      expect(acl, name).toContain(`"${name}"`);
      expect(caps, name).toContain(`"allow-${name.replace(/_/g, "-")}"`);
      expect(read(modules[name] ?? ""), name).toMatch(new RegExp(`#\\[tauri::command\\]\\s*pub async fn ${name}\\(`));
    }
  });

  it("invoke with the documented argument names", async () => {
    const calls: { cmd: string; args: InvokeArgs | undefined }[] = [];
    const client = new TauriClient({
      invoke: <T,>(cmd: string, args?: InvokeArgs): Promise<T> => {
        calls.push({ cmd, args });
        return Promise.resolve(null as T);
      },
      listen: <T,>(_e: string, _h: (e: TauriEvent<T>) => void) => Promise.resolve(() => undefined),
    });
    const opts = { cellStyles: true, pageStyles: false, overwrite: true };
    await client.loadStylesFromTemplate(3, "user:Budget", opts, "Load Styles");
    await client.loadStylesFromFile(3, opts, null);
    expect(calls).toEqual([
      { cmd: "load_styles_from_template", args: { doc: 3, id: "user:Budget", opts, label: "Load Styles" } },
      { cmd: "load_styles_dialog", args: { doc: 3, opts, label: null } },
    ]);
  });
});

describe("Load Styles rules", () => {
  it("needs a source and something to load (page styles only with Overwrite)", () => {
    expect(loadStylesValid({ cellStyles: true, pageStyles: false, overwrite: false }, null)).toBe(false);
    expect(loadStylesValid({ cellStyles: true, pageStyles: false, overwrite: false }, "invoices")).toBe(true);
    expect(loadStylesValid({ cellStyles: false, pageStyles: true, overwrite: false }, "invoices")).toBe(false);
    expect(loadStylesValid({ cellStyles: false, pageStyles: true, overwrite: true }, "file")).toBe(true);
    expect(loadStylesValid({ cellStyles: false, pageStyles: false, overwrite: true }, "file")).toBe(false);
  });

  it("adds new names only without Overwrite, replaces with it", () => {
    const current = [style("Default", null, {}, true), style("Heading", "Default", { bold: true }, true)];
    const source = [style("Default", null, {}, true), style("Heading", "Default", { bold: true, color: 0xff0000ff }, true), style("Mine", "Default", { italic: true })];
    expect(planLoadStyles(current, source, false).map((s) => s.name)).toEqual(["Mine"]);
    const all = planLoadStyles(current, source, true);
    expect(all.map((s) => s.name)).toEqual(["Default", "Heading", "Mine"]);
    expect(all.find((s) => s.name === "Heading")?.style.color).toBe(0xff0000ff);
    expect(all.find((s) => s.name === "Heading")?.builtin).toBe(true);
  });

  it("creates parents first and re-bases onto this document's parent", () => {
    const current = [style("Default", null, {}, true), style("Heading", "Default", { bold: true, fontSizePt: 16 }, true)];
    const source = [style("Default", null, {}, true), style("Heading", "Default", { italic: true }, true), style("Sub", "Heading", { italic: true, strike: true }), style("Child", "Base", { fill: 0x00ff00ff, bold: true }), style("Base", "Default", { fill: 0x00ff00ff })];
    const plan = planLoadStyles(current, source, false);
    expect(plan.map((s) => s.name)).toEqual(["Base", "Sub", "Child"]);
    // Sub: its own strike from the source, Heading's attributes from here.
    const sub = plan.find((s) => s.name === "Sub")!;
    expect(sub.style).toMatchObject({ strike: true, bold: true, fontSizePt: 16, italic: false });
    expect(plan.find((s) => s.name === "Child")?.parent).toBe("Base");
    expect(plan.indexOf(plan.find((s) => s.name === "Base")!)).toBeLessThan(plan.indexOf(plan.find((s) => s.name === "Child")!));
  });

  it("this document's children follow a replaced parent; cycles and missing parents fall back to Default", () => {
    const current = [style("Default", null, {}, true), style("Heading", "Default", { bold: true }, true), style("Mine", "Heading", { bold: true, underline: true })];
    const source = [style("Default", null, {}, true), style("Heading", "Default", { bold: true, color: 0xff0000ff }, true), style("X", "Y"), style("Y", "X"), style("Z", "Nowhere")];
    const plan = planLoadStyles(current, source, true);
    expect(plan.find((s) => s.name === "Mine")?.style).toMatchObject({ underline: true, color: 0xff0000ff });
    expect(plan.find((s) => s.name === "Z")?.parent).toBe("Default");
    const x = plan.find((s) => s.name === "X")!;
    const y = plan.find((s) => s.name === "Y")!;
    expect(x.parent === "Default" || y.parent === "Default").toBe(true);
  });

  it("re-bases attribute by attribute and keeps the sheet's print ranges", () => {
    expect(rebaseStyle({ ...BASE, bold: true }, { ...BASE, bold: true, italic: true }, { ...BASE, color: 1 })).toMatchObject({ bold: false, italic: true, color: 1 });
    const cur: PageSetupDto = { ...defaultPageSetup(), printRange: { r0: 0, c0: 0, r1: 9, c1: 2 }, rowBreaks: [5] };
    const src: PageSetupDto = { ...defaultPageSetup(), orientation: "landscape", printGridlines: true };
    expect(pageStyleOn(src, cur)).toMatchObject({ orientation: "landscape", printGridlines: true, printRange: cur.printRange, rowBreaks: [5] });
  });
});

describe("Load Styles in the browser mock", () => {
  it("loads a built-in template's styles and page format as one undo step", async () => {
    const c = new WebMockClient();
    const info = await c.newWorkbook(null);
    const doc = info.doc;
    const sheet = info.activeSheet;
    await c.apply(doc, { ops: [{ type: "setPageSetup", sheet, setup: { ...defaultPageSetup(), orientation: "landscape", printRange: { r0: 0, c0: 0, r1: 3, c1: 3 } } }], label: null, selectionBefore: null, selectionAfter: null });
    const before = (await c.cellStyles(doc)).map((s) => s.name);
    const res = await c.loadStylesFromTemplate(doc, "invoices", { cellStyles: true, pageStyles: true, overwrite: true }, "Load Styles");
    expect(res.errors).toEqual([]);
    const after = (await c.cellStyles(doc)).map((s) => s.name);
    expect(after).toEqual(expect.arrayContaining(["Table Header", "Amount", "Remark"]));
    const page = await c.pageSetup(doc, sheet);
    expect(page.orientation).toBe("portrait");
    expect(page.printRange).toEqual({ r0: 0, c0: 0, r1: 3, c1: 3 });
    expect(res.undo.undoLabel).toBe("Load Styles");
    await c.undo(doc);
    expect((await c.cellStyles(doc)).map((s) => s.name)).toEqual(before);
    expect((await c.pageSetup(doc, sheet)).orientation).toBe("landscape");
    await expect(c.loadStylesFromTemplate(doc, "nope", { cellStyles: true, pageStyles: false, overwrite: false }, null)).rejects.toMatchObject({ code: "not_found" });
  });
});
