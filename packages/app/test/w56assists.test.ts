/**
 * W5-6C: AutoCorrect rules and table, the AutoInput matcher, the Function
 * Wizard argument model and the display-level function-name localisation.
 */
import { describe, expect, it } from "vitest";
import { applyAutoCorrect, applyAutoCorrectToEdit, applyReplacementTable, capitalizeSentences, fixOrdinals, fixTwoInitialCapitals, isPlainText, replaceDashes, smartQuotes } from "../src/autocorrect/autocorrect";
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
    // Typed with a leading + or -: AOO's formula mode turns AutoCorrect off.
    for (const s of ["-teh", "+B1+B2", "- note -- here"]) expect(applyAutoCorrect(s), s).toBe(s);
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
    // A leading + or - is formula mode (AOO): no column completion.
    expect(wantsCompletion("-Ap")).toBe(false);
    expect(wantsCompletion("+Ap")).toBe(false);
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
  });

  it("uses FunctionInfo.params, growing the repeating group as it fills", () => {
    const fn = (name: string, params: { name: string; optional?: boolean; repeat?: boolean }[]) => ({
      name,
      category: "Mathematical",
      signature: "ignored(x)",
      description: "",
      example: "",
      params: params.map((p) => ({ name: p.name, kind: "number", optional: p.optional ?? false, repeat: p.repeat ?? false })),
    });
    const sumif = wizardParams(fn("SUMIF", [{ name: "range" }, { name: "criterion" }, { name: "sum_range", optional: true }]));
    expect(sumif).toEqual([
      { name: "range", optional: false, variadic: false },
      { name: "criterion", optional: false, variadic: false },
      { name: "sum_range", optional: true, variadic: false },
    ]);
    expect(argumentSlots(sumif, []).map((p) => p.name)).toEqual(["range", "criterion", "sum_range"]);
    const sum = wizardParams(fn("SUM", [{ name: "number", repeat: true }]));
    expect(argumentSlots(sum, []).map((p) => `${p.name}:${p.optional}`)).toEqual(["number1:false"]);
    expect(argumentSlots(sum, ["A1"]).map((p) => p.name)).toEqual(["number1", "number2"]);
    expect(argumentSlots(sum, ["A1", "B1"]).map((p) => p.name)).toEqual(["number1", "number2", "number3"]);
    expect(argumentError(argumentSlots(sum, []), [])).toBe("number1 is required.");
    // SUMIFS: one fixed parameter, then (range; criterion) pairs.
    const sumifs = wizardParams(fn("SUMIFS", [{ name: "sum_range" }, { name: "range", repeat: true }, { name: "criterion", repeat: true }]));
    expect(argumentSlots(sumifs, []).map((p) => p.name)).toEqual(["sum_range", "range1", "criterion1"]);
    expect(argumentSlots(sumifs, ["C1:C9", "A1:A9"]).map((p) => p.name)).toEqual(["sum_range", "range1", "criterion1", "range2", "criterion2"]);
    expect(argumentSlots(sumifs, ["C1:C9", "A1:A9", '">1"']).map((p) => p.optional)).toEqual([false, false, false, true, true]);
    // At most 255 arguments.
    expect(argumentSlots(sum, Array.from({ length: 300 }, () => "1")).length).toBe(255);
    // No metadata: the signature is parsed.
    expect(wizardParams({ ...fn("X", []), signature: "X(a, [b])" }).map((p) => p.name)).toEqual(["a", "b"]);
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
    const fns = localizeFunctions("de", [{ name: "SUM", category: "Mathematical", signature: "SUM(number1, …)", description: "", params: [], example: "" }, { name: "MAX", category: "Statistical", signature: "MAX(a)", description: "", params: [], example: "" }]);
    expect(fns.map((f) => f.signature)).toEqual(["SUMME(number1, …)", "MAX(a)"]);
  });
});

describe("localized number literals and quoted names (review 4 H15/M41)", () => {
  it("swaps the decimal separator in numeric literals only", () => {
    expect(toCanonicalFormula("de", "=SUMME(1;1,5)")).toBe("=SUM(1;1.5)");
    expect(toCanonicalFormula("fr", "=ARRONDI(2,345;2)")).toBe("=ROUND(2.345;2)");
    expect(toCanonicalFormula("es", '=SUMA(A1;12,25)&"1,5"')).toBe('=SUM(A1;12.25)&"1,5"');
    expect(toDisplayFormula("de", "=SUM(1;1.5)")).toBe("=SUMME(1;1,5)");
    expect(toDisplayFormula("de", '=SUM(A1;2.5)&"1.5"')).toBe('=SUMME(A1;2,5)&"1.5"');
    expect(toCanonicalFormula("de", "=SUMME(10;20)")).toBe("=SUM(10;20)");
    expect(toDisplayFormula(null, "=SUM(1;1.5)")).toBe("=SUM(1;1.5)");
  });
  it("leaves quoted sheet names and strings alone", () => {
    expect(toCanonicalFormula("de", "='Summe (2024)'.A1*2")).toBe("='Summe (2024)'.A1*2");
    expect(toDisplayFormula("de", "='Sum (1.5)'.A1+SUM(1.5)")).toBe("='Sum (1.5)'.A1+SUMME(1,5)");
    expect(toCanonicalFormula("de", "='It''s 1,5'.A1+SUMME(1,5)")).toBe("='It''s 1,5'.A1+SUM(1.5)");
  });
});

describe("AutoCorrect scope (review 4 M42/M43)", () => {
  it("only corrects newly typed text, not what was already in the cell", () => {
    expect(applyAutoCorrectToEdit("iPhone and eBay", "iPhone and eBay sale", DEFAULT_AUTOCORRECT)).toBe("iPhone and eBay sale");
    expect(applyAutoCorrectToEdit("MHz clock", "MHz clock teh", DEFAULT_AUTOCORRECT)).toBe("MHz clock the");
    expect(applyAutoCorrectToEdit("hello world", "hello world", DEFAULT_AUTOCORRECT)).toBe("hello world");
    expect(applyAutoCorrectToEdit(null, "teh cat sat", DEFAULT_AUTOCORRECT)).toBe("The cat sat");
  });
  it("does not capitalise a lone word", () => {
    expect(applyAutoCorrect("hello", DEFAULT_AUTOCORRECT)).toBe("hello");
    expect(applyAutoCorrect("hello there", DEFAULT_AUTOCORRECT)).toBe("Hello there");
  });
  it("handles non-ASCII letters and the exception list", () => {
    expect(fixTwoInitialCapitals("ÄRGer")).toBe("ÄRGer");
    expect(fixTwoInitialCapitals("ÉCole")).toBe("École");
    expect(fixTwoInitialCapitals("CDs IDs MHz")).toBe("CDs IDs MHz");
    expect(capitalizeSentences("ça marche. élan vital")).toBe("Ça marche. Élan vital");
  });
});
