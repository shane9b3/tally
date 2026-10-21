// @vitest-environment happy-dom
/**
 * Wave-5 app seams (W5-0b): the `t()` hook, the wave-5 registry commands
 * (disabled with the wave-5 reason, except the Function Wizard), the
 * Function Wizard's pure parts and the dialog focus trap.
 */
import { afterEach, describe, expect, it } from "vitest";
import { buildCommands, WAVE5_SOON } from "../src/commands";
import { trapTab } from "../src/components/Dialogs";
import { ALL, insertCallAt, LAST_USED, lastUsedFunctions, noteFunctionUsed, resetLastUsed, wizardCategories, wizardList } from "../src/components/FunctionWizard";
import type { AppController } from "../src/controller";
import { setTranslations, t } from "../src/i18n";
import { BINDINGS } from "../src/keymap";
import { CommandRegistry } from "../src/registry";
import { useUiStore } from "../src/stores/uiStore";

const FUNCS = [
  { name: "SUM", category: "Mathematical", signature: "SUM(number1, [number2], …)", description: "Adds the numbers.", params: [], example: "" },
  { name: "SUMIF", category: "Mathematical", signature: "SUMIF(range, criterion, [sum_range])", description: "Adds the cells that meet a condition.", params: [], example: "" },
  { name: "AVERAGE", category: "Statistical", signature: "AVERAGE(number1, …)", description: "The arithmetic mean.", params: [], example: "" },
  { name: "IF", category: "Logical", signature: "IF(test, then, [else])", description: "Chooses by a condition.", params: [], example: "" },
  { name: "VLOOKUP", category: "Spreadsheet", signature: "VLOOKUP(key, range, column, [sorted])", description: "Vertical lookup in a range.", params: [], example: "" },
];

afterEach(() => {
  setTranslations({});
  resetLastUsed();
  useUiStore.getState().openDialog(null);
});

describe("t()", () => {
  it("returns the fallback until a table is installed, then the table's string with params", () => {
    expect(t("cmd.chart.edit", "Edit chart")).toBe("Edit chart");
    setTranslations({ "cmd.chart.edit": "Diagramm bearbeiten", "outline.level": "Ebene {n}" });
    expect(t("cmd.chart.edit", "Edit chart")).toBe("Diagramm bearbeiten");
    expect(t("outline.level", "Level {n}", { n: 3 })).toBe("Ebene 3");
    expect(t("missing.key", "Keep {x}", { x: "me" })).toBe("Keep me");
  });
});

describe("wave-5 commands", () => {
  const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };
  const cmds = buildCommands({} as AppController, ui);
  const byId = new Map(cmds.map((c) => [c.id, c]));

  it("registers every seam id disabled with the wave-5 reason", () => {
    // W5-5 (charts, pictures, print, split, detective, zoom presets) enabled its ids; the rest wait for W5-6.
    // W5-6 enabled the data tools, styles, options and sheet commands; W5-5 the charts, print and view ones.
    const ids: string[] = [];
    const registry = new CommandRegistry();
    registry.registerAll(cmds);
    for (const id of ids) {
      const c = byId.get(id);
      expect(c, id).toBeDefined();
      expect(c?.enabled?.(), id).toBe(false);
      expect(c?.reason, id).toBe(WAVE5_SOON);
      expect(registry.run(id), id).toBe(false);
    }
    expect(WAVE5_SOON).toBe("coming in wave 5");
  });

  it("the Function Wizard is enabled and opens its dialog (⌘F2)", () => {
    const c = byId.get("insert.function");
    expect(c?.enabled?.() ?? true).toBe(true);
    expect(c?.shortcut).toBe("⌘F2");
    void c?.run();
    expect(useUiStore.getState().dialog).toEqual({ kind: "functionWizard" });
    expect(BINDINGS.find((b) => b.command === "insert.function")?.menu).toBe(true);
  });
});

describe("Function Wizard (pure parts)", () => {
  it("lists categories, filters by category and search, ranks name prefixes first", () => {
    expect(wizardCategories(FUNCS)).toEqual([LAST_USED, ALL, "Logical", "Mathematical", "Spreadsheet", "Statistical"]);
    expect(wizardList(FUNCS, LAST_USED, "").map((f) => f.name)).toEqual(["SUM", "AVERAGE", "IF"]);
    expect(wizardList(FUNCS, "Mathematical", "").map((f) => f.name)).toEqual(["SUM", "SUMIF"]);
    expect(wizardList(FUNCS, ALL, "sumi").map((f) => f.name)).toEqual(["SUMIF"]);
    expect(wizardList(FUNCS, ALL, "lookup").map((f) => f.name)).toEqual(["VLOOKUP"]);
    expect(wizardList(FUNCS, ALL, "mean").map((f) => f.name)).toEqual(["AVERAGE"]);
    noteFunctionUsed("VLOOKUP");
    expect(lastUsedFunctions().slice(0, 2)).toEqual(["VLOOKUP", "SUM"]);
    expect(wizardList(FUNCS, LAST_USED, "")[0]?.name).toBe("VLOOKUP");
  });

  it("insertCallAt writes NAME( at the caret", () => {
    expect(insertCallAt("=1+", 3, "SUM")).toEqual({ text: "=1+SUM(", caret: 7 });
    expect(insertCallAt("=A1", 1, "IF")).toEqual({ text: "=IF(A1", caret: 4 });
  });
});

describe("dialog focus trap", () => {
  it("Tab wraps from the last control to the first and Shift+Tab the other way", () => {
    const form = document.createElement("form");
    form.innerHTML = '<input id="a"><select id="b"></select><button id="c" type="submit">OK</button>';
    document.body.appendChild(form);
    const a = form.querySelector("#a") as HTMLElement;
    const c = form.querySelector("#c") as HTMLElement;
    let prevented = 0;
    const ev = (shift: boolean) => ({ key: "Tab", shiftKey: shift, preventDefault: () => void prevented++ });
    c.focus();
    expect(trapTab(form, ev(false), c)).toBe(true);
    expect(document.activeElement).toBe(a);
    expect(trapTab(form, ev(true), a)).toBe(true);
    expect(document.activeElement).toBe(c);
    // In the middle the browser's own order applies.
    expect(trapTab(form, ev(false), a)).toBe(false);
    expect(trapTab(form, { key: "Enter", shiftKey: false, preventDefault: () => undefined }, a)).toBe(false);
    expect(prevented).toBe(2);
    form.remove();
  });
});
