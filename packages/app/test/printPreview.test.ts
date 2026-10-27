/**
 * W5-5: Print Preview reducers (paging, zoom steps, fit zoom, dpi) and the
 * print store.
 */
import { describe, expect, it } from "vitest";
import { clampPage, clampPreviewZoom, DEFAULT_PRINT_SCOPE, dpiFor, fitWidthZoom, fitZoom, MAX_PAGE_RANGES, MAX_PRINT_PAGE, pageLabel, paperPx, parsePageRanges, printOptionsFor, stepPage, stepPreviewZoom, thumbWindow } from "../src/print/printReducer";
import { usePrintStore } from "../src/print/printStore";

describe("print preview reducers", () => {
  it("clamps and steps pages", () => {
    expect(clampPage(5, 3)).toBe(2);
    expect(clampPage(-1, 3)).toBe(0);
    expect(clampPage(2, 0)).toBe(0);
    expect(stepPage(0, 3, -1)).toBe(0);
    expect(stepPage(0, 3, 1)).toBe(1);
    expect(stepPage(2, 3, 1)).toBe(2);
    expect(pageLabel(0, 3)).toBe("Page 1 of 3");
    expect(pageLabel(0, 0)).toBe("No pages");
  });
  it("steps zoom through the presets and clamps", () => {
    expect(stepPreviewZoom(1, 1)).toBe(1.25);
    expect(stepPreviewZoom(1, -1)).toBe(0.75);
    expect(stepPreviewZoom(3, 1)).toBe(3);
    expect(stepPreviewZoom(0.25, -1)).toBe(0.25);
    expect(clampPreviewZoom(9)).toBe(3);
    expect(clampPreviewZoom(NaN)).toBe(1);
  });
  it("fits a page in a viewport", () => {
    expect(paperPx([210, 297])).toEqual({ width: 794, height: 1123 });
    const z = fitZoom([210, 297], { width: 900, height: 700 });
    expect(z).toBeCloseTo((700 - 48) / 1123, 2);
    expect(fitWidthZoom([210, 297], { width: 900 })).toBeCloseTo((900 - 48) / 794, 2);
    expect(dpiFor(1)).toBe(96);
    expect(dpiFor(2, 2)).toBe(384);
    expect(dpiFor(3, 3)).toBe(600);
  });
  it("renders a window of thumbnails around the page", () => {
    expect(thumbWindow(0, 3)).toEqual([0, 1, 2]);
    expect(thumbWindow(20, 50, 6)).toEqual([17, 18, 19, 20, 21, 22]);
    expect(thumbWindow(49, 50, 6)).toEqual([44, 45, 46, 47, 48, 49]);
  });
  it("the store clamps the page to the layout and resets on close", () => {
    const st = usePrintStore.getState();
    st.setOpen(true);
    st.setJob(4, { pages: [page(0), page(1)], warnings: [] }, 7);
    st.setPage(9);
    expect(usePrintStore.getState().page).toBe(1);
    st.setPagePng("1:96", "data:x");
    st.setJob(5, { pages: [page(0)], warnings: [] }, 8);
    expect(usePrintStore.getState().page).toBe(0);
    expect(usePrintStore.getState().pages).toEqual({});
    st.setOpen(false);
    expect(usePrintStore.getState().job).toBeNull();
    expect(usePrintStore.getState().open).toBe(false);
  });
});

describe("print range", () => {
  it("parses page lists like AOO's Pages field", () => {
    expect(parsePageRanges("")).toEqual({ ok: true, ranges: null });
    expect(parsePageRanges("   ")).toEqual({ ok: true, ranges: null });
    expect(parsePageRanges("1-3,5")).toEqual({ ok: true, ranges: [[1, 3], [5, 5]] });
    expect(parsePageRanges(" 2 - 4 ; 7 ")).toEqual({ ok: true, ranges: [[2, 4], [7, 7]] });
    expect(parsePageRanges("4-")).toEqual({ ok: true, ranges: [[4, MAX_PRINT_PAGE]] });
    expect(parsePageRanges("4-", 6)).toEqual({ ok: true, ranges: [[4, 6]] });
    // Tolerant of empty items, as AOO.
    expect(parsePageRanges("1, ")).toEqual({ ok: true, ranges: [[1, 1]] });
    expect(parsePageRanges("1,,2;")).toEqual({ ok: true, ranges: [[1, 1], [2, 2]] });
    expect(parsePageRanges(" , ;")).toEqual({ ok: true, ranges: null });
  });
  it("rejects invalid page lists", () => {
    for (const [spec, error] of [
      ["abc", "syntax"],
      ["1-2-3", "syntax"],
      ["-3", "syntax"],
      ["1.5", "syntax"],
      ["0", "zero"],
      ["0-2", "zero"],
      ["5-3", "reversed"],
      [String(MAX_PRINT_PAGE + 1), "tooLarge"],
      [Array.from({ length: MAX_PAGE_RANGES + 1 }, (_, i) => String(i + 1)).join(","), "tooMany"],
    ] as const) {
      const r = parsePageRanges(spec);
      expect(r.ok, spec).toBe(false);
      if (!r.ok) expect(r.error, spec).toBe(error);
    }
  });
  it("checks page lists against the page count", () => {
    expect(parsePageRanges("2-9", 4)).toEqual({ ok: true, ranges: [[2, 4]] });
    expect(parsePageRanges("5", 4)).toMatchObject({ ok: false, error: "outOfRange", part: "5" });
    expect(parsePageRanges("1, 6-8", 4)).toMatchObject({ ok: false, error: "outOfRange", part: "6-8" });
    expect(parsePageRanges("4", 4)).toEqual({ ok: true, ranges: [[4, 4]] });
  });
  it("builds engine options on the user's sheet", () => {
    expect(DEFAULT_PRINT_SCOPE).toBe("selected");
    const sel = [{ r0: 1, c0: 1, r1: 4, c1: 2 }];
    expect(printOptionsFor("selected", null, 2, sel)).toEqual({ sheets: { type: "active" }, selection: null, suppressEmptyPages: true, pageFrom: null, pageTo: null, activeSheet: 2, pageRanges: null });
    expect(printOptionsFor("all", [[1, 3]], 2, sel)).toMatchObject({ sheets: { type: "all" }, selection: null, activeSheet: 2, pageRanges: [[1, 3]] });
    expect(printOptionsFor("selection", null, 3, sel)).toMatchObject({ sheets: { type: "active" }, selection: sel, activeSheet: 3 });
  });
  it("the store keeps the chosen range when the preview closes", () => {
    const st = usePrintStore.getState();
    st.reset();
    expect(usePrintStore.getState().scope).toBe("selected");
    st.setRange(1, "all", "1-2");
    st.setOpen(true);
    st.setOpen(false);
    expect(usePrintStore.getState()).toMatchObject({ scope: "all", pageSpec: "1-2", scopeDoc: 1 });
    st.reset();
    expect(usePrintStore.getState()).toMatchObject({ scope: "selected", pageSpec: "", scopeDoc: null });
  });
});

function page(index: number) {
  return { index, number: index + 1, sheet: 1, rows: [0, 10] as [number, number], cols: [0, 5] as [number, number], repeatRows: null, repeatCols: null, scalePct: 100, paperMm: [210, 297] as [number, number], originMm: [20, 20] as [number, number], header: null, footer: null };
}
