// @vitest-environment happy-dom
/**
 * Document tabs (tabs.ts, components/TabStrip.tsx, desktop `windows.rs`):
 * the pure helpers, the strip (render, close, middle-click, reorder,
 * keyboard, context menu → the tab shell commands), the controller wiring
 * (`window_init` / `tabs-changed` fill the store, `place_document` answering
 * `"tab"`, the Window commands), the `TauriClient` side, the
 * "Open documents in" pref and the platform-specific tab keys.
 */
import type { WorkbookInfo } from "@tally/bridge";
import { COMMANDS, EVENTS, TAB_SHELL_COMMANDS, TauriClient, type InvokeArgs, type TabsState, type TauriEvent } from "@tally/bridge/tauri";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { buildCommands } from "../src/commands";
import { OpenDocuments } from "../src/components/OpenDocuments";
import { ControllerContext } from "../src/context";
import { AppController } from "../src/controller";
import { WebMockClient } from "../src/dev/webMock";
import { installMockTabs, mockTabsState } from "../src/dev/mockTabs";
import { BINDINGS, platformBindings, resolveKey, shortcutFor, type KeyLike } from "../src/keymap";
import { FormulaBar } from "../src/components/FormulaBar";
import { renderShortcutsDoc } from "../src/options/shortcutsDoc";
import { DEFAULT_PREFS, parsePrefs } from "../src/options/prefs";
import { useDocStore } from "../src/stores/docStore";
import { canDetach, cycleTab, editedSince, moveIndex, openDocuments, tabByNumber, tabTooltip, useTabsStore } from "../src/tabs";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const tick = () => new Promise((r) => setTimeout(r, 0));

function state(n: number, active = "doc-1"): TabsState {
  return { ...mockTabsState(n), active };
}

describe("tab helpers (pure)", () => {
  it("lists this window's tabs, then the other windows' documents", () => {
    expect(openDocuments(null)).toEqual([]);
    expect(openDocuments(state(2)).map((d) => d.label)).toEqual(["doc-1", "doc-2", "doc-21"]);
  });

  it("tracks since when each document is edited", () => {
    const docs = (edited: string[]) => ["doc-1", "doc-2"].map((label) => ({ label, window: "win-1", title: "", edited: edited.includes(label), path: null }));
    let since = editedSince({}, docs(["doc-1"]), 100);
    expect(since).toEqual({ "doc-1": 100 });
    since = editedSince(since, docs(["doc-1", "doc-2"]), 200);
    expect(since).toEqual({ "doc-1": 100, "doc-2": 200 });
    // Saved: dropped; edited again later starts anew.
    since = editedSince(since, docs(["doc-2"]), 300);
    expect(since).toEqual({ "doc-2": 200 });
    expect(editedSince(since, docs(["doc-1", "doc-2"]), 400)).toEqual({ "doc-1": 400, "doc-2": 200 });
  });

  it("cycles with wrap-around", () => {
    expect(cycleTab(state(3), 1)).toBe("doc-2");
    expect(cycleTab(state(3), -1)).toBe("doc-3");
    expect(cycleTab(state(3, "doc-3"), 1)).toBe("doc-1");
    expect(cycleTab(state(1), 1)).toBeNull();
    expect(cycleTab(null, 1)).toBeNull();
  });

  it("numbers tabs 1–8 and 9 = the last one", () => {
    const s = state(4);
    expect(tabByNumber(s, 1)).toBe("doc-1");
    expect(tabByNumber(s, 4)).toBe("doc-4");
    expect(tabByNumber(s, 5)).toBeNull();
    expect(tabByNumber(s, 9)).toBe("doc-4");
    expect(tabByNumber(state(12), 9)).toBe("doc-12");
    expect(tabByNumber(s, 0)).toBeNull();
  });

  it("detaches only with a tab left behind; drop indexes skip the dragged tab", () => {
    expect(canDetach(state(1))).toBe(false);
    expect(canDetach(state(2))).toBe(true);
    const s = state(4);
    expect(moveIndex(s, "doc-1", "doc-3")).toBe(1);
    expect(moveIndex(s, "doc-4", "doc-1")).toBe(0);
    expect(moveIndex(s, "doc-2", null)).toBe(3);
    expect(moveIndex(s, "doc-2", "doc-3")).toBeNull();
    expect(moveIndex(s, "doc-2", "doc-2")).toBeNull();
    expect(moveIndex(s, "doc-9", null)).toBeNull();
  });

  it("tooltips name the file, else the title", () => {
    expect(tabTooltip({ label: "doc-1", window: "win-1", title: "", edited: false, path: null }, "Untitled")).toBe("Untitled");
    expect(tabTooltip({ label: "doc-2", window: "win-1", title: "A.ods", edited: false, path: "/x/A.ods" }, "Untitled")).toBe("/x/A.ods");
  });
});

/** A controller over the browser mock with a fake tab shell recording its calls. */
function tabController() {
  const client = new WebMockClient();
  const calls: [string, ...unknown[]][] = [];
  const rec =
    (name: string) =>
    (...args: unknown[]): Promise<void> => {
      calls.push([name, ...args]);
      return Promise.resolve();
    };
  Object.assign(client, {
    tabActivate: rec("tabActivate"),
    tabClose: rec("tabClose"),
    tabCloseOthers: rec("tabCloseOthers"),
    tabMove: rec("tabMove"),
    tabDetach: rec("tabDetach"),
    mergeWindows: rec("mergeWindows"),
  });
  const ctl = new AppController(client, client, { nativeMenu: true });
  return { client, ctl, calls };
}

describe("sidebar ▸ Open (OpenDocuments)", () => {
  let host: HTMLDivElement;
  let root: Root;
  const props = { dotFor: () => "#2563eb", formatTag: (p: string) => /\.(ods|xlsx|csv)$/i.exec(p)?.[1]?.toUpperCase() ?? "" };

  beforeEach(() => {
    useTabsStore.getState().reset();
    useDocStore.getState().reset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  function mount(ctl: AppController, s: TabsState | null, self = "doc-1") {
    act(() => {
      useTabsStore.getState().setSelf(self);
      useTabsStore.getState().setState(s);
    });
    act(() => root.render(createElement(ControllerContext.Provider, { value: ctl }, createElement(OpenDocuments, props))));
  }

  const row = (label: string) => host.querySelector<HTMLElement>(`[data-testid="open-doc-${label}"]`)!;
  const rows = () => [...host.querySelectorAll<HTMLElement>('[role="tab"]')];

  it("without a tab shell shows this document as the one row", () => {
    const ctl = new AppController(new WebMockClient(), new WebMockClient(), { nativeMenu: false });
    act(() => useDocStore.getState().setInfo({ doc: 1, title: "Untitled 1", path: null, sheets: [], activeSheet: 0, dirty: false, version: 0, warnings: [], externalLinks: null } as unknown as WorkbookInfo));
    mount(ctl, null, "");
    expect(host.querySelector("h3")?.textContent).toBe("Editing");
    expect(rows().map((r) => r.querySelector("span:nth-child(2)")?.textContent)).toEqual(["Untitled 1"]);
    expect(rows()[0]?.getAttribute("aria-selected")).toBe("true");
    expect(host.querySelector('[data-testid^="open-doc-close-"]')).toBeNull();
  });

  it("lists this window's tabs first, then the other windows'; highlights this document; edited dot; tooltips", () => {
    const { ctl } = tabController();
    mount(ctl, state(3, "doc-2"), "doc-2");
    expect(host.querySelector('[role="tablist"]')?.getAttribute("aria-label")).toBe("Open documents");
    expect(rows().map((r) => r.dataset.testid)).toEqual(["open-doc-doc-1", "open-doc-doc-2", "open-doc-doc-3", "open-doc-doc-21"]);
    expect(rows().map((r) => r.querySelector("span:nth-child(2)")?.textContent)).toEqual(["Untitled", "Book 2.ods", "Book 3.ods", "Report.xlsx"]);
    expect(rows().map((r) => r.getAttribute("aria-selected"))).toEqual(["false", "true", "false", "false"]);
    expect(rows().map((r) => r.getAttribute("tabindex"))).toEqual(["-1", "0", "-1", "-1"]);
    expect(row("doc-21").dataset.window).toBe("win-2");
    expect(row("doc-2").dataset.edited).toBe("true");
    // The meta shows only the time; the dot and the tooltip say "Edited".
    expect(row("doc-2").querySelector("span:nth-child(4)")?.textContent).toBe("just now");
    expect(row("doc-2").textContent).not.toContain("Edited");
    expect(row("doc-2").querySelector('[role="img"]')?.getAttribute("aria-label")).toBe("Edited · just now");
    expect(row("doc-2").title).toBe("/Users/me/Documents/Book 2.ods\nEdited · just now");
    expect(row("doc-3").textContent).toContain("ODS");
    expect(row("doc-1").title).toBe("Untitled");
    expect(row("doc-21").title).toBe("/Users/me/Documents/Report.xlsx");
    // Draggable only within this window.
    expect(row("doc-1").getAttribute("draggable")).toBe("true");
    expect(row("doc-21").getAttribute("draggable")).toBe("false");
  });

  it("click shows a document (also another window's), × closes it", async () => {
    const { ctl, calls } = tabController();
    const focus = vi.spyOn(ctl, "focusGrid").mockImplementation(() => undefined);
    mount(ctl, state(3));
    act(() => row("doc-3").click());
    act(() => row("doc-21").click());
    // This document's own row only focuses the grid.
    act(() => row("doc-1").click());
    act(() => host.querySelector<HTMLElement>('[data-testid="open-doc-close-doc-2"]')!.click());
    act(() => host.querySelector<HTMLElement>('[data-testid="open-doc-close-doc-21"]')!.click());
    await tick();
    expect(calls).toEqual([
      ["tabActivate", "doc-3"],
      ["tabActivate", "doc-21"],
      ["tabClose", "doc-2"],
      ["tabClose", "doc-21"],
    ]);
    expect(focus).toHaveBeenCalledTimes(1);
  });

  it("keyboard: ↑/↓/Home/End move the focus, Enter shows, Delete closes", () => {
    const { ctl, calls } = tabController();
    mount(ctl, state(3));
    const key = (label: string, k: string) => act(() => row(label).dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true })));
    key("doc-1", "ArrowDown");
    expect(document.activeElement).toBe(row("doc-2"));
    expect(row("doc-2").getAttribute("tabindex")).toBe("0");
    key("doc-2", "End");
    expect(document.activeElement).toBe(row("doc-21"));
    key("doc-21", "ArrowDown");
    expect(document.activeElement).toBe(row("doc-21"));
    key("doc-21", "ArrowUp");
    expect(document.activeElement).toBe(row("doc-3"));
    key("doc-3", "Home");
    expect(document.activeElement).toBe(row("doc-1"));
    key("doc-21", "Enter");
    key("doc-3", "Delete");
    key("doc-2", "Backspace");
    expect(calls).toEqual([
      ["tabActivate", "doc-21"],
      ["tabClose", "doc-3"],
      ["tabClose", "doc-2"],
    ]);
    // Nothing focusable inside a tab (the × is pointer-only and hidden from AT).
    for (const label of ["doc-1", "doc-2", "doc-3", "doc-21"]) {
      expect(row(label).querySelector('button, a[href], input, [tabindex]')).toBeNull();
      expect(row(label).querySelector('[data-testid^="open-doc-close-"]')?.getAttribute("aria-hidden")).toBe("true");
    }
  });

  it("drag reorders this window's tabs only (tab_move with the index in this window)", () => {
    const { ctl, calls } = tabController();
    mount(ctl, state(3));
    const dt = { effectAllowed: "", setData: () => undefined };
    const drag = (el: HTMLElement, type: string, y = 0) => {
      const e = new Event(type, { bubbles: true, cancelable: true }) as Event;
      Object.assign(e, { dataTransfer: dt, clientY: y });
      act(() => el.dispatchEvent(e));
    };
    // doc-1 dropped on the upper half of doc-3: before it, i.e. index 1 once doc-1 is out.
    drag(row("doc-1"), "dragstart");
    drag(row("doc-3"), "dragover", -1);
    drag(row("doc-3"), "drop", -1);
    // Onto another window's row: no drop target.
    drag(row("doc-2"), "dragstart");
    drag(row("doc-21"), "dragover", -1);
    drag(row("doc-21"), "drop", -1);
    drag(row("doc-2"), "dragend");
    expect(calls).toEqual([["tabMove", "doc-1", 1]]);
  });

  it("context menu: Close, Close Others, Move to New Window; the last two only for this window's rows", () => {
    const { ctl, calls } = tabController();
    mount(ctl, state(3));
    const open = (label: string) => {
      act(() => row(label).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 10 })));
      return document.querySelector('[data-testid="open-doc-menu"]')!;
    };
    for (const item of ["close", "closeOthers", "detach"]) {
      const menu = open("doc-2");
      expect([...menu.querySelectorAll("[data-item]")].map((b) => b.textContent)).toEqual(["Close", "Close Others", "Move to New Window"]);
      act(() => menu.querySelector<HTMLElement>(`[data-item="${item}"]`)!.click());
      expect(document.querySelector('[data-testid="open-doc-menu"]')).toBeNull();
    }
    const other = open("doc-21");
    expect([...other.querySelectorAll("[data-item]")].map((b) => b.getAttribute("aria-disabled"))).toEqual([null, "true", "true"]);
    act(() => other.querySelector<HTMLElement>('[data-item="detach"]')!.click());
    act(() => other.querySelector<HTMLElement>('[data-item="close"]')?.click());
    expect(calls).toEqual([
      ["tabClose", "doc-2"],
      ["tabCloseOthers", "doc-2"],
      ["tabDetach", "doc-2"],
      ["tabClose", "doc-21"],
    ]);
    // One tab here: Move to New Window is off for it too.
    act(() => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    mount(ctl, { ...state(1), others: state(3).others });
    const single = open("doc-1");
    expect(single.querySelector('[data-item="detach"]')?.getAttribute("aria-disabled")).toBe("true");
  });
});

describe("a question pending across a reload (review fix)", () => {
  beforeEach(() => {
    useTabsStore.getState().reset();
    useDocStore.getState().reset();
  });

  it("window_init's ask re-runs the close question", async () => {
    const { client, ctl } = tabController();
    Object.assign(client, {
      windowInit: () => Promise.resolve({ doc: null, offerRecovery: false, label: "doc-2", tabs: state(2), ask: "tabs" }),
    });
    const asked = vi.spyOn(ctl, "closeRequested").mockResolvedValue(true);
    await ctl.start(null);
    expect(asked).toHaveBeenCalledWith("tabs");
    asked.mockClear();
    Object.assign(client, {
      windowInit: () => Promise.resolve({ doc: null, offerRecovery: false, label: "doc-2", tabs: state(2), ask: null }),
    });
    await ctl.start(null);
    expect(asked).not.toHaveBeenCalled();
  });
});

describe("controller wiring", () => {
  beforeEach(() => {
    useTabsStore.getState().reset();
    useDocStore.getState().reset();
  });

  it("window_init and tabs-changed fill the store; becoming the visible tab focuses the grid", async () => {
    const { client, ctl } = tabController();
    let tabsCb: ((s: TabsState) => void) | null = null;
    Object.assign(client, {
      windowInit: () => Promise.resolve({ doc: null, offerRecovery: false, label: "doc-2", tabs: state(2) }),
      placeDocument: () => Promise.resolve("tab"),
      onTabsChanged: (cb: (s: TabsState) => void) => {
        tabsCb = cb;
        return () => {
          tabsCb = null;
        };
      },
    });
    const focus = vi.spyOn(ctl, "focusGrid");
    const detach = ctl.attach();
    await ctl.start(null);
    expect(useTabsStore.getState().self).toBe("doc-2");
    expect(useTabsStore.getState().state).toEqual(state(2));
    expect(focus).not.toHaveBeenCalled();
    tabsCb!(state(3, "doc-2"));
    expect(useTabsStore.getState().state?.tabs).toHaveLength(3);
    expect(focus).toHaveBeenCalledTimes(1);
    // Still active: no second focus.
    tabsCb!(state(4, "doc-2"));
    expect(focus).toHaveBeenCalledTimes(1);
    detach();
    expect(tabsCb).toBeNull();
  });

  it("a document placed as a new tab is not shown here (open warnings become toasts)", async () => {
    const { client, ctl } = tabController();
    Object.assign(client, { placeDocument: () => Promise.resolve("tab" as const) });
    await ctl.start(null);
    const before = ctl.info?.doc;
    const info: WorkbookInfo = { ...(await client.newWorkbook("budget")), warnings: ["w"] };
    await ctl.present(info);
    expect(ctl.info?.doc).toBe(before);
  });

  it("the Window commands call the tab shell (selectTab9 = the last tab)", async () => {
    const { ctl, calls } = tabController();
    const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };
    ctl.registry.registerAll(buildCommands(ctl, ui));
    useTabsStore.getState().setSelf("doc-1");
    useTabsStore.getState().setState(state(4));
    for (const id of ["window.nextTab", "window.prevTab", "window.selectTab9", "window.selectTab2", "window.selectTab1", "window.moveTabToNewWindow", "window.mergeAllWindows"]) {
      expect(ctl.registry.run(id)).toBe(true);
    }
    await tick();
    expect(calls).toEqual([
      ["tabActivate", "doc-2"],
      ["tabActivate", "doc-4"],
      ["tabActivate", "doc-4"],
      ["tabActivate", "doc-2"],
      ["tabDetach", "doc-1"],
      ["mergeWindows"],
    ]);
  });

  it("without a tab shell the Window commands are disabled", () => {
    const ctl = new AppController(new WebMockClient(), new WebMockClient(), { nativeMenu: false });
    const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };
    ctl.registry.registerAll(buildCommands(ctl as unknown as AppController, ui));
    expect(ctl.tabs.available).toBe(false);
    expect(ctl.registry.isEnabled("window.nextTab")).toBe(false);
    expect(ctl.registry.run("window.mergeAllWindows")).toBe(false);
  });

  it("the dev mock (?tabs=n) drives the store", async () => {
    const client = new WebMockClient();
    installMockTabs(client, 3);
    const ctl = new AppController(client, client, { nativeMenu: false });
    await ctl.tabs.activate("doc-3");
    expect(useTabsStore.getState().state?.active).toBe("doc-3");
    await ctl.tabs.move("doc-3", 0);
    expect(useTabsStore.getState().state?.tabs.map((t) => t.label)).toEqual(["doc-3", "doc-1", "doc-2"]);
    await ctl.tabs.close("doc-3");
    expect(useTabsStore.getState().state).toMatchObject({ active: "doc-1", tabs: [{ label: "doc-1" }, { label: "doc-2" }] });
  });
});

describe("TauriClient tab commands", () => {
  it("invokes the windows.rs commands with camelCase arguments and listens to tabs-changed", async () => {
    const calls: { cmd: string; args?: InvokeArgs }[] = [];
    const handlers = new Map<string, (e: TauriEvent<unknown>) => void>();
    const client = new TauriClient({
      invoke: <T,>(cmd: string, args?: InvokeArgs) => {
        calls.push({ cmd, args });
        return Promise.resolve(null as T);
      },
      listen: <T,>(event: string, h: (e: TauriEvent<T>) => void) => {
        handlers.set(event, h as (e: TauriEvent<unknown>) => void);
        return Promise.resolve(() => undefined);
      },
    });
    await client.tabActivate("doc-2");
    await client.tabClose("doc-3");
    await client.tabCloseOthers("doc-1");
    await client.tabMove("doc-1", 2);
    await client.tabDetach("doc-4");
    await client.mergeWindows();
    expect(calls).toEqual([
      { cmd: "tab_activate", args: { label: "doc-2" } },
      { cmd: "tab_close", args: { label: "doc-3" } },
      { cmd: "tab_close_others", args: { label: "doc-1" } },
      { cmd: "tab_move", args: { label: "doc-1", index: 2 } },
      { cmd: "tab_detach", args: { label: "doc-4" } },
      { cmd: "merge_windows", args: undefined },
    ]);
    expect(TAB_SHELL_COMMANDS.map((m) => COMMANDS[m])).toEqual(calls.map((c) => c.cmd));
    const got: TabsState[] = [];
    client.onTabsChanged((s) => got.push(s));
    await tick();
    handlers.get(EVENTS.tabsChanged)?.({ payload: state(2) } as TauriEvent<unknown>);
    expect(got).toEqual([state(2)]);
    expect(EVENTS.tabsChanged).toBe("tabs-changed");
    const kinds: string[] = [];
    client.onCloseRequested((k) => kinds.push(k));
    await tick();
    handlers.get(EVENTS.closeRequested)?.({ payload: { kind: "tabs" } } as TauriEvent<unknown>);
    expect(kinds).toEqual(["tabs"]);
  });
});

describe("Open documents in (prefs)", () => {
  it("defaults to tabs and accepts only tabs/windows", () => {
    expect(DEFAULT_PREFS.openDocumentsIn).toBe("tabs");
    expect(parsePrefs({}).openDocumentsIn).toBe("tabs");
    expect(parsePrefs({ openDocumentsIn: "windows" }).openDocumentsIn).toBe("windows");
    expect(parsePrefs({ openDocumentsIn: "popups" }).openDocumentsIn).toBe("tabs");
    expect(parsePrefs({ openDocumentsIn: 1 }).openDocumentsIn).toBe("tabs");
  });
});

describe("tab keys (platform-specific)", () => {
  const ev = (k: Partial<KeyLike>): KeyLike => ({ key: "", metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...k });
  const mac = { typing: false, nativeMenu: true, mac: true };
  const win = { typing: false, nativeMenu: true, mac: false };

  it("⌥⌘N on macOS, Alt+N on Windows/Linux; ⌘1 stays Format Cells", () => {
    expect(resolveKey(ev({ key: "¡", code: "Digit1", metaKey: true, altKey: true }), mac)).toBe("window.selectTab1");
    expect(resolveKey(ev({ key: "9", code: "Digit9", altKey: true }), win)).toBe("window.selectTab9");
    // Not on the other platform: Alt+N on macOS types characters; Ctrl+Alt is AltGr on Windows.
    expect(resolveKey(ev({ key: "¡", code: "Digit1", altKey: true }), mac)).toBeNull();
    expect(resolveKey(ev({ key: "{", code: "Digit7", ctrlKey: true, altKey: true }), win)).toBeNull();
    expect(resolveKey(ev({ key: "1", code: "Digit1", metaKey: true }), { ...mac, nativeMenu: false })).toBe("format.cells");
    // Alt+N is not taken while typing; ⌥⌘N is.
    expect(resolveKey(ev({ key: "2", code: "Digit2", altKey: true }), { ...win, typing: true })).toBeNull();
    expect(resolveKey(ev({ key: "™", code: "Digit2", metaKey: true, altKey: true }), { ...mac, typing: true })).toBe("window.selectTab2");
  });

  it("⇧⌘] / ⇧⌘[ on macOS; ⌃⇥ belongs to the native menu, the webview takes it without one", () => {
    expect(resolveKey(ev({ key: "}", code: "BracketRight", metaKey: true, shiftKey: true }), mac)).toBe("window.nextTab");
    expect(resolveKey(ev({ key: "{", code: "BracketLeft", metaKey: true, shiftKey: true }), mac)).toBe("window.prevTab");
    expect(resolveKey(ev({ key: "}", code: "BracketRight", ctrlKey: true, shiftKey: true }), win)).toBeNull();
    expect(resolveKey(ev({ key: "Tab", ctrlKey: true }), mac)).toBeNull();
    expect(resolveKey(ev({ key: "Tab", ctrlKey: true }), { ...win, nativeMenu: false })).toBe("window.nextTab");
    expect(resolveKey(ev({ key: "Tab", ctrlKey: true, shiftKey: true }), { ...win, nativeMenu: false })).toBe("window.prevTab");
    // ⌃PgDn stays Next Sheet.
    expect(resolveKey(ev({ key: "PageDown", ctrlKey: true }), win)).toBe("sheet.next");
  });
});

describe("tab keys: code-only digits and platform labels (review fixes)", () => {
  const ev = (k: Partial<KeyLike>): KeyLike => ({ key: "", metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...k });
  const win = { typing: false, nativeMenu: true, mac: false };
  const mac = { typing: false, nativeMenu: true, mac: true };

  it("Alt-digit tab keys match on the top-row code only, never on key", () => {
    // Odd `key` values (Windows Alt combinations, ⌥ on macOS) still match by code…
    expect(resolveKey(ev({ key: "Unidentified", code: "Digit3", altKey: true }), win)).toBe("window.selectTab3");
    expect(resolveKey(ev({ key: "£", code: "Digit3", metaKey: true, altKey: true }), mac)).toBe("window.selectTab3");
    // …and a digit `key` from another physical key never does (numpad: Windows Alt-codes).
    expect(resolveKey(ev({ key: "3", code: "Numpad3", altKey: true }), win)).toBeNull();
    expect(resolveKey(ev({ key: "3", altKey: true }), win)).toBeNull();
  });

  it("labels follow the platform: Windows/Linux never see ⌥⌘", () => {
    expect(shortcutFor("window.selectTab1", BINDINGS, false)).toBe("Alt+1");
    expect(shortcutFor("window.selectTab1", BINDINGS, true)).toBe("⌥⌘1");
    expect(shortcutFor("window.nextTab", BINDINGS, false)).toBe("⌃⇥");
    expect(shortcutFor("format.bold", BINDINGS, false)).toBe("⌘B");
    const other = platformBindings(false).map((b) => b.label);
    expect(other).toContain("Alt+9");
    expect(other.some((l) => l.startsWith("⌥⌘") && /\d$/.test(l))).toBe(false);
    expect(other).not.toContain("⇧⌘]");
    expect(platformBindings(true).map((b) => b.label)).not.toContain("Alt+1");
    // The generated (cross-platform) doc names the platform of platform-only keys.
    const doc = renderShortcutsDoc(BINDINGS, [], (id) => id);
    expect(doc).toContain("| ⌥⌘1 (macOS) / Alt+1 (Windows/Linux) | window.selectTab1 |");
  });
});

describe("FormulaBar leaves ⌃Tab to the native menu (review fix)", () => {
  it("⌃Tab / ⌘Tab neither commit nor are prevented; Tab still commits", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    const client = new WebMockClient();
    const ctl = new AppController(client, client, { nativeMenu: true });
    await ctl.start(null);
    const commit = vi.spyOn(ctl, "commitEdit").mockResolvedValue(true);
    act(() => root.render(createElement(ControllerContext.Provider, { value: ctl }, createElement(FormulaBar))));
    const input = host.querySelector<HTMLInputElement>('[data-testid="formula-input"]')!;
    for (const init of [{ ctrlKey: true }, { ctrlKey: true, shiftKey: true }, { metaKey: true }]) {
      const e = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true, ...init });
      act(() => input.dispatchEvent(e));
      expect(e.defaultPrevented).toBe(false);
    }
    expect(commit).not.toHaveBeenCalled();
    const plain = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    act(() => input.dispatchEvent(plain));
    expect(plain.defaultPrevented).toBe(true);
    expect(commit).toHaveBeenCalledWith("right");
    act(() => root.unmount());
    host.remove();
  });

  it("useKeymap's ⌃Tab binding yields to the native menu", () => {
    expect(resolveKey({ key: "Tab", ctrlKey: true, metaKey: false, shiftKey: false, altKey: false }, { typing: true, nativeMenu: true, mac: true })).toBeNull();
    expect(resolveKey({ key: "Tab", ctrlKey: true, metaKey: false, shiftKey: true, altKey: false }, { typing: false, nativeMenu: true, mac: false })).toBeNull();
  });
});
