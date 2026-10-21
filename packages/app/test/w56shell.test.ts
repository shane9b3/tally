/**
 * W5-6: the shell commands of `TauriClient` (prefs, autosave, templates, UI
 * language) are registered in `lib.rs` and defined in their Rust modules,
 * and the browser mock emulates the data-tool ops the dialogs issue.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { COMMANDS, TauriClient, W56_SHELL_COMMANDS, type InvokeArgs, type TauriEvent } from "@tally/bridge/tauri";
import { describe, expect, it } from "vitest";
import { WebMockClient } from "../src/dev/webMock";

const SRC = new URL("../../../apps/desktop/src-tauri/src/", import.meta.url);
const LIB = readFileSync(fileURLToPath(new URL("lib.rs", SRC)), "utf8");

describe("W5-6 shell commands", () => {
  it("are registered in lib.rs and defined in their modules", () => {
    const modules: Record<string, string> = { get_prefs: "prefs.rs", set_prefs: "prefs.rs", autosave_snapshot: "autosave.rs", list_recovery: "autosave.rs", recover_snapshot: "autosave.rs", discard_recovery: "autosave.rs", save_as_template: "templates.rs", list_user_templates: "templates.rs", new_from_user_template: "templates.rs", set_ui_language: "l10n.rs", clear_recent: "files.rs" };
    for (const m of W56_SHELL_COMMANDS) {
      const name = COMMANDS[m];
      expect(LIB, name).toMatch(new RegExp(`::${name},`));
      const file = readFileSync(fileURLToPath(new URL(modules[name] ?? "", SRC)), "utf8");
      expect(file, name).toMatch(new RegExp(`#\\[tauri::command\\]\\s*pub async fn ${name}\\(`));
    }
  });

  it("invoke with the documented argument names", async () => {
    const calls: { cmd: string; args: InvokeArgs | undefined }[] = [];
    const client = new TauriClient({
      invoke: <T,>(cmd: string, args?: InvokeArgs): Promise<T> => {
        calls.push({ cmd, args });
        return Promise.resolve((cmd === "get_prefs" ? "{}" : cmd === "list_recovery" || cmd === "list_user_templates" ? [] : null) as T);
      },
      listen: <T,>(_e: string, _h: (e: TauriEvent<T>) => void) => Promise.resolve(() => undefined),
    });
    expect(await client.getPrefs()).toBe("{}");
    await client.setPrefs('{"autoInput":false}');
    await client.autosave(3);
    await client.listRecovery();
    await client.recoverSnapshot("/x/doc-3-Book.ods");
    await client.discardRecovery("/x/doc-3-Book.ods");
    await client.saveAsTemplate(3, "Budget");
    await client.listUserTemplates();
    await client.newFromUserTemplate("user:Budget");
    await client.setUiLanguage("de");
    await client.clearRecent();
    expect(calls).toEqual([
      { cmd: "get_prefs", args: undefined },
      { cmd: "set_prefs", args: { json: '{"autoInput":false}' } },
      { cmd: "autosave_snapshot", args: { doc: 3 } },
      { cmd: "list_recovery", args: undefined },
      { cmd: "recover_snapshot", args: { path: "/x/doc-3-Book.ods" } },
      { cmd: "discard_recovery", args: { path: "/x/doc-3-Book.ods" } },
      { cmd: "save_as_template", args: { doc: 3, name: "Budget" } },
      { cmd: "list_user_templates", args: undefined },
      { cmd: "new_from_user_template", args: { id: "user:Budget" } },
      { cmd: "set_ui_language", args: { lang: "de" } },
      { cmd: "clear_recent", args: undefined },
    ]);
  });
});

async function setup() {
  const c = new WebMockClient();
  const info = await c.newWorkbook(null);
  const doc = info.doc;
  const sheet = info.activeSheet;
  const tx = (ops: Parameters<WebMockClient["apply"]>[1]["ops"]) => c.apply(doc, { ops, label: null, selectionBefore: null, selectionAfter: null });
  await tx([{ type: "setRangeInputs", sheet, r: 0, c: 0, rows: [["Name", "Qty"], ["apple", "3"], ["Banana", "7"], ["apple", "3"], ["cherry", "12"]] }]);
  const hiddenRows = async () => (await c.sheetLayout(doc, sheet)).rowRuns.filter((r) => r.hidden).flatMap((r) => Array.from({ length: r.end - r.start + 1 }, (_, i) => r.start + i));
  const col = async (cc: number, r1 = 8) => (await c.cells(doc, sheet, [{ r0: 0, c0: cc, r1, c1: cc }]))[0]!.cells.map((v) => `${v.r}:${v.text}`);
  return { c, doc, sheet, tx, hiddenRows, col };
}

describe("mock data tools", () => {
  it("standard filter hides non-matching rows; reset shows them", async () => {
    const { tx, sheet, hiddenRows } = await setup();
    const res = await tx([{ type: "standardFilter", sheet, range: { r0: 0, c0: 0, r1: 4, c1: 1 }, conditions: [{ connector: "and", col: 1, op: "gt", value: "5" }], options: { caseSensitive: false, regex: false, hasHeader: true, noDuplicates: false, copyTo: null, keepCriteria: true } }]);
    expect(res.errors).toEqual([]);
    expect(await hiddenRows()).toEqual([1, 3]);
    await tx([{ type: "resetFilter", sheet }]);
    expect(await hiddenRows()).toEqual([]);
    await tx([{ type: "standardFilter", sheet, range: { r0: 0, c0: 0, r1: 4, c1: 1 }, conditions: [{ connector: "and", col: 0, op: "beginsWith", value: "a" }, { connector: "or", col: 0, op: "contains", value: "err" }], options: { caseSensitive: false, regex: false, hasHeader: true, noDuplicates: true, copyTo: null, keepCriteria: true } }]);
    expect(await hiddenRows()).toEqual([2, 3]);
  });

  it("group collapses rows through the outline model and undo restores them", async () => {
    const { c, doc, tx, sheet, hiddenRows } = await setup();
    await tx([{ type: "group", sheet, axis: "rows", start: 1, end: 3 }]);
    await tx([{ type: "setOutlineCollapsed", sheet, axis: "rows", start: 1, end: 3, collapsed: true }]);
    expect(await hiddenRows()).toEqual([1, 2, 3]);
    await tx([{ type: "showOutlineLevel", sheet, axis: "rows", level: 2 }]);
    expect(await hiddenRows()).toEqual([]);
    await c.undo(doc);
    expect(await hiddenRows()).toEqual([1, 2, 3]);
  });

  it("text to columns, remove duplicates, delete contents and fill series rewrite cells", async () => {
    const { tx, sheet, col } = await setup();
    await tx([{ type: "setCellInput", sheet, r: 6, c: 0, input: "a;b;c" }]);
    await tx([{ type: "textToColumns", sheet, range: { r0: 6, c0: 0, r1: 6, c1: 0 }, options: { delimiters: ";", mergeDelimiters: false, quote: '"', trim: true, detectNumbers: true, fixedWidths: null, columnTypes: [] } }]);
    expect((await col(1)).filter((s) => s.startsWith("6:"))).toEqual(["6:b"]);
    await tx([{ type: "removeDuplicates", sheet, range: { r0: 0, c0: 0, r1: 4, c1: 1 }, cols: [], hasHeader: true }]);
    expect(await col(0, 4)).toEqual(["0:Name", "1:apple", "2:Banana", "3:cherry"]);
    await tx([{ type: "deleteContents", sheet, ranges: [{ r0: 1, c0: 0, r1: 3, c1: 1 }], what: { text: false, numbers: true, dates: false, formulas: false, notes: false, formats: false, objects: false } }]);
    expect(await col(1, 4)).toEqual(["0:Qty"]);
    expect(await col(0, 4)).toEqual(["0:Name", "1:apple", "2:Banana", "3:cherry"]);
    await tx([{ type: "fillSeries", sheet, range: { r0: 1, c0: 1, r1: 3, c1: 1 }, direction: "down", kind: "linear", unit: null, start: "10", end: null, increment: "5" }]);
    expect(await col(1, 4)).toEqual(["0:Qty", "1:10", "2:15", "3:20"]);
  });

  it("scenarios store and restore values, cell styles and sheet state are reported", async () => {
    const { c, doc, tx, sheet, col } = await setup();
    const res = await tx([{ type: "createScenario", sheet, scenario: { id: 0, name: "High", comment: "", ranges: [{ r0: 1, c0: 1, r1: 1, c1: 1 }], borderColor: null, displayBorder: true, copyBack: true, copyStyles: false, preventChanges: false, active: false } }]);
    expect(res.errors).toEqual([]);
    await tx([{ type: "setCellInput", sheet, r: 1, c: 1, input: "99" }]);
    await tx([{ type: "createScenario", sheet, scenario: { id: 0, name: "Low", comment: "", ranges: [{ r0: 1, c0: 1, r1: 1, c1: 1 }], borderColor: null, displayBorder: true, copyBack: false, copyStyles: false, preventChanges: false, active: false } }]);
    const list = await c.scenarios(doc, sheet);
    expect(list.map((s) => [s.name, s.active])).toEqual([["High", false], ["Low", true]]);
    await tx([{ type: "showScenario", sheet, id: list[0]!.id }]);
    expect((await col(1, 1))[1]).toBe("1:3");
    const styles = await c.cellStyles(doc);
    expect(styles.map((s) => s.name)).toEqual(["Default", "Heading", "Heading 1", "Result", "Result 2"]);
    await tx([{ type: "defineCellStyle", style: { name: "Mine", parent: "Default", builtin: false, style: { ...styles[0]!.style, bold: true, fill: 0xffee00ff } } }]);
    await tx([{ type: "applyCellStyle", sheet, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 1 }], name: "Mine" }]);
    const [b] = await c.cells(doc, sheet, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]);
    const [st] = await c.styles(doc, [b!.cells[0]!.style]);
    expect(st?.bold).toBe(true);
    await tx([{ type: "addSheet", name: "Two", index: null }]);
    const hide = await tx([{ type: "setSheetHidden", sheet, hidden: true }]);
    expect(hide.info?.sheets.find((s) => s.id === sheet)?.hidden).toBe(true);
    const color = await tx([{ type: "setTabColor", sheet, color: 0xff0000ff }]);
    expect(color.info?.sheets.find((s) => s.id === sheet)?.tabColor).toBe("#ff0000");
    await tx([{ type: "setCalcSettings", settings: { iterations: { steps: 50, minChange: 0.01 }, dateEpoch: "d1904", caseSensitive: true, precisionAsShown: false, wholeCellMatch: true, criteriaMode: "regex", textToNumber: "treatAsZero" } }]);
    expect((await c.calcSettings(doc)).dateEpoch).toBe("d1904");
    await tx([{ type: "defineDatabaseRange", name: "Fruit", sheet, range: { r0: 0, c0: 0, r1: 4, c1: 1 }, hasHeader: true }]);
    expect((await c.databaseRanges(doc)).map((d) => d.name)).toEqual(["Fruit"]);
    expect((await c.navigator(doc)).databaseRanges.map((d) => d.title)).toEqual(["Fruit"]);
    expect((await c.autocomplete(doc, sheet, 5, 0, "ch")).completion).toBe("cherry");
    expect((await c.autocomplete(doc, sheet, 5, 0, "")).items).toEqual(["apple", "Banana", "cherry", "Name"]);
  });
});
