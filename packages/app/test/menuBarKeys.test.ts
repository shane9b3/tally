// @vitest-environment happy-dom
/**
 * Keyboard and clipboard details of the Windows in-app menu bar, where the
 * webview's keymap owns every shortcut: AltGr characters are text, the
 * reload keys never reach the webview, the OS's clipboard items act on a
 * focused text field, and the system's Alt+F4 / Alt+Space pass through.
 */
import type { MenuNode } from "@tally/bridge";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MenuBar, ariaKeyShortcuts } from "../src/chrome/MenuBar";
import { NATIVE_MENU_COMMANDS, runTextCommand, textCommandOf, textTarget, type TextClipboard } from "../src/chrome/textCommands";
import type { AppController } from "../src/controller";
import fixture from "../src/dev/menuModel.json";
import { isReloadKey, resolveKey, type KeyLike } from "../src/keymap";
import { useKeymap } from "../src/useKeymap";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const MODEL = fixture as MenuNode[];
const k = (key: string, code: string, mods: Partial<KeyLike> = {}): KeyLike => ({ key, code, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...mods });
const WIN = { typing: false, nativeMenu: false, mac: false };

describe("AltGr never runs a command (Windows reports it as Ctrl+Alt)", () => {
  it("Ctrl+Alt shortcuts need the binding's own letter", () => {
    const ctrlAlt = { ctrlKey: true, altKey: true };
    expect(resolveKey(k("f", "KeyF", ctrlAlt), WIN)).toBe("edit.findReplace");
    expect(resolveKey(k("v", "KeyV", ctrlAlt), WIN)).toBe("edit.pasteSpecial");
    expect(resolveKey(k("k", "KeyK", ctrlAlt), WIN)).toBe("insert.hyperlink");
    expect(resolveKey(k("c", "KeyC", ctrlAlt), WIN)).toBe("insert.note");
    // Czech / Slovak / Hungarian layouts: AltGr+F is "[", AltGr+V is "@", AltGr+K is "ł", AltGr+C is "&".
    for (const [key, code] of [
      ["[", "KeyF"],
      ["@", "KeyV"],
      ["ł", "KeyK"],
      ["&", "KeyC"],
      ["đ", "KeyF"],
    ] as const) {
      expect(resolveKey(k(key, code, ctrlAlt), WIN), key).toBeNull();
      expect(resolveKey(k(key, code, ctrlAlt), { ...WIN, typing: true }), key).toBeNull();
    }
    // The browser says AltGr is down: never a command, whatever the key reads.
    expect(resolveKey(k("f", "KeyF", { ...ctrlAlt, altGraph: true }), WIN)).toBeNull();
    expect(resolveKey(k("1", "Digit1", { altKey: true, altGraph: true }), WIN)).toBeNull();
  });

  it("the other bindings are unchanged", () => {
    // macOS: ⌥ changes the character; the binding still matches by code.
    expect(resolveKey(k("ƒ", "KeyF", { metaKey: true, altKey: true }), { ...WIN, mac: true })).toBe("edit.findReplace");
    expect(resolveKey(k("√", "KeyV", { metaKey: true, altKey: true }), { ...WIN, mac: true })).toBe("edit.pasteSpecial");
    // Alt+digit tab shortcuts match by code only.
    expect(resolveKey(k("1", "Digit1", { altKey: true }), WIN)).toBe("window.selectTab1");
    expect(resolveKey(k("s", "KeyS", { ctrlKey: true }), WIN)).toBe("file.save");
    expect(resolveKey(k("w", "KeyW", { ctrlKey: true }), WIN)).toBe("file.close");
  });
});

describe("reload keys", () => {
  it("isReloadKey: F5, Ctrl+F5, Ctrl+R, Ctrl+Shift+R, ⌘R", () => {
    expect(isReloadKey(k("F5", "F5"))).toBe(true);
    expect(isReloadKey(k("F5", "F5", { ctrlKey: true }))).toBe(true);
    expect(isReloadKey(k("r", "KeyR", { ctrlKey: true }))).toBe(true);
    expect(isReloadKey(k("R", "KeyR", { ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(isReloadKey(k("r", "KeyR", { metaKey: true }))).toBe(true);
    expect(isReloadKey(k("r", "KeyR"))).toBe(false);
    expect(isReloadKey(k("r", "KeyR", { ctrlKey: true, altKey: true }))).toBe(false);
    expect(isReloadKey(k("F6", "F6"))).toBe(false);
  });

  describe("useKeymap", () => {
    let host: HTMLElement;
    let root: Root;
    let ran: string[];
    const mount = (nativeMenu: boolean, desktop: boolean) => {
      const ctl = { registry: { run: (id: string) => void ran.push(id) } } as unknown as AppController;
      function Probe() {
        useKeymap(ctl, nativeMenu, desktop);
        return createElement("input", { id: "field" });
      }
      act(() => root.render(createElement(Probe)));
    };
    const press = (target: Element, init: KeyboardEventInit, altGraph = false): boolean => {
      const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
      // happy-dom answers "AltGraph" with altKey; a browser only for the AltGr key.
      Object.defineProperty(e, "getModifierState", { value: (key: string) => key === "AltGraph" && altGraph });
      target.dispatchEvent(e);
      return e.defaultPrevented;
    };
    beforeEach(() => {
      ran = [];
      host = document.createElement("div");
      document.body.appendChild(host);
      root = createRoot(host);
    });
    afterEach(() => {
      act(() => root.unmount());
      document.body.innerHTML = "";
    });

    it("the desktop app swallows them while typing, without running the sheet command", () => {
      mount(false, true);
      const field = host.querySelector("#field")!;
      for (const init of [{ key: "F5", code: "F5" }, { key: "r", code: "KeyR", ctrlKey: true }, { key: "R", code: "KeyR", ctrlKey: true, shiftKey: true }, { key: "F5", code: "F5", ctrlKey: true }]) {
        expect(press(field, init), JSON.stringify(init)).toBe(true);
      }
      expect(ran).toEqual([]);
      // Other keys in a text field are the field's.
      expect(press(field, { key: "b", code: "KeyB", ctrlKey: true })).toBe(false);
      // On the sheet F5 and Ctrl+R are commands, run once.
      expect(press(document.body, { key: "F5", code: "F5" })).toBe(true);
      expect(press(document.body, { key: "r", code: "KeyR", ctrlKey: true })).toBe(true);
      expect(ran).toEqual(["view.navigator", "edit.fillRight"]);
    });

    it("with a native menu the keys are still kept from the webview", () => {
      mount(true, true);
      expect(press(document.body, { key: "F5", code: "F5" })).toBe(true);
      expect(press(host.querySelector("#field")!, { key: "r", code: "KeyR", metaKey: true })).toBe(true);
      expect(ran).toEqual([]);
    });

    it("the browser build leaves them alone", () => {
      mount(false, false);
      expect(press(host.querySelector("#field")!, { key: "F5", code: "F5" })).toBe(false);
      expect(press(host.querySelector("#field")!, { key: "r", code: "KeyR", ctrlKey: true })).toBe(false);
      expect(ran).toEqual([]);
    });

    it("AltGr characters typed in a field are not shortcuts", () => {
      mount(false, true);
      const field = host.querySelector("#field")!;
      expect(press(field, { key: "[", code: "KeyF", ctrlKey: true, altKey: true })).toBe(false);
      expect(press(document.body, { key: "@", code: "KeyV", ctrlKey: true, altKey: true })).toBe(false);
      expect(ran).toEqual([]);
      expect(press(document.body, { key: "f", code: "KeyF", ctrlKey: true, altKey: true })).toBe(true);
      expect(ran).toEqual(["edit.findReplace"]);
      // The AltGr key itself, on a layout where it leaves the letter alone.
      expect(press(document.body, { key: "f", code: "KeyF", ctrlKey: true, altKey: true }, true)).toBe(false);
      expect(ran).toEqual(["edit.findReplace"]);
    });
  });
});

describe("clipboard items on a text field", () => {
  let clip: { text: string | null; written: string[]; unreadable: number };
  const clipboard = (): TextClipboard => ({
    readText: () => Promise.resolve(clip.text),
    writeText: (t) => {
      clip.written.push(t);
      clip.text = t;
      return Promise.resolve();
    },
    unreadable: () => void clip.unreadable++,
  });
  const flush = async () => {
    for (let i = 0; i < 4; i++) await Promise.resolve();
  };
  const field = (value: string, tag: "input" | "textarea" = "input", parentClass?: string) => {
    const parent = document.createElement("div");
    if (parentClass) parent.className = parentClass;
    const el = document.createElement(tag);
    el.value = value;
    parent.appendChild(el);
    document.body.appendChild(parent);
    el.focus();
    return el;
  };
  beforeEach(() => {
    clip = { text: null, written: [], unreadable: 0 };
  });
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("textTarget: text fields, and the grid's textarea only during a cell edit", () => {
    expect(Object.keys(NATIVE_MENU_COMMANDS).map(textCommandOf)).toEqual(["cut", "copy", "paste", "selectAll"]);
    expect(textCommandOf("edit.copy")).toBeUndefined();
    const input = field("x");
    expect(textTarget(false)).toBe(input);
    const area = field("x", "textarea");
    expect(textTarget(false)).toBe(area);
    const grid = field("", "textarea", "tally-grid");
    expect(textTarget(false)).toBeNull();
    expect(textTarget(true)).toBe(grid);
    expect(textTarget(true, input)).toBe(input);
    const box = document.createElement("input");
    box.type = "checkbox";
    document.body.appendChild(box);
    expect(textTarget(false, box)).toBeNull();
    expect(textTarget(false, document.body)).toBeNull();
    expect(textTarget(false, null)).toBeNull();
  });

  it("Select All selects the field's text", () => {
    const el = field("hello world");
    el.setSelectionRange(2, 4);
    runTextCommand(el, "selectAll", clipboard());
    expect([el.selectionStart, el.selectionEnd]).toEqual([0, 11]);
  });

  it("Copy and Cut take the selected text, not the sheet", async () => {
    const el = field("hello world");
    const inputs: string[] = [];
    el.addEventListener("input", () => void inputs.push(el.value));
    el.setSelectionRange(0, 5);
    runTextCommand(el, "copy", clipboard());
    await flush();
    expect(clip.written).toEqual(["hello"]);
    expect(el.value).toBe("hello world");
    el.setSelectionRange(5, 11);
    runTextCommand(el, "cut", clipboard());
    await flush();
    expect(clip.written).toEqual(["hello", " world"]);
    expect(el.value).toBe("hello");
    expect(inputs).toEqual(["hello"]);
    // Nothing selected: nothing happens.
    runTextCommand(el, "copy", clipboard());
    await flush();
    expect(clip.written.length).toBe(2);
  });

  it("Paste replaces the selection and tells the field; an unreadable clipboard gives the hint", async () => {
    const el = field("a-b", "textarea");
    const inputs: string[] = [];
    el.addEventListener("input", () => void inputs.push(el.value));
    el.setSelectionRange(1, 2);
    clip.text = "+++";
    runTextCommand(el, "paste", clipboard());
    await flush();
    expect(el.value).toBe("a+++b");
    expect([el.selectionStart, el.selectionEnd]).toEqual([4, 4]);
    expect(inputs).toEqual(["a+++b"]);
    clip.text = null;
    runTextCommand(el, "paste", clipboard());
    await flush();
    expect(clip.unreadable).toBe(1);
    expect(el.value).toBe("a+++b");
  });
});

describe("MenuBar details", () => {
  let host: HTMLElement;
  let root: Root;
  let outside: HTMLInputElement;
  const chosen: string[] = [];
  const top = (name: string) => [...host.querySelectorAll<HTMLElement>('[role="menubar"] > [role="menuitem"]')].find((b) => b.textContent === name)!;
  const menus = () => [...host.querySelectorAll<HTMLElement>('[role="menu"]')].map((m) => m.getAttribute("aria-label"));
  const click = (el: HTMLElement) =>
    act(() => {
      el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      el.focus();
      el.click();
    });
  beforeEach(() => {
    chosen.length = 0;
    outside = document.createElement("input");
    document.body.appendChild(outside);
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });
  afterEach(() => {
    act(() => root.unmount());
    document.body.innerHTML = "";
  });

  it("aria-keyshortcuts uses the spec's key names", () => {
    expect(ariaKeyShortcuts("Ctrl+Shift+S")).toBe("Control+Shift+S");
    expect(ariaKeyShortcuts("Ctrl+Alt+K")).toBe("Control+Alt+K");
    expect(ariaKeyShortcuts("Ctrl+,")).toBe("Control+,");
    expect(ariaKeyShortcuts("Shift+F2")).toBe("Shift+F2");
    expect(ariaKeyShortcuts("F12")).toBe("F12");
    expect(ariaKeyShortcuts(null)).toBeUndefined();
    expect(ariaKeyShortcuts("⇧⌘S")).toBeUndefined();
    act(() => root.render(createElement(MenuBar, { model: MODEL, label: "Menu", onActivate: (id: string) => void chosen.push(id) })));
    click(top("File"));
    expect(host.querySelector('[data-id="file.saveAs"]')?.getAttribute("aria-keyshortcuts")).toBe("Control+Shift+S");
  });

  it("Alt+F4 and Alt+Space close the menus and go on to the system", () => {
    act(() => root.render(createElement(MenuBar, { model: MODEL, label: "Menu", onActivate: (id: string) => void chosen.push(id) })));
    const seen = vi.fn();
    window.addEventListener("keydown", seen);
    for (const init of [
      { key: "F4", code: "F4", altKey: true },
      { key: " ", code: "Space", altKey: true },
    ]) {
      outside.focus();
      click(top("Edit"));
      expect(menus()).toEqual(["Edit"]);
      const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
      act(() => void document.activeElement!.dispatchEvent(e));
      expect(e.defaultPrevented).toBe(false);
      expect(menus()).toEqual([]);
      expect(document.activeElement).toBe(outside);
    }
    expect(seen).toHaveBeenCalledTimes(2);
    // Any other Alt / Ctrl combination stays in the bar.
    click(top("Edit"));
    const e = new KeyboardEvent("keydown", { key: "s", ctrlKey: true, bubbles: true, cancelable: true });
    act(() => void document.activeElement!.dispatchEvent(e));
    expect(e.defaultPrevented).toBe(true);
    expect(seen).toHaveBeenCalledTimes(2);
    window.removeEventListener("keydown", seen);
  });

  it("a modal dialog that opens closes the menus", async () => {
    act(() => root.render(createElement(MenuBar, { model: MODEL, label: "Menu", onActivate: (id: string) => void chosen.push(id) })));
    click(top("Data"));
    expect(menus()).toEqual(["Data"]);
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    await act(async () => {
      document.body.appendChild(dialog);
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(menus()).toEqual([]);
  });

  it("itemState learns where the focus was (a text field keeps Cut and Paste)", () => {
    const asked: [string, Element | null][] = [];
    act(() =>
      root.render(
        createElement(MenuBar, {
          model: MODEL,
          label: "Menu",
          onActivate: (id: string) => void chosen.push(id),
          itemState: (id: string, focus: Element | null) => {
            asked.push([id, focus]);
            return { enabled: true };
          },
        }),
      ),
    );
    outside.focus();
    click(top("Edit"));
    expect(asked.find(([id]) => id === "native.cut")?.[1]).toBe(outside);
  });
});
