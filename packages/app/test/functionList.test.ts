// @vitest-environment happy-dom
/** Function List panel: categories, Last Used, insert text, separators, localized names, type-ahead. */
import { afterEach, describe, expect, it } from "vitest";
import type { FunctionInfo } from "@tally/bridge";
import { ALL, functionsOf, insertFunctionText, LAST_USED, listCategories, signatureWithSeparator, typeAheadIndex } from "../src/functionList/model";
import { localizedName, localizeFunctions } from "../src/i18n/formulaNames";
import { lastUsedFunctions, noteFunctionUsed, resetLastUsed } from "../src/stores/lastUsedStore";

const f = (name: string, category: string, signature = `${name}(a, b)`): FunctionInfo => ({ name, category, signature, description: "", params: [], example: "" });
const FUNCS = [f("SUM", "Mathematical", "SUM(number1, [number2], …)"), f("AVERAGE", "Statistical"), f("TEXT", "Text"), f("DB", "Financial"), f("DCOUNT", "Database"), f("IF", "Logical"), f("MIN", "Statistical"), f("MAX", "Statistical"), f("FOO", "Zebra")];

afterEach(resetLastUsed);

describe("categories", () => {
  it("lists Last Used, All, then AOO's order, extras last", () => {
    expect(listCategories(FUNCS)).toEqual([LAST_USED, ALL, "Database", "Financial", "Logical", "Mathematical", "Statistical", "Text", "Zebra"]);
  });
  it("filters a category and All", () => {
    expect(functionsOf(FUNCS, "Statistical", []).map((x) => x.name)).toEqual(["AVERAGE", "MIN", "MAX"]);
    expect(functionsOf(FUNCS, ALL, [])).toHaveLength(FUNCS.length);
  });
});

describe("Last Used", () => {
  it("starts with AOO's defaults, moves inserts to the front, caps at 10", () => {
    expect(functionsOf(FUNCS, LAST_USED, lastUsedFunctions()).map((x) => x.name)).toEqual(["SUM", "AVERAGE", "MIN", "MAX", "IF"]);
    noteFunctionUsed("TEXT");
    noteFunctionUsed("SUM");
    expect(lastUsedFunctions().slice(0, 3)).toEqual(["SUM", "TEXT", "AVERAGE"]);
    for (let i = 0; i < 15; i++) noteFunctionUsed(`F${i}`);
    expect(lastUsedFunctions()).toHaveLength(10);
    expect(lastUsedFunctions()[0]).toBe("F14");
  });
  it("shows localized names for canonical recents", () => {
    const de = localizeFunctions("de", FUNCS);
    const names = functionsOf(de, LAST_USED, ["SUM", "IF"], (n) => localizedName("de", n)).map((x) => x.name);
    expect(names).toEqual(["SUMME", "WENN"]);
  });
});

describe("insert text", () => {
  it("writes NAME() at the caret and leaves the caret inside", () => {
    expect(insertFunctionText("", 0, "SUM")).toEqual({ text: "=SUM()", caret: 5 });
    expect(insertFunctionText("=1+", 3, "SUM")).toEqual({ text: "=1+SUM()", caret: 7 });
    expect(insertFunctionText("=A1*2", 3, "IF")).toEqual({ text: "=A1IF()*2", caret: 6 });
  });
  it("uses the localized name", () => {
    const de = localizeFunctions("de", FUNCS);
    const sum = localizedName("de", "SUM");
    expect(de.some((x) => x.name === sum)).toBe(true);
    expect(insertFunctionText("=", 1, sum)).toEqual({ text: "=SUMME()", caret: 7 });
  });
});

describe("syntax separators", () => {
  it("follows the UI argument separator", () => {
    expect(signatureWithSeparator("SUM(number1, [number2], …)", ";")).toBe("SUM(number1; [number2]; …)");
    expect(signatureWithSeparator("SUM(number1; number2)", ",")).toBe("SUM(number1, number2)");
  });
  it("localizes the name in the signature", () => {
    const de = localizeFunctions("de", FUNCS);
    expect(signatureWithSeparator(de.find((x) => x.name === "SUMME")!.signature, ";")).toBe("SUMME(number1; [number2]; …)");
  });
});

describe("type-ahead", () => {
  const names = ["AVERAGE", "MAX", "MIN", "SUM"];
  it("finds the first prefix match from the position, wrapping", () => {
    expect(typeAheadIndex(names, "m", 0)).toBe(1);
    expect(typeAheadIndex(names, "mi", 1)).toBe(2);
    expect(typeAheadIndex(names, "a", 3)).toBe(0);
    expect(typeAheadIndex(names, "x", 0)).toBe(-1);
  });
});
