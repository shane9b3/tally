/**
 * W5-6C: AutoCorrect rules and table, the AutoInput matcher, the Function
 * Wizard argument model and the display-level function-name localisation.
 */
import { describe, expect, it } from "vitest";
import { applyAutoCorrect, applyReplacementTable, capitalizeSentences, fixOrdinals, fixTwoInitialCapitals, isPlainText, replaceDashes, smartQuotes } from "../src/autocorrect/autocorrect";
import { REPLACEMENTS } from "../src/autocorrect/replacements";
import { completionFor, ghostSuffix, matchingItems, wantsCompletion } from "../src/autocorrect/autoinput";
import { DEFAULT_AUTOCORRECT } from "../src/options/prefs";
import { argumentError, argumentSlots, buildCall, parseCall, parseSignature, wizardParams } from "../src/wizard/wizardArgs";
import { canonicalName, formulaLangOf, localizeFunctions, localizedName, mapFunctionNames, toCanonicalFormula, toDisplayFormula } from "../src/i18n/formulaNames";

describe("AutoCorrect", () => {
  it("has a replacement table of about 200 AOO entries without duplicates", () => {
    expect(REPLACEMENTS.length).toBeGreaterThanOrEqual(200);
    const keys = REPLACEMENTS.map(([k]) => k.toLowerCase());
    expect(new Set(keys).size).toBe(keys.length);
  });
  it("applies the rules the acceptance test names", () => {
    expect(applyAutoCorrect("TEst")).toBe("Test");
    expect(applyAutoCorrect("1st")).toBe("1st");
    expect(applyAutoCorrect("teh 2th time")).toBe("The 2nd time");
    expect(applyAutoCorrect("hello. world")).toBe("Hello. World");
    expect(applyAutoCorrect("a - b")).toBe("A – b");
    expect(fixTwoInitialCapitals("NASA and TWo")).toBe("NASA and Two");
    expect(capitalizeSentences("e.g. this one. and that")).toBe("E.g. this one. And that");
    expect(replaceDashes("x--y")).toBe("x—y");
    expect(fixOrdinals("11st 22th 3th 101st")).toBe("11th 22nd 3rd 101st");
    expect(smartQuotes(`"quoted" it's 'x'`)).toBe("“quoted” it’s ‘x’");
    expect(applyReplacementTable("Teh (c) 1/2")).toBe("The © ½");
    expect(applyReplacementTable("I like TEH")).toBe("I like THE");
  });
  it("leaves formulas, numbers, dates, URLs and quoted text alone", () => {
    for (const s of ["=SUM(A1)", "{=A1:A2}", "'teh", "12.5", "-3,000", "2024-01-05", "12:30", "https://teh.example", "me@teh.example", "TRUE"]) {
      expect(isPlainText(s), s).toBe(false);
      expect(applyAutoCorrect(s), s).toBe(s);
    }
    expect(applyAutoCorrect("teh", { ...DEFAULT_AUTOCORRECT, replacementTable: false, capitalizeSentence: false })).toBe("teh");
  });
});

describe("AutoInput", () => {
  it("offers the first alphabetical match keeping the typed prefix", () => {
    const items = ["Banana", "apple", "Apricot", "apple pie"];
    expect(completionFor("ap", items)).toBe("apple");
    expect(completionFor("Ap", items)).toBe("Apple");
    expect(completionFor("apple", items)).toBe("apple pie");
    expect(completionFor("z", items)).toBeNull();
    expect(completionFor("", items)).toBeNull();
    expect(matchingItems("", items)).toEqual(["apple", "apple pie", "Apricot", "Banana"]);
    expect(matchingItems("b", items)).toEqual(["Banana"]);
    expect(ghostSuffix("Ap", "Apple")).toBe("ple");
    expect(ghostSuffix("Apple", "Apple")).toBe("");
    expect(wantsCompletion("=SUM")).toBe(false);
    expect(wantsCompletion("12")).toBe(false);
    expect(wantsCompletion("Ap")).toBe(true);
  });
});

describe("Function Wizard arguments", () => {
  it("parses signatures with optional and repeating parameters", () => {
    expect(parseSignature("SUMIF(range, criterion, [sum_range])")).toEqual([
      { name: "range", optional: false, variadic: false },
      { name: "criterion", optional: false, variadic: false },
      { name: "sum_range", optional: true, variadic: false },
    ]);
    expect(parseSignature("SUM(number1, [number2], …)")).toEqual([
      { name: "number1", optional: false, variadic: false },
      { name: "number2", optional: true, variadic: true },
    ]);
    expect(parseSignature("PI()")).toEqual([]);
    expect(parseSignature("IF(test; then; [else])").map((p) => p.name)).toEqual(["test", "then", "else"]);
    const params = parseSignature("SUM(number1, [number2], …)");
    expect(argumentSlots(params, []).map((p) => p.name)).toEqual(["number1", "number2"]);
    expect(argumentSlots(params, ["A1", "B1"]).map((p) => p.name)).toEqual(["number1", "number2", "number3"]);
    expect(argumentError(argumentSlots(params, []), [])).toBe("number1 is required.");
    expect(argumentError(parseSignature("SUMIF(range, criterion, [sum_range])"), ["A1:A5", ">3"])).toBeNull();
    expect(buildCall("SUMIF", ["A1:A5", '">3"', ""])).toBe('SUMIF(A1:A5;">3")');
    expect(buildCall("SUM", ["A1", "", "B1"], ",")).toBe("SUM(A1,,B1)");
    expect(parseCall('=SUMIF(A1:A5;">3";B1:B5)')).toEqual({ name: "SUMIF", args: ["A1:A5", '">3"', "B1:B5"] });
    expect(parseCall("=IF(SUM(A1;A2)>3;\"a;b\";0)")).toEqual({ name: "IF", args: ["SUM(A1;A2)>3", '"a;b"', "0"] });
    expect(parseCall("hello")).toBeNull();
    const meta = { name: "X", category: "c", signature: "X(a)", description: "", params: [{ name: "a", optional: false, description: "first" }] } as unknown as Parameters<typeof wizardParams>[0];
    expect(wizardParams(meta)).toEqual([{ name: "a", optional: false, variadic: false, description: "first" }]);
  });
});

describe("display-level function names", () => {
  it("maps canonical names to the document locale and back", () => {
    expect(formulaLangOf("de-DE")).toBe("de");
    expect(formulaLangOf("en-GB")).toBeNull();
    expect(localizedName("de", "SUM")).toBe("SUMME");
    expect(localizedName("fr", "MAX")).toBe("MAX");
    expect(canonicalName("es", "suma")).toBe("SUM");
    expect(canonicalName("de", "SUMA")).toBeNull();
    expect(toDisplayFormula("de", '=SUM(A1:A3)+IF(A1>1;"SUM(";0)')).toBe('=SUMME(A1:A3)+WENN(A1>1;"SUM(";0)');
    expect(toCanonicalFormula("de", "=SUMME(A1)*MAX(B1:B2)")).toBe("=SUM(A1)*MAX(B1:B2)");
    expect(toCanonicalFormula("de", "=summe(A1)")).toBe("=SUM(A1)");
    expect(toDisplayFormula(null, "=SUM(A1)")).toBe("=SUM(A1)");
    expect(toDisplayFormula("fr", "plain text")).toBe("plain text");
    expect(mapFunctionNames("=A1.SUM(1)", () => "X")).toBe("=A1.SUM(1)");
    const fns = localizeFunctions("de", [{ name: "SUM", category: "Mathematical", signature: "SUM(number1, …)", description: "" }, { name: "MAX", category: "Statistical", signature: "MAX(a)", description: "" }]);
    expect(fns.map((f) => f.signature)).toEqual(["SUMME(number1, …)", "MAX(a)"]);
  });
});
