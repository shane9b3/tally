/** AutoFilter Top 10, Find & Replace search-in / current-selection options and the CSV Text Export checks. */
import { describe, expect, it } from "vitest";
import { DEFAULT_FIND, FILTER_OPS, findQuery, searchIn, TOP_10_FILTER, columnFilterFrom } from "../src/dataOps";
import { csvOptionsProblem, DEFAULT_CSV_OPTIONS, lastCsvOptions, useCsvPromptStore } from "../src/csvExport";

describe("AutoFilter Top 10", () => {
  it("is AOO's largest-10 condition and the condition list has the top/bottom operators", () => {
    expect(TOP_10_FILTER).toEqual({ values: null, condition: { op: "topValues", value: "10" } });
    const ops = FILTER_OPS.map((o) => o.op);
    expect(ops).toEqual(expect.arrayContaining(["topValues", "bottomValues", "topPercent", "bottomPercent"]));
    expect(columnFilterFrom(["1", "2"], new Set(["1", "2"]), { op: "bottomPercent", value: " 25 " })).toEqual({ values: null, condition: { op: "bottomPercent", value: "25" } });
  });
});

describe("find options", () => {
  it("search in notes and the current selection reach the query", () => {
    expect(searchIn(DEFAULT_FIND)).toBe("values");
    expect(searchIn({ ...DEFAULT_FIND, inFormulas: true })).toBe("formulas");
    const notes = { ...DEFAULT_FIND, inFormulas: true, inNotes: true };
    expect(searchIn(notes)).toBe("notes");
    const q = findQuery("x", notes, 3, null);
    expect(q.inNotes).toBe(true);
    expect(q.inFormulas).toBe(false);
    expect(q.selection).toBeNull();
    const scoped = findQuery("x", { ...DEFAULT_FIND, allSheets: true, scope: { sheet: 7, ranges: [{ r0: 0, c0: 0, r1: 4, c1: 1 }] } }, 3, null);
    // Current selection only searches the selection's sheet, not all sheets.
    expect(scoped.sheet).toBe(7);
    expect(scoped.selection).toEqual([{ r0: 0, c0: 0, r1: 4, c1: 1 }]);
    expect(findQuery("x", DEFAULT_FIND, 3, null).inNotes).toBeNull();
  });
});

describe("CSV Text Export", () => {
  it("checks the delimiters as the engine does", () => {
    expect(csvOptionsProblem(DEFAULT_CSV_OPTIONS)).toBeNull();
    expect(csvOptionsProblem({ ...DEFAULT_CSV_OPTIONS, fieldDelimiter: "\t", textDelimiter: "" })).toBeNull();
    expect(csvOptionsProblem({ ...DEFAULT_CSV_OPTIONS, fieldDelimiter: "" })).toBe("field");
    expect(csvOptionsProblem({ ...DEFAULT_CSV_OPTIONS, fieldDelimiter: "ab" })).toBe("field");
    expect(csvOptionsProblem({ ...DEFAULT_CSV_OPTIONS, fieldDelimiter: "\n" })).toBe("field");
    expect(csvOptionsProblem({ ...DEFAULT_CSV_OPTIONS, fieldDelimiter: "é" })).toBe("field");
    expect(csvOptionsProblem({ ...DEFAULT_CSV_OPTIONS, textDelimiter: "''" })).toBe("text");
    expect(csvOptionsProblem({ ...DEFAULT_CSV_OPTIONS, fieldDelimiter: '"' })).toBe("same");
  });

  it("remembers the last confirmed choices for the session", async () => {
    const store = useCsvPromptStore.getState();
    const first = store.askOptions("a.csv", true, "Sheet1");
    expect(useCsvPromptStore.getState().options?.initial).toEqual(DEFAULT_CSV_OPTIONS);
    const picked = { ...DEFAULT_CSV_OPTIONS, fieldDelimiter: ";", charset: "utf16" as const };
    useCsvPromptStore.getState().answerOptions(picked);
    expect(await first).toEqual(picked);
    expect(lastCsvOptions()).toEqual(picked);
    const second = useCsvPromptStore.getState().askOptions("b.csv", false, "Sheet1");
    expect(useCsvPromptStore.getState().options?.initial).toEqual(picked);
    useCsvPromptStore.getState().answerOptions(null);
    expect(await second).toBeNull();
    expect(lastCsvOptions()).toEqual(picked);
    const keep = useCsvPromptStore.getState().askKeep("a.csv", true, "Sheet1");
    useCsvPromptStore.getState().answerKeep("odf");
    expect(await keep).toBe("odf");
  });
});
