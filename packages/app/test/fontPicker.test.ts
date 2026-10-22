// @vitest-environment happy-dom
/**
 * Font pickers (fonts/fontModel.ts, components/FontPickers.tsx, Format
 * Cells ▸ Font): list sections and filtering, keyboard in the toolbar combo
 * box, recently used fonts, the selection's font (mixed = blank), typeface
 * mapping, and the patches a pick sends (one ApplyStyle on the selection).
 */
import { FALLBACK_FONTS, type FontFamilyInfo, type StyleDto } from "@tally/bridge";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FontNameBox, FontSizeBox, type FontInfo } from "../src/components/FontPickers";
import { ControllerContext } from "../src/context";
import { AppController } from "../src/controller";
import { WebMockClient } from "../src/dev/webMock";
import {
  cleanFontName,
  documentFonts,
  familyTypefaces,
  fontFamilyPatch,
  fontRows,
  parseFontSize,
  selectionFont,
  stepRow,
  typeAheadIndex,
  typefaceFields,
  typefaceOf,
  withRecent,
} from "../src/fonts/fontModel";
import { readRecentFonts, useFontStore } from "../src/fonts/fontStore";
import { EMPTY_PATCH } from "../src/formats";
import { mixedOf, patchFromState, patchIsEmpty, shownFamily, stateFromStyle, styleFamily, withFamily, withTypeface } from "../src/styles/formatCells";
import { useSelectionStore } from "../src/stores/selectionStore";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const tick = () => new Promise((r) => setTimeout(r, 0));
const fam = (family: string, faces: [number, boolean][] = [[400, false]], stretch = 5): FontFamilyInfo => ({ family, faces: faces.map(([weight, italic]) => ({ weight, italic, stretch })), monospaced: false });
const FAMILIES = [fam("Arial"), fam("Avenir"), fam("Courier New"), fam("Helvetica"), fam("Inter"), fam("Lucida Grande"), fam("Times New Roman")];
const style = (id: number, over: Partial<StyleDto> = {}): StyleDto => ({ id, bold: false, italic: false, underline: false, strike: false, fontFamily: null, fontSizePt: 11, color: null, fill: null, hAlign: "general", vAlign: "bottom", wrap: false, indent: 0, borders: { top: null, bottom: null, left: null, right: null }, numberFormat: "General", display: null, ...over });

describe("font list (pure)", () => {
  it("has recent, document and installed sections; filters with prefix matches first", () => {
    const rows = fontRows("", FAMILIES, ["Times New Roman"], ["Arial", "Inter"]);
    expect(rows.filter((r) => r.kind === "header").map((r) => r.section)).toEqual(["recent", "document", "all"]);
    expect(rows[1]).toEqual({ kind: "font", family: "Times New Roman", section: "recent" });
    expect(rows.filter((r) => r.kind === "font" && r.section === "all")).toHaveLength(FAMILIES.length);
    const q = fontRows("a", FAMILIES, ["Times New Roman"], ["Arial"]).map((r) => (r.kind === "font" ? r.family : ""));
    // Starting with "a" first, then containing it; each name once.
    expect(q).toEqual(["Arial", "Avenir", "Times New Roman", "Helvetica", "Lucida Grande"]);
    expect(fontRows("zz", FAMILIES, [], [])).toEqual([]);
    const typed = fontRows("hel", FAMILIES, [], []);
    expect(typeAheadIndex(typed, "hel")).toBe(0);
    expect(typeAheadIndex(fontRows("", FAMILIES, [], []), "")).toBe(-1);
    // Headers are skipped when stepping.
    expect(stepRow(rows, 1, 1)).toBe(3);
    expect(stepRow(rows, 3, -1)).toBe(1);
    expect(stepRow(rows, 1, -1)).toBe(1);
  });

  it("keeps recent fonts unique, newest first, bounded", () => {
    let r: string[] = [];
    for (const n of ["A", "B", "C", "a", "D", "E", "F", "G", "H", "I"]) r = withRecent(r, n);
    expect(r).toEqual(["I", "H", "G", "F", "E", "D", "a", "C"]);
    expect(withRecent(["X"], "  ")).toEqual(["X"]);
    expect(cleanFontName(" Bad\u0007Name ")).toBe("BadName");
    expect(cleanFontName("x".repeat(300))).toHaveLength(128);
  });

  it("lists the document's fonts from its styles", () => {
    expect(documentFonts("Arial", [style(1, { fontFamily: "Georgia" }), style(2, { fontFamily: "arial" }), undefined, style(3)])).toEqual(["Arial", "Georgia"]);
  });

  it("parses sizes as half points within 1–409", () => {
    expect(parseFontSize("10.5")).toBe(10.5);
    expect(parseFontSize("10,5")).toBe(10.5);
    expect(parseFontSize(" 12 pt")).toBe(12);
    expect(parseFontSize("10.3")).toBe(10.5);
    expect(parseFontSize("0")).toBeNull();
    expect(parseFontSize("410")).toBeNull();
    expect(parseFontSize("abc")).toBeNull();
    expect(parseFontSize("")).toBeNull();
  });
});

describe("typefaces", () => {
  it("maps faces to bold/italic and marks synthesised ones and extra weights", () => {
    expect(typefaceOf(true, true)).toBe("boldItalic");
    expect(typefaceFields("italic")).toEqual({ bold: false, italic: true });
    const full = familyTypefaces(fam("Helvetica", [[400, false], [400, true], [700, false], [700, true]]));
    expect(full.faces.every((f) => f.real)).toBe(true);
    expect(full.extra).toBe(false);
    const light = familyTypefaces(fam("Avenir", [[300, false], [500, false], [800, true]]));
    expect(light.faces.map((f) => [f.typeface, f.real])).toEqual([
      ["regular", true],
      ["italic", false],
      ["bold", false],
      ["boldItalic", true],
    ]);
    expect(light.extra).toBe(true);
    expect(familyTypefaces(fam("Narrow", [[400, false]], 3)).extra).toBe(true);
    // Not installed: all four offered.
    expect(familyTypefaces(undefined).faces.every((f) => f.real)).toBe(true);
  });
});

describe("selection font", () => {
  const styles = new Map<number, StyleDto>([
    [0, style(0, { fontFamily: "Arial" })],
    [1, style(1, { fontFamily: "Arial", bold: true })],
    [2, style(2, { fontFamily: "Georgia", fontSizePt: 14 })],
  ]);
  const cells = new Map<string, number>([
    ["0,0", 1],
    ["0,1", 1],
    ["1,0", 2],
  ]);
  const source = {
    getCell: (r: number, c: number) => (cells.has(`${r},${c}`) ? { style: cells.get(`${r},${c}`)! } : undefined),
    getStyle: (id: number) => styles.get(id),
  };
  const used = { r0: 0, c0: 0, r1: 1, c1: 1 };

  it("is the common font, or null fields where the selection mixes", () => {
    expect(selectionFont(source, [{ r0: 0, c0: 0, r1: 0, c1: 1 }], { r: 0, c: 0 }, used)).toEqual({ family: "Arial", sizePt: 11, bold: true, italic: false });
    const mixed = selectionFont(source, [{ r0: 0, c0: 0, r1: 1, c1: 0 }], { r: 0, c: 0 }, used);
    expect(mixed).toEqual({ family: null, sizePt: null, bold: null, italic: false });
    expect(mixedOf(mixed)).toEqual({ fontFamily: true, fontSizePt: true, typeface: true });
    // Empty cells (style 0, the document font) count, including those past the used range.
    expect(selectionFont(source, [{ r0: 0, c0: 0, r1: 5, c1: 1 }], { r: 0, c: 0 }, used)?.bold).toBeNull();
    // A family-less style is the app font.
    expect(selectionFont({ getCell: () => undefined, getStyle: () => style(0) }, [{ r0: 0, c0: 0, r1: 0, c1: 0 }], { r: 0, c: 0 }, null)?.family).toBe("Inter");
    // Scanning is bounded.
    let calls = 0;
    const counting = { getCell: () => (calls++, undefined), getStyle: () => style(0) };
    selectionFont(counting, [{ r0: 0, c0: 0, r1: 999_999, c1: 99 }], { r: 0, c: 0 }, { r0: 0, c0: 0, r1: 999_999, c1: 99 });
    expect(calls).toBeLessThanOrEqual(10_001);
  });
});

describe("patches", () => {
  it("a font pick sets the family; the document font clears it", () => {
    expect(fontFamilyPatch("Georgia", "Arial")).toEqual({ ...EMPTY_PATCH, fontFamily: "Georgia" });
    expect(fontFamilyPatch("arial", "Arial")).toEqual({ ...EMPTY_PATCH, fontFamily: "" });
    expect(fontFamilyPatch("Not Installed Font", "Inter")).toEqual({ ...EMPTY_PATCH, fontFamily: "Not Installed Font" });
  });

  it("Format Cells sends only changed fields, and mixed fields only once set", () => {
    const base = style(5, { fontFamily: "Arial" });
    const st = stateFromStyle(base, { fontFamily: true, typeface: true });
    expect(patchIsEmpty(patchFromState(st, base))).toBe(true);
    // Picking the family the active cell already has still unifies a mixed selection.
    const fam1 = patchFromState(withFamily(st, "Arial"), base);
    expect(fam1.fontFamily).toBe("Arial");
    expect(fam1.bold).toBeNull();
    const bi = patchFromState(withTypeface(st, "boldItalic"), base);
    expect([bi.bold, bi.italic, bi.fontFamily, bi.fontSizePt]).toEqual([true, true, null, null]);
    // Not mixed: the same family is no change; the app font equals "no family".
    const plain = stateFromStyle(style(6));
    expect(patchFromState(withFamily(plain, "Inter"), style(6)).fontFamily).toBeNull();
    expect(patchFromState(withFamily(plain, "Georgia"), style(6)).fontFamily).toBe("Georgia");
    expect(patchFromState(withTypeface(plain, "bold"), style(6))).toMatchObject({ bold: true, italic: null });
  });

  it("applying a font from the picker is one ApplyStyle on the whole selection; undo restores", async () => {
    const client = new WebMockClient();
    const ctl = new AppController(client, client, { nativeMenu: false });
    await ctl.start("invoices");
    await tick();
    useSelectionStore.getState().setSelection({ range: { r0: 1, c0: 0, r1: 3, c1: 2 }, active: { r: 1, c: 0 }, anchor: { r: 1, c: 0 }, others: [{ r0: 6, c0: 4, r1: 6, c1: 4 }] });
    const before = client.recorded.length;
    await ctl.setFontFamily("Georgia", "Inter");
    const ops = client.recorded.slice(before);
    expect(ops).toEqual([{ type: "applyStyle", sheet: ctl.sheet, ranges: [{ r0: 6, c0: 4, r1: 6, c1: 4 }, { r0: 1, c0: 0, r1: 3, c1: 2 }], patch: { ...EMPTY_PATCH, fontFamily: "Georgia" } }]);
    const doc = ctl.info!.doc;
    const cell = (await client.cells(doc, ctl.sheet, [{ r0: 2, c0: 1, r1: 2, c1: 1 }]))[0]!.cells[0]!;
    expect((await client.styles(doc, [cell.style]))[0]?.fontFamily).toBe("Georgia");
    await ctl.undo();
    const after = (await client.cells(doc, ctl.sheet, [{ r0: 2, c0: 1, r1: 2, c1: 1 }]))[0]!.cells[0]!;
    expect((await client.styles(doc, [after.style]))[0]?.fontFamily ?? null).toBeNull();
    await ctl.setFontFamily("Inter", "Inter");
    expect(client.recorded.at(-1)).toMatchObject({ type: "applyStyle", patch: { fontFamily: "" } });
  });
});

describe("toolbar combo boxes", () => {
  let host: HTMLDivElement;
  let root: Root;
  let ctl: { shell: { listFonts: (r?: boolean) => Promise<FontFamilyInfo[]> }; setFontFamily: ReturnType<typeof vi.fn>; setFontSize: ReturnType<typeof vi.fn>; notifyError: ReturnType<typeof vi.fn> };

  beforeEach(async () => {
    localStorage.clear();
    useFontStore.setState({ families: [...FALLBACK_FONTS], loaded: false, recent: [] });
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    ctl = { shell: { listFonts: vi.fn(() => Promise.resolve(FAMILIES)) }, setFontFamily: vi.fn(() => Promise.resolve(null)), setFontSize: vi.fn(() => Promise.resolve(null)), notifyError: vi.fn() };
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  const INFO: FontInfo = { sel: undefined, docFont: "Inter", docFonts: ["Inter"] };
  async function mount(el: typeof FontNameBox | typeof FontSizeBox, onDone = vi.fn(), info: FontInfo = INFO) {
    act(() => root.render(createElement(ControllerContext.Provider, { value: ctl as unknown as AppController }, createElement(el, { onDone, info }))));
    await act(async () => {
      await tick();
    });
    return onDone;
  }

  const input = (id: string) => host.querySelector<HTMLInputElement>(`[data-testid="${id}-input"]`)!;
  const key = (el: HTMLElement, k: string) => act(() => void el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true })));
  const type = (el: HTMLInputElement, text: string) =>
    act(() => {
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      set.call(el, text);
      el.dispatchEvent(new Event("input", { bubbles: true }));
    });
  const options = () => [...document.querySelectorAll<HTMLElement>('[data-testid="font-name-list"] [role="option"]')].map((o) => o.dataset.value);

  it("loads the installed list, shows the font, filters while typing, Enter applies the match and remembers it", async () => {
    const done = await mount(FontNameBox);
    expect(ctl.shell.listFonts).toHaveBeenCalledTimes(1);
    const box = input("font-name");
    expect(box.getAttribute("role")).toBe("combobox");
    expect(box.value).toBe("Inter");
    act(() => box.focus());
    type(box, "tim");
    expect(box.getAttribute("aria-expanded")).toBe("true");
    expect(options()).toEqual(["Times New Roman"]);
    const active = box.getAttribute("aria-activedescendant");
    expect(active && document.getElementById(active)?.dataset.value).toBe("Times New Roman");
    // Each name is drawn in its own face.
    expect(document.getElementById(active!)?.querySelector("span")?.getAttribute("style")).toContain("Times New Roman");
    key(box, "Enter");
    expect(ctl.setFontFamily).toHaveBeenCalledWith("Times New Roman", "Inter");
    expect(done).toHaveBeenCalled();
    expect(useFontStore.getState().recent).toEqual(["Times New Roman"]);
    expect(readRecentFonts()).toEqual(["Times New Roman"]);
  });

  it("arrow keys walk the sections (recent first), Esc closes then leaves; unknown names apply as typed", async () => {
    useFontStore.setState({ recent: ["Courier New"] });
    const done = await mount(FontNameBox);
    const box = input("font-name");
    act(() => box.focus());
    key(box, "ArrowDown");
    expect(box.getAttribute("aria-expanded")).toBe("true");
    const headers = [...document.querySelectorAll('[data-testid="font-name-list"] [role="presentation"]')].map((h) => h.textContent);
    expect(headers).toEqual(["Recently used", "Fonts in this document", "All fonts"]);
    // The current font (Inter, in "Fonts in this document") is highlighted; ↑ goes to the recent one.
    const cur = () => document.getElementById(box.getAttribute("aria-activedescendant") ?? "")?.dataset.value;
    expect(cur()).toBe("Inter");
    key(box, "ArrowUp");
    expect(cur()).toBe("Courier New");
    expect(box.value).toBe("Courier New");
    key(box, "Escape");
    expect(box.getAttribute("aria-expanded")).toBe("false");
    key(box, "Escape");
    expect(done).toHaveBeenCalled();
    expect(ctl.setFontFamily).not.toHaveBeenCalled();
    act(() => box.focus());
    type(box, "My Missing Font");
    expect(options()).toEqual([]);
    key(box, "Enter");
    expect(ctl.setFontFamily).toHaveBeenCalledWith("My Missing Font", "Inter");
  });

  it("the size box takes typed half points and rejects nonsense", async () => {
    await mount(FontSizeBox);
    const box = input("font-size");
    expect(box.value).toBe("10");
    act(() => box.focus());
    type(box, "10.5");
    key(box, "Enter");
    expect(ctl.setFontSize).toHaveBeenCalledWith(10.5);
    act(() => box.focus());
    type(box, "999");
    key(box, "Enter");
    expect(ctl.setFontSize).toHaveBeenCalledTimes(1);
    expect(ctl.notifyError).toHaveBeenCalled();
    key(box, "Escape");
    key(box, "Escape");
    act(() => box.focus());
    key(box, "ArrowDown");
    const listed = [...document.querySelectorAll<HTMLElement>('[data-testid="font-size-list"] [role="option"]')].map((o) => o.dataset.value);
    expect(listed[0]).toBe("6");
    expect(listed).toContain("10.5");
  });

  it("Enter on an unchanged box applies nothing (no undo step); a mixed selection applies any pick", async () => {
    const done = await mount(FontNameBox, vi.fn(), { sel: { family: "Georgia", sizePt: 12, bold: false, italic: false }, docFont: "Inter", docFonts: ["Georgia", "Inter"] });
    const box = input("font-name");
    expect(box.value).toBe("Georgia");
    act(() => box.focus());
    key(box, "Enter");
    expect(ctl.setFontFamily).not.toHaveBeenCalled();
    expect(done).toHaveBeenCalled();
    act(() => box.focus());
    type(box, "georgia");
    key(box, "Enter");
    expect(ctl.setFontFamily).not.toHaveBeenCalled();
    act(() => root.unmount());
    root = createRoot(host);
    await mount(FontNameBox, vi.fn(), { sel: { family: null, sizePt: 12, bold: false, italic: false }, docFont: "Inter", docFonts: [] });
    const mixed = input("font-name");
    expect(mixed.value).toBe("");
    act(() => mixed.focus());
    type(mixed, "Georgia");
    key(mixed, "Enter");
    expect(ctl.setFontFamily).toHaveBeenCalledWith("Georgia", "Inter");
  });

  it("the size box skips an unchanged size", async () => {
    await mount(FontSizeBox, vi.fn(), { sel: { family: "Inter", sizePt: 12, bold: false, italic: false }, docFont: "Inter", docFonts: [] });
    const box = input("font-size");
    act(() => box.focus());
    type(box, "12");
    key(box, "Enter");
    expect(ctl.setFontSize).not.toHaveBeenCalled();
  });

  it("mouse presses anywhere in the open list keep the field focused", async () => {
    await mount(FontNameBox);
    const box = input("font-name");
    act(() => box.focus());
    key(box, "ArrowDown");
    const header = document.querySelector<HTMLElement>('[data-testid="font-name-list"] [role="presentation"]')!;
    const ev = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    act(() => void header.dispatchEvent(ev));
    expect(ev.defaultPrevented).toBe(true);
    const popup = document.querySelector<HTMLElement>('[data-testid="font-name-list"]')!.parentElement!;
    const ev2 = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    act(() => void popup.dispatchEvent(ev2));
    expect(ev2.defaultPrevented).toBe(true);
  });
});

describe("font list loading (single flight)", () => {
  beforeEach(() => useFontStore.setState({ families: [...FALLBACK_FONTS], loaded: false }));

  it("joins running requests and never lets a plain load overwrite a refresh", async () => {
    const resolvers: ((v: FontFamilyInfo[]) => void)[] = [];
    const calls: boolean[] = [];
    const shell = { listFonts: (refresh?: boolean) => (calls.push(refresh ?? false), new Promise<FontFamilyInfo[]>((r) => resolvers.push(r))) };
    const a = useFontStore.getState().load(shell);
    const b = useFontStore.getState().load(shell);
    expect(a).toBe(b);
    const r1 = useFontStore.getState().load(shell, true);
    const r2 = useFontStore.getState().load(shell, true);
    expect(r1).toBe(r2);
    // The refresh waits for the plain load: one request at a time.
    await tick();
    expect(calls).toEqual([false]);
    resolvers[0]!([fam("Old")]);
    await a;
    await tick();
    expect(calls).toEqual([false, true]);
    resolvers[1]!([fam("New")]);
    await r1;
    expect(useFontStore.getState().families.map((f) => f.family)).toEqual(["New"]);
    expect(useFontStore.getState().loaded).toBe(true);
  });
});

describe("Format Cells font field and the document font", () => {
  it("an emptied field means the document font", () => {
    const base = style(5, { fontFamily: "Arial" });
    const st = stateFromStyle(base);
    // Arial is the document font: emptying the field changes nothing.
    expect(patchFromState(withFamily(st, ""), base, "Arial").fontFamily).toBeNull();
    // A cell in Georgia, document font Arial: emptying resets it to the document font.
    const g = style(6, { fontFamily: "Georgia" });
    expect(patchFromState(withFamily(stateFromStyle(g), ""), g, "Arial").fontFamily).toBe("");
    expect(patchFromState(withFamily(stateFromStyle(g), "arial"), g, "Arial").fontFamily).toBe("arial");
    expect(patchFromState(withFamily(stateFromStyle(base), "Inter"), base, "Arial").fontFamily).toBe("Inter");
  });

  it("named styles: empty or the document font is null, anything else is kept (Inter too)", () => {
    expect(styleFamily("", "Arial")).toBeNull();
    expect(styleFamily(" arial ", "Arial")).toBeNull();
    expect(styleFamily("Inter", "Arial")).toBe("Inter");
    expect(styleFamily("Georgia", "Inter")).toBe("Georgia");
    // A family-less named style shows the document font in the dialog.
    expect(shownFamily(stateFromStyle(style(1)).fontFamily, "Arial")).toBe("Arial");
  });
});
