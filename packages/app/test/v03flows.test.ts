/**
 * v0.3 flows through the controller and the browser mock: validation Stop
 * / Warning / Information reporting, protection refusals, conditional
 * format evaluation, undo of side-model changes, Colour by value, goal
 * seek, the locale and typing auto-fit. Also checks that every TauriClient
 * command is registered in the Tauri shell.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { CellView, Op, ValidationDto } from "@tally/bridge";
import { COMMANDS } from "@tally/bridge/tauri";
import { beforeEach, describe, expect, it } from "vitest";
import { AppController } from "../src/controller";
import { evaluateFormat, mixColor, operand } from "../src/dev/mockCf";
import { applyV3Op, emptyV3, passes, protectionRefusal, subtract } from "../src/dev/mockV3";
import { WebMockClient } from "../src/dev/webMock";
import { opCell, PROTECTED_MESSAGE } from "../src/features";
import { useUiStore } from "../src/stores/uiStore";
import { useV3Store } from "../src/stores/v3Store";
import { presetRule } from "../src/v03";

const tick = () => new Promise((r) => setTimeout(r, 0));

async function controller(template: string | null = "invoices") {
  const client = new WebMockClient();
  const ctl = new AppController(client, client, { nativeMenu: false });
  await ctl.start(template);
  await tick();
  return { client, ctl, sheet: ctl.sheet, doc: ctl.info!.doc };
}

function listValidation(style: "stop" | "warning" | "info"): ValidationDto {
  return {
    id: 0,
    ranges: [{ r0: 1, c0: 6, r1: 14, c1: 6 }],
    kind: { type: "list", source: { type: "values", items: ["Paid", "Due", "Overdue"] } },
    allowBlank: true,
    showDropdown: true,
    inputHelp: { title: "Status", text: "Pick one" },
    error: { style, title: "Status", text: "Choose Paid, Due or Overdue." },
  };
}

beforeEach(() => {
  useUiStore.getState().reset();
  useV3Store.getState().reset();
});

describe("validation flows", () => {
  it("Stop rejects the value and opens the alert with Retry", async () => {
    const { client, ctl, sheet, doc } = await controller();
    await ctl.v3.applyValidation(listValidation("stop"), false);
    const res = await ctl.apply("Typing", [{ type: "setCellInput", sheet, r: 2, c: 6, input: "Maybe" }]);
    expect(res?.errors).toEqual([{ opIndex: 0, code: "validation", message: "Choose Paid, Due or Overdue." }]);
    await tick();
    await tick();
    const d = useUiStore.getState().dialog;
    expect(d).toMatchObject({ kind: "validationAlert", title: "Status", message: "Choose Paid, Due or Overdue.", retry: "Maybe", cell: { r: 2, c: 6 } });
    // The value is unchanged, and no generic error toast was shown.
    const [b] = await client.cells(doc, sheet, [{ r0: 2, c0: 6, r1: 2, c1: 6 }]);
    expect(b?.cells[0]?.text).toBe("Due");
    expect(useUiStore.getState().toasts.filter((t) => t.kind === "error")).toEqual([]);
    // Allowed values pass, the dropdown flag is set.
    const ok = await ctl.apply("Typing", [{ type: "setCellInput", sheet, r: 2, c: 6, input: "due" }]);
    expect(ok?.errors).toEqual([]);
    expect((b?.cells[0]?.flags ?? 0) & 16).toBe(16);
  });

  it("Warning keeps the value and offers Keep / Undo", async () => {
    const { client, ctl, sheet, doc } = await controller();
    await ctl.v3.applyValidation(listValidation("warning"), false);
    const res = await ctl.apply("Typing", [{ type: "setCellInput", sheet, r: 3, c: 6, input: "Maybe" }]);
    expect(res?.errors).toEqual([]);
    expect(res?.warnings).toHaveLength(1);
    await tick();
    await tick();
    const toast = useUiStore.getState().toasts.at(-1);
    expect(toast).toMatchObject({ kind: "warning", message: "Status: Choose Paid, Due or Overdue." });
    expect(toast?.actions?.map((a) => a.label)).toEqual(["Keep", "Undo"]);
    toast?.actions?.[1]?.run();
    await tick();
    const [b] = await client.cells(doc, sheet, [{ r0: 3, c0: 6, r1: 3, c1: 6 }]);
    expect(b?.cells[0]?.text).toBe("Paid");
  });

  it("Information keeps the value with an info toast", async () => {
    const { ctl, sheet } = await controller();
    await ctl.v3.applyValidation(listValidation("info"), false);
    await ctl.apply("Typing", [{ type: "setCellInput", sheet, r: 3, c: 6, input: "Maybe" }]);
    await tick();
    await tick();
    expect(useUiStore.getState().toasts.at(-1)).toMatchObject({ kind: "info", message: "Status: Choose Paid, Due or Overdue." });
  });

  it("input help shows for a validated cell", async () => {
    const { ctl } = await controller();
    await ctl.v3.applyValidation(listValidation("stop"), false);
    await ctl.v3.refreshSheet();
    expect(useV3Store.getState().hasValidations).toBe(true);
  });

  it("the mock's validation checks", () => {
    const v = listValidation("stop");
    expect(passes(v, "paid", null)).toBe(true);
    expect(passes(v, "", null)).toBe(true);
    expect(passes({ ...v, allowBlank: false }, "", null)).toBe(false);
    const whole: ValidationDto = { ...v, kind: { type: "whole", op: "between", formulas: ["1", "10"] } };
    expect(passes(whole, "5", null)).toBe(true);
    expect(passes(whole, "5.5", null)).toBe(false);
    expect(passes(whole, "11", null)).toBe(false);
    expect(passes({ ...v, kind: { type: "textLength", op: "le", formulas: ["3"] } }, "abcd", null)).toBe(false);
  });
});

describe("protection", () => {
  it("refuses edits of locked cells with the lock feedback", async () => {
    const { ctl, sheet } = await controller();
    await ctl.v3.protectSheet("pw", { selectLocked: true, selectUnlocked: true, insertRows: false, insertCols: false, deleteRows: false, deleteCols: false, formatCells: false, sort: false, autoFilter: false });
    const res = await ctl.apply("Typing", [{ type: "setCellInput", sheet, r: 1, c: 1, input: "x" }]);
    expect(res?.errors[0]?.code).toBe("protected");
    expect(useUiStore.getState().toasts.at(-1)).toMatchObject({ kind: "error", message: PROTECTED_MESSAGE });
    expect(await ctl.v3.unprotectSheet("wrong")).toBe(false);
    expect(await ctl.v3.unprotectSheet("pw")).toBe(true);
    const ok = await ctl.apply("Typing", [{ type: "setCellInput", sheet, r: 1, c: 1, input: "x" }]);
    expect(ok?.errors).toEqual([]);
  });

  it("the mock's protection rules", () => {
    const x = emptyV3();
    expect(applyV3Op(x, { type: "setCellProtection", sheet: 1, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], locked: false, hiddenFormula: null })).toMatchObject({ kind: "handled" });
    applyV3Op(x, { type: "protectSheet", sheet: 1, password: null, options: null });
    expect(protectionRefusal(x, { type: "setCellInput", sheet: 1, r: 0, c: 0, input: "ok" })).toBeNull();
    expect(protectionRefusal(x, { type: "setCellInput", sheet: 1, r: 0, c: 1, input: "no" })).toMatch(/protected/);
    expect(protectionRefusal(x, { type: "insertRows", sheet: 1, at: 0, count: 1 })).toMatch(/protected/);
    expect(protectionRefusal(x, { type: "setCellInput", sheet: 2, r: 0, c: 1, input: "other sheet" })).toBeNull();
    applyV3Op(x, { type: "protectDocument", password: null });
    expect(protectionRefusal(x, { type: "addSheet", name: null, index: null })).toMatch(/structure/);
  });

  it("new validations replace older ones where they overlap", () => {
    const x = emptyV3();
    const v = listValidation("stop");
    applyV3Op(x, { type: "addValidation", sheet: 1, validation: v });
    applyV3Op(x, { type: "addValidation", sheet: 1, validation: { ...v, ranges: [{ r0: 5, c0: 6, r1: 6, c1: 6 }] } });
    expect(x.validations[1]![0]!.ranges).toEqual([
      { r0: 1, c0: 6, r1: 4, c1: 6 },
      { r0: 7, c0: 6, r1: 14, c1: 6 },
    ]);
    expect(subtract({ r0: 0, c0: 0, r1: 2, c1: 2 }, { r0: 1, c0: 1, r1: 1, c1: 1 })).toHaveLength(4);
  });
});

describe("conditional formatting", () => {
  it("the mock evaluates rules, scales, bars and icons", () => {
    const v = (r: number, n: number): CellView => ({ r, c: 0, text: String(n), kind: "number", num: n, style: 0, color: null, flags: 0 });
    const cells = [v(0, 0), v(1, 50), v(2, 100)];
    const range = [{ r0: 0, c0: 0, r1: 2, c1: 0 }];
    const gt = evaluateFormat({ id: 1, ranges: range, rules: [presetRule("gt", ["60"])] }, cells);
    expect([...gt.keys()]).toEqual(["2,0"]);
    const scale = evaluateFormat({ id: 1, ranges: range, rules: [presetRule("colorScale", [])] }, cells);
    expect(scale.get("0,0")?.patch?.fill).toBe(0xf8696bff);
    expect(scale.get("2,0")?.patch?.fill).toBe(0x63be7bff);
    const bar = evaluateFormat({ id: 1, ranges: range, rules: [presetRule("dataBar", [])] }, cells);
    expect(bar.get("1,0")?.bar?.fraction).toBeCloseTo(0.5);
    const icons = evaluateFormat({ id: 1, ranges: range, rules: [presetRule("iconSet", [])] }, cells);
    expect([0, 1, 2].map((r) => icons.get(`${r},0`)?.icon?.index)).toEqual([0, 1, 2]);
    const top = evaluateFormat({ id: 1, ranges: range, rules: [{ ...presetRule("top10", []), kind: { type: "top", rank: 1, percent: false, bottom: false } }] }, cells);
    expect([...top.keys()]).toEqual(["2,0"]);
    expect(mixColor(0x000000ff, 0xffffffff, 0.5)).toBe(0x808080ff);
    expect(operand('"Paid"')).toBe("Paid");
    expect(operand("$B$1")).toBeNull();
  });

  it("cells get a synthesized style that resolves, and undo removes the rule", async () => {
    const { client, ctl, sheet, doc } = await controller();
    await ctl.v3.addConditionalFormat([{ r0: 1, c0: 5, r1: 14, c1: 5 }], [presetRule("gt", ["60"])]);
    const [b] = await client.cells(doc, sheet, [{ r0: 2, c0: 5, r1: 2, c1: 5 }]);
    const id = b?.cells[0]?.cfStyle ?? 0;
    expect(id).toBeGreaterThanOrEqual(0x40000000);
    const [st] = await client.styles(doc, [id]);
    expect(st?.fill).toBe(0xfee2e2ff);
    expect(await ctl.v3.conditionalFormats()).toHaveLength(1);
    await ctl.undo();
    expect(await ctl.v3.conditionalFormats()).toHaveLength(0);
    await ctl.redo();
    expect(await ctl.v3.conditionalFormats()).toHaveLength(1);
  });

  it("Colour by value replaces its own earlier format", async () => {
    const { client, ctl, sheet, doc } = await controller();
    const body = { r0: 1, c0: 6, r1: 14, c1: 6 };
    await ctl.colorByValueOn(body);
    await ctl.colorByValueOn(body);
    const cfs = await ctl.v3.conditionalFormats();
    expect(cfs).toHaveLength(1);
    expect(client.recorded.at(-1)?.type).toBe("updateConditionalFormat");
    const [b] = await client.cells(doc, sheet, [{ r0: 1, c0: 6, r1: 1, c1: 6 }]);
    const [st] = await client.styles(doc, [b!.cells[0]!.cfStyle!]);
    expect(st?.display).toEqual({ type: "chip", bg: 0xdcfce7ff, fg: 0x166534ff });
  });
});

describe("other v0.3 flows", () => {
  it("goal seek (mock: proportional) and apply", async () => {
    const { client, ctl } = await controller();
    const res = await ctl.v3.goalSeek({ r: 1, c: 5 }, 100, { r: 1, c: 3 });
    expect(res.found).toBe(true);
    expect(res.value).toBeCloseTo(100 / 4.5, 6);
    await ctl.v3.applyGoalSeek({ r: 1, c: 3 }, res.value);
    expect(client.recorded.at(-1)).toMatchObject({ type: "setCellInput", r: 1, c: 3 });
  });

  it("locale switch calls setLocale and remembers it", async () => {
    const { client, ctl } = await controller();
    await ctl.v3.setLocale("fr-FR");
    expect(client.recordedCalls.at(-1)).toEqual({ type: "setLocale", locale: "fr-FR" });
    expect(useV3Store.getState().locale).toBe("fr-FR");
  });

  it("typing into a wrapped cell also auto-fits the row", async () => {
    const { ctl, sheet } = await controller();
    await ctl.apply("Wrap", [{ type: "applyStyle", sheet, ranges: [{ r0: 1, c0: 2, r1: 1, c1: 2 }], patch: { bold: null, italic: null, underline: null, strike: null, fontFamily: null, fontSizePt: null, color: null, fill: null, hAlign: null, vAlign: null, wrap: true, numberFormat: null } }]);
    // Let the grid source cache the cell and its style.
    const src = (await import("../src/stores/docStore")).useDocStore.getState().source!;
    src.ensureRanges([{ r0: 0, c0: 0, r1: 31, c1: 63 }]);
    for (let i = 0; i < 5; i++) await tick();
    src.getStyle(src.getCell(1, 2)?.style ?? 0);
    for (let i = 0; i < 5; i++) await tick();
    expect(ctl.typingOps(sheet, { r: 1, c: 2 }, "long text")).toEqual([
      // AutoCorrect (W5-6) capitalises the sentence start on commit, as AOO does.
      { type: "setCellInput", sheet, r: 1, c: 2, input: "Long text" },
      { type: "autoFitRows", sheet, start: 1, end: 1 },
    ]);
    expect(ctl.typingOps(sheet, { r: 1, c: 3 }, "line 1\nline 2")).toHaveLength(2);
    expect(ctl.typingOps(sheet, { r: 1, c: 3 }, "12")).toHaveLength(1);
  });

  it("finds the cell an op writes", () => {
    const ops: Op[] = [
      { type: "setCellInput", sheet: 1, r: 3, c: 4, input: "x" },
      { type: "paste", sheet: 1, target: { r0: 5, c0: 6, r1: 7, c1: 8 }, payload: { type: "tsv", text: "a" } },
      { type: "setFrozen", sheet: 1, rows: 1, cols: 0 },
    ];
    expect(ops.map(opCell)).toEqual([{ r: 3, c: 4 }, { r: 5, c: 6 }, null]);
  });
});

describe("Tauri shell", () => {
  it("registers every TauriClient command", () => {
    const lib = readFileSync(fileURLToPath(new URL("../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url)), "utf8");
    const registered = new Set([...lib.matchAll(/(?:commands|commands_v4|dialogs|prefs|autosave|templates|l10n|files|closing|windows)::([a-z_]+),/g)].map((m) => m[1]));
    expect(Object.values(COMMANDS).filter((c) => !registered.has(c))).toEqual([]);
  });
});
