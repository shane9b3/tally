// @vitest-environment happy-dom
/**
 * Toolbar customisation (chrome/chromeModel.ts, chromeActions.ts): the
 * prefs shape and its validation, separators collapsing around hidden
 * buttons, the overflow fit, and persistence through the app prefs
 * (localStorage in the browser build, `set_prefs` + broadcast in the
 * desktop app).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { openCustomize, saveChrome, toggleBar, usePeekStore } from "../src/chrome/chromeActions";
import { useEditStore } from "../src/stores/editStore";
import {
  DEFAULT_CHROME,
  FORMATTING_SECTIONS,
  TOOLBAR_ITEMS,
  TOOLBAR_SECTIONS,
  chooseLayout,
  fitCount,
  isDefaultChrome,
  isItemVisible,
  parseChrome,
  slotsOf,
  visibleSections,
  withBar,
  withItem,
} from "../src/chrome/chromeModel";
import { AppController } from "../src/controller";
import { useW56Store } from "../src/data/store";
import { WebMockClient } from "../src/dev/webMock";
import { DEFAULT_PREFS, parsePrefs, readLocalPrefs, serializePrefs } from "../src/options/prefs";

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("chrome prefs", () => {
  it("default to everything visible and validate what they read", () => {
    expect(DEFAULT_PREFS.chrome).toEqual(DEFAULT_CHROME);
    expect(isDefaultChrome(parsePrefs({}).chrome)).toBe(true);
    const c = parseChrome({ bars: { toolbar: false, statusBar: "no", bogus: true }, hidden: { formatting: ["percent", "bold", "nope", 3], toolbar: "sort", other: ["x"] } });
    expect(c.bars).toEqual({ toolbar: false, formatting: true, formulaBar: true, statusBar: true });
    // Known ids only, in toolbar order.
    expect(c.hidden).toEqual({ toolbar: [], formatting: ["bold", "percent"] });
    expect(parsePrefs(JSON.parse(serializePrefs({ ...DEFAULT_PREFS, chrome: c }))).chrome).toEqual(c);
  });

  it("toggles bars and buttons", () => {
    let c = withItem(DEFAULT_CHROME, "toolbar", "sort", false);
    expect(isItemVisible(c, "toolbar", "sort")).toBe(false);
    c = withItem(c, "toolbar", "save", false);
    expect(c.hidden.toolbar).toEqual(["save", "sort"]);
    c = withItem(c, "toolbar", "sort", true);
    expect(c.hidden.toolbar).toEqual(["save"]);
    expect(withBar(c, "formulaBar", false).bars.formulaBar).toBe(false);
    expect(isDefaultChrome(c)).toBe(false);
    // Every button has a label key.
    expect(TOOLBAR_ITEMS.formatting).toContain("fontName");
    expect(new Set([...TOOLBAR_ITEMS.toolbar, ...TOOLBAR_ITEMS.formatting]).size).toBe(TOOLBAR_ITEMS.toolbar.length + TOOLBAR_ITEMS.formatting.length);
  });
});

describe("separators", () => {
  const kinds = (hidden: string[], sections = FORMATTING_SECTIONS) => slotsOf(visibleSections(sections, hidden)).map((s) => (s.kind === "sep" ? "|" : typeof s.el === "string" ? s.el : `[${s.el.join(" ")}]`));

  it("collapse when every button of a section is hidden; groups shrink", () => {
    expect(kinds([], TOOLBAR_SECTIONS)).toEqual(["save", "|", "undo", "redo", "|", "sort", "filter", "addUp", "chart"]);
    expect(kinds(["undo", "redo"], TOOLBAR_SECTIONS)).toEqual(["save", "|", "sort", "filter", "addUp", "chart"]);
    expect(kinds(["save"], TOOLBAR_SECTIONS)).toEqual(["undo", "redo", "|", "sort", "filter", "addUp", "chart"]);
    expect(kinds(["sort", "filter", "addUp", "chart"], TOOLBAR_SECTIONS)).toEqual(["save", "|", "undo", "redo"]);
    expect(kinds([...TOOLBAR_ITEMS.toolbar], TOOLBAR_SECTIONS)).toEqual([]);
    expect(kinds(["italic", "underline"]).slice(0, 5)).toEqual(["fontName", "fontSize", "|", "[bold strike]", "|"]);
    const noColours = kinds(["fontColor", "fillColor"]);
    expect(noColours.join(" ")).not.toContain("| |");
    expect(kinds(["currency", "percent"])).toContain("numberFormat");
  });
});

describe("overflow fit", () => {
  it("keeps what fits, reserves room for », never ends on a separator", () => {
    const w = [100, 1, 50, 50, 1, 80];
    const sep = [false, true, false, false, true, false];
    // Everything: 282 + 5 gaps of 6 = 312.
    expect(fitCount(w, sep, 312, 6, 30)).toBe(6);
    expect(fitCount(w, sep, 400, 6, 30)).toBe(6);
    // 311 px: the » takes 36, so 275 remain: 100+6+1+6+50+6+50 = 219, +6+1 = 226, +6+80 = 312 too much → 5, minus the trailing separator → 4.
    expect(fitCount(w, sep, 311, 6, 30)).toBe(4);
    expect(fitCount(w, sep, 120, 6, 30)).toBe(0);
    expect(fitCount([], [], 100, 6, 30)).toBe(0);
  });
});

describe("compact before overflow", () => {
  const sep = [false, true, false, false];
  it("keeps labels when everything fits, drops them before anything goes into »", () => {
    expect(chooseLayout([100, 9, 120, 140], [100, 9, 50, 40], sep, 400, 6, 30)).toEqual({ compact: false, n: 4 });
    // 387 + 18 > 380: compact (217 + 18) fits.
    expect(chooseLayout([100, 9, 120, 140], [100, 9, 50, 40], sep, 380, 6, 30)).toEqual({ compact: true, n: 4 });
    // Even compact does not fit: » with the compact widths.
    expect(chooseLayout([100, 9, 120, 140], [100, 9, 50, 40], sep, 210, 6, 30)).toEqual({ compact: true, n: 3 });
    // Compact widths not measured yet: draw compact to measure them.
    expect(chooseLayout([100, 9, 120, 140], [100, 9, null, null], sep, 380, 6, 30)).toEqual({ compact: true, n: 4 });
    // Full widths unknown: draw everything with labels.
    expect(chooseLayout([100, null, 120, 140], null, sep, 100, 6, 30)).toEqual({ compact: false, n: 4 });
    // No compactable slots: straight to ».
    expect(chooseLayout([100, 9, 120, 140], null, sep, 300, 6, 30)).toEqual({ compact: false, n: 3 });
  });
});

describe("persistence", () => {
  beforeEach(() => {
    localStorage.clear();
    useW56Store.getState().setPrefs(DEFAULT_PREFS);
  });

  it("View ▸ Toolbars and Visible Buttons store the prefs (browser: localStorage) and tell the other windows", async () => {
    const client = new WebMockClient();
    const ctl = new AppController(client, client, { nativeMenu: false });
    const broadcast = vi.spyOn(ctl, "broadcastPrefs").mockImplementation(() => undefined);
    await toggleBar(ctl, "formulaBar");
    expect(useW56Store.getState().prefs.chrome.bars.formulaBar).toBe(false);
    expect(readLocalPrefs().chrome.bars.formulaBar).toBe(false);
    await saveChrome(ctl, withItem(useW56Store.getState().prefs.chrome, "toolbar", "sort", false));
    expect(readLocalPrefs().chrome.hidden.toolbar).toEqual(["sort"]);
    expect(broadcast).toHaveBeenCalledTimes(2);
    // A new window reads them back.
    const other = new AppController(client, client, { nativeMenu: false });
    useW56Store.getState().setPrefs(DEFAULT_PREFS);
    await other.w56.loadPrefs();
    await tick();
    expect(useW56Store.getState().prefs.chrome.hidden.toolbar).toEqual(["sort"]);
    expect(useW56Store.getState().prefs.chrome.bars.formulaBar).toBe(false);
    openCustomize();
    expect(useW56Store.getState().dialog).toEqual({ kind: "customizeToolbars" });
    useW56Store.getState().openDialog(null);
  });

  it("the desktop app stores them in prefs.json", async () => {
    const client = new WebMockClient();
    let stored = "{}";
    Object.assign(client, { getPrefs: () => Promise.resolve(stored), setPrefs: (json: string) => ((stored = json), Promise.resolve()) });
    const ctl = new AppController(client, client, { nativeMenu: false });
    vi.spyOn(ctl, "broadcastPrefs").mockImplementation(() => undefined);
    await toggleBar(ctl, "statusBar");
    expect(JSON.parse(stored).chrome.bars.statusBar).toBe(false);
  });

  it("hiding the formula bar commits an edit typed in it first", async () => {
    const client = new WebMockClient();
    const ctl = new AppController(client, client, { nativeMenu: false });
    vi.spyOn(ctl, "broadcastPrefs").mockImplementation(() => undefined);
    await ctl.start("invoices");
    await tick();
    useEditStore.getState().start({ r: 20, c: 1 }, "12345", "bar");
    await toggleBar(ctl, "formulaBar");
    expect(useEditStore.getState().edit).toBeNull();
    const [b] = await client.cells(ctl.info!.doc, ctl.sheet, [{ r0: 20, c0: 1, r1: 20, c1: 1 }]);
    expect(b?.cells[0]?.text).toBe("12345");
    expect(useW56Store.getState().prefs.chrome.bars.formulaBar).toBe(false);
    // The peek is cleared when the bar is hidden on purpose.
    usePeekStore.getState().setFormulaBar(true);
    await toggleBar(ctl, "formulaBar");
    await toggleBar(ctl, "formulaBar");
    expect(usePeekStore.getState().formulaBar).toBe(false);
  });
});
