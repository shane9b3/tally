/** v0.3 helpers: links, rule builders, validation, page setup, parsing. */
import { describe, expect, it } from "vitest";
import {
  autoPageBreaks,
  colorByValueRules,
  defaultKind,
  defaultPageSetup,
  defaultRule,
  defaultValidation,
  insertToken,
  isColorByValue,
  isForbiddenLink,
  isOpenableUrl,
  linkError,
  listItems,
  moveItem,
  normalizeLink,
  operandFrom,
  operandText,
  paperMm,
  parseCell,
  parseNumber,
  parseRanges,
  parseSpan,
  presetRule,
  previewHeaderFooter,
  rangesText,
  ruleError,
  ruleSummary,
  spanText,
  usesStyle,
  validationError,
  HIGHLIGHT_PRESETS,
  ICON_SETS,
  iconThresholds,
  CF_RULE_TYPES,
} from "../src/v03";

describe("links", () => {
  it("opens only http(s) and mailto links (mirrors links.rs::check_url)", () => {
    for (const ok of ["https://example.com", "http://example.com/a?b=1&c=%20#x", "HTTPS://Example.com:8080/p", "mailto:a@example.com", " https://x.org "]) {
      expect(isOpenableUrl(ok), ok).toBe(true);
    }
    for (const bad of ["", "javascript:alert(1)", "file:///etc/passwd", "data:text/html,x", "ftp://x.org", "#Sheet1.A1", "example.com", "http://", "http:///x", "https://a b.com", 'https://x.org/"q"', "mailto:", `https://x.org/${"a".repeat(2048)}`]) {
      expect(isOpenableUrl(bad), bad).toBe(false);
    }
  });

  it("normalizes typed addresses", () => {
    expect(normalizeLink("example.com/orders")).toBe("https://example.com/orders");
    expect(normalizeLink("www.example.com")).toBe("https://www.example.com");
    expect(normalizeLink("sam@example.com")).toBe("mailto:sam@example.com");
    expect(normalizeLink("#Sheet2.A1")).toBe("#Sheet2.A1");
    expect(normalizeLink("http://x.org")).toBe("http://x.org");
    expect(normalizeLink("javascript:alert(1)")).toBe("javascript:alert(1)");
  });

  it("explains links that cannot be saved", () => {
    expect(isForbiddenLink(" JavaScript:x")).toBe(true);
    expect(linkError("javascript:alert(1)")).toMatch(/not allowed/);
    expect(linkError("data:text/html,x")).toMatch(/not allowed/);
    expect(linkError("#")).toMatch(/after #/);
    expect(linkError("#Sheet2.A1")).toBeNull();
    expect(linkError("https://example.com")).toBeNull();
    expect(linkError("file:///tmp/a.ods")).toBeNull();
    expect(linkError("foo bar")).toMatch(/web address/);
  });
});

describe("conditional formatting rules", () => {
  it("builds the Highlight presets", () => {
    expect(presetRule("gt", ["100"]).kind).toEqual({ type: "cellValue", op: "gt", formulas: ["100"] });
    expect(presetRule("between", ["1", "5"]).kind).toEqual({ type: "cellValue", op: "between", formulas: ["1", "5"] });
    expect(presetRule("eq", ["Paid"]).kind).toEqual({ type: "cellValue", op: "eq", formulas: ['"Paid"'] });
    expect(presetRule("textContains", [" café "]).kind).toEqual({ type: "text", op: "contains", text: "café" });
    expect(presetRule("duplicates", []).kind).toEqual({ type: "duplicate" });
    expect(presetRule("top10", []).kind).toEqual({ type: "top", rank: 10, percent: false, bottom: false });
    const scale = presetRule("colorScale", []);
    expect(scale.kind.type).toBe("colorScale");
    expect(usesStyle(scale.kind)).toBe(false);
    expect(scale.style.fill).toBeNull();
    expect(presetRule("dataBar", []).kind.type).toBe("dataBar");
    expect(presetRule("iconSet", []).kind).toMatchObject({ type: "iconSet", set: "threeArrows" });
    expect(HIGHLIGHT_PRESETS.map((p) => p.id)).toEqual(["gt", "lt", "between", "eq", "textContains", "duplicates", "top10", "colorScale", "dataBar", "iconSet"]);
  });

  it("quotes text operands and keeps numbers and references", () => {
    expect(operandFrom("42")).toBe("42");
    expect(operandFrom("-1.5e3")).toBe("-1.5e3");
    expect(operandFrom("=$B$1")).toBe("$B$1");
    expect(operandFrom("B2*2")).toBe("B2*2");
    expect(operandFrom('Say "hi"')).toBe('"Say ""hi"""');
    expect(operandText('"Say ""hi"""')).toBe('Say "hi"');
    expect(operandFrom("")).toBe("0");
  });

  it("summarizes rules", () => {
    expect(ruleSummary(presetRule("gt", ["100"]))).toBe("Cell value > 100");
    expect(ruleSummary(presetRule("between", ["1", "5"]))).toBe("Cell value between 1 and 5");
    expect(ruleSummary(presetRule("eq", ["Paid"]))).toBe("Cell value = Paid");
    expect(ruleSummary({ ...defaultRule("top"), kind: { type: "top", rank: 5, percent: true, bottom: true } })).toBe("Bottom 5%");
    expect(ruleSummary(defaultRule("formula"))).toBe("Formula: =");
    for (const t of CF_RULE_TYPES) expect(ruleSummary(defaultRule(t.type)).length).toBeGreaterThan(3);
  });

  it("validates rules in the editor", () => {
    expect(ruleError(defaultRule("formula"))).toMatch(/formula/);
    expect(ruleError({ ...defaultRule(), kind: { type: "cellValue", op: "gt", formulas: [""] } })).toMatch(/value/);
    expect(ruleError({ ...defaultRule(), kind: { type: "top", rank: 0, percent: false, bottom: false } })).toMatch(/Rank/);
    expect(ruleError(defaultRule("cellValue"))).toBeNull();
    expect(ruleError(defaultRule("colorScale"))).toBeNull();
  });

  it("icon sets have one threshold per icon", () => {
    for (const s of ICON_SETS) expect(iconThresholds(s.set).length).toBe(s.set.startsWith("three") ? 3 : s.set.startsWith("four") ? 4 : 5);
    expect(defaultKind("iconSet")).toMatchObject({ thresholds: [{ value: "0" }, { value: "33" }, { value: "67" }] });
  });

  it("Colour by value makes one chip rule per value", () => {
    const colors = new Map([
      ["Paid", { bg: 1, fg: 2 }],
      ["Due", { bg: 3, fg: 4 }],
    ]);
    const rules = colorByValueRules(["Paid", "Due"], colors);
    expect(rules.map((r) => r.kind)).toEqual([
      { type: "cellValue", op: "eq", formulas: ['"Paid"'] },
      { type: "cellValue", op: "eq", formulas: ['"Due"'] },
    ]);
    expect(rules[1]!.style.display).toEqual({ type: "chip", bg: 3, fg: 4 });
    expect(isColorByValue(rules)).toBe(true);
    expect(isColorByValue([presetRule("gt", ["1"])])).toBe(false);
  });

  it("moves rules up and down", () => {
    expect(moveItem([1, 2, 3], 0, 1)).toEqual([2, 1, 3]);
    expect(moveItem([1, 2, 3], 2, -1)).toEqual([1, 3, 2]);
    expect(moveItem([1, 2, 3], 0, -1)).toEqual([1, 2, 3]);
  });

  it("parses range lists", () => {
    expect(parseRanges("F2:F15, $H$2")).toEqual([
      { r0: 1, c0: 5, r1: 14, c1: 5 },
      { r0: 1, c0: 7, r1: 1, c1: 7 },
    ]);
    expect(parseRanges("nope")).toBeNull();
    expect(parseRanges("")).toBeNull();
    expect(rangesText([{ r0: 1, c0: 5, r1: 14, c1: 5 }])).toBe("F2:F15");
  });
});

describe("data validation", () => {
  it("builds a list validation for the selection", () => {
    const v = defaultValidation([{ r0: 1, c0: 6, r1: 14, c1: 6 }]);
    expect(v.kind).toEqual({ type: "list", source: { type: "values", items: [] } });
    expect(v.error?.style).toBe("stop");
    expect(validationError(v)).toMatch(/items/);
    expect(validationError({ ...v, kind: { type: "list", source: { type: "values", items: ["a"] } } })).toBeNull();
    expect(validationError({ ...v, kind: { type: "list", source: { type: "range", reference: "" } } })).toMatch(/source range/);
    expect(validationError({ ...v, kind: { type: "whole", op: "between", formulas: ["1", ""] } })).toMatch(/limits/);
    expect(validationError({ ...v, kind: { type: "whole", op: "gt", formulas: ["1", ""] } })).toBeNull();
    expect(validationError({ ...v, kind: { type: "custom", formula: "=" } })).toMatch(/formula/);
  });

  it("splits inline list items", () => {
    expect(listItems("Paid\nDue\n\n Overdue ")).toEqual(["Paid", "Due", "Overdue"]);
    expect(listItems("Paid, Due,Overdue")).toEqual(["Paid", "Due", "Overdue"]);
    expect(listItems("One, two\nThree")).toEqual(["One, two", "Three"]);
  });
});

describe("page setup", () => {
  it("orients paper sizes", () => {
    expect(paperMm(defaultPageSetup())).toEqual([210, 297]);
    expect(paperMm({ paper: { type: "letter" }, orientation: "landscape" })).toEqual([279.4, 215.9]);
    expect(paperMm({ paper: { type: "custom", widthMm: 100, heightMm: 50 }, orientation: "portrait" })).toEqual([100, 50]);
  });

  it("packs rows and columns onto pages", () => {
    const setup = defaultPageSetup();
    const rows = { runs: [], dflt: 24 };
    const cols = { runs: [], dflt: 100 };
    const area = { r0: 0, c0: 0, r1: 199, c1: 19 };
    const b = autoPageBreaks(setup, area, rows, cols);
    // A4 portrait minus 20 mm margins: 257 mm ≈ 971 px tall → 40 rows of 24 px; 170 mm ≈ 642 px wide → 6 columns.
    expect(b.rows.slice(0, 3)).toEqual([40, 80, 120]);
    expect(b.cols.slice(0, 2)).toEqual([6, 12]);
    // A manual break starts a new page there.
    const manual = autoPageBreaks({ ...setup, rowBreaks: [10] }, area, rows, cols);
    expect(manual.rows.slice(0, 2)).toEqual([10, 50]);
    // Scaling to 50 % fits twice as much.
    expect(autoPageBreaks({ ...setup, scale: { type: "percent", percent: 50 } }, area, rows, cols).rows[0]).toBe(80);
    // Fit to 1 page wide: no column breaks.
    expect(autoPageBreaks({ ...setup, scale: { type: "fitTo", width: 1, height: 0 } }, area, rows, cols).cols).toEqual([]);
    // Hidden rows take no space.
    const hidden = autoPageBreaks(setup, area, { runs: [{ start: 0, end: 9, size: 24, hidden: true }], dflt: 24 }, cols);
    expect(hidden.rows[0]).toBe(50);
  });

  it("inserts header/footer placeholders and previews them", () => {
    expect(insertToken("Page  of", 5, 5, "&[PAGE]")).toEqual({ text: "Page &[PAGE] of", caret: 12 });
    expect(insertToken("abc", 1, 2, "X")).toEqual({ text: "aXc", caret: 2 });
    expect(previewHeaderFooter("&[SHEET] p&[PAGE]/&[PAGES] &[FILE]", { sheet: "Orders", file: "Invoices", page: 2, pages: 3 })).toBe("Orders p2/3 Invoices");
  });

  it("parses rows/columns to repeat", () => {
    expect(parseSpan("1:2", "row")).toEqual([0, 1]);
    expect(parseSpan("3", "row")).toEqual([2, 2]);
    expect(parseSpan("B:A", "col")).toEqual([0, 1]);
    expect(parseSpan("", "row")).toBeNull();
    expect(parseSpan("x", "row")).toBeUndefined();
    expect(spanText([0, 1], "row")).toBe("1:2");
    expect(spanText([0, 2], "col")).toBe("A:C");
  });
});

describe("parsing", () => {
  it("reads cells and numbers", () => {
    expect(parseCell("$D$2")).toEqual({ r: 1, c: 3 });
    expect(parseCell("D2:D3")).toBeNull();
    expect(parseNumber("1,234.5")).toBe(1234.5);
    expect(parseNumber("12%")).toBeCloseTo(0.12);
    expect(parseNumber("abc")).toBeNull();
  });
});
