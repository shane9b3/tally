// @vitest-environment happy-dom
/**
 * The in-app menu bar (Windows title bar): it draws the shell's menu model
 * (the fixture is written from `apps/desktop/src-tauri/src/menu.rs`) with the
 * ARIA menubar roles, opens by click, hover and keyboard, and reports the
 * chosen id; the mock shell refuses what the model does not offer.
 */
import type { MenuNode } from "@tally/bridge";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MenuBar, typeAhead, type MenuItemState } from "../src/chrome/MenuBar";
import { useMenuModel, type MenuShell } from "../src/chrome/menuShell";
import fixture from "../src/dev/menuModel.json";
import { mockMenuShell } from "../src/dev/mockMenu";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const MODEL = fixture as MenuNode[];

let host: HTMLElement;
let root: Root;
let outside: HTMLButtonElement;
let chosen: string[];

function render(model: MenuNode[] = MODEL, itemState?: (id: string) => MenuItemState): void {
  act(() => root.render(createElement(MenuBar, { model, label: "Menu", onActivate: (id: string) => void chosen.push(id), itemState })));
}

const q = (sel: string) => host.querySelector<HTMLElement>(sel);
const byId = (id: string) => q(`[data-id=${JSON.stringify(id)}]`)!;
const top = (name: string) => [...host.querySelectorAll<HTMLElement>('[role="menubar"] > [role="menuitem"]')].find((b) => b.textContent === name)!;
const menus = () => [...host.querySelectorAll<HTMLElement>('[role="menu"]')].map((m) => m.getAttribute("aria-label"));
const focused = () => (document.activeElement as HTMLElement | null)?.dataset.id;
const key = (k: string, init: KeyboardEventInit = {}) => act(() => void (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init })));
const keyUp = (k: string) => act(() => void (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent("keyup", { key: k, bubbles: true, cancelable: true })));
const click = (el: HTMLElement) =>
  act(() => {
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    el.focus();
    el.click();
  });
const hover = (el: HTMLElement) => act(() => void el.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));

beforeEach(() => {
  chosen = [];
  outside = document.createElement("button");
  outside.id = "outside";
  document.body.appendChild(outside);
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
});

describe("MenuBar", () => {
  it("draws the shell's menus as a menubar with one tab stop", () => {
    render();
    const bar = q('[role="menubar"]')!;
    expect(bar.getAttribute("aria-label")).toBe("Menu");
    const tops = [...bar.querySelectorAll<HTMLElement>('[role="menuitem"]')];
    expect(tops.map((b) => b.textContent)).toEqual(["File", "Edit", "Insert", "Format", "Data", "View", "Sheet", "Tools", "Window", "Help"]);
    expect(tops.map((b) => b.tabIndex)).toEqual([0, -1, -1, -1, -1, -1, -1, -1, -1, -1]);
    expect(tops.every((b) => b.getAttribute("aria-haspopup") === "menu" && b.getAttribute("aria-expanded") === "false")).toBe(true);
    expect(menus()).toEqual([]);
    // The bar is never part of the window's drag region.
    expect(bar.parentElement?.getAttribute("data-tauri-drag-region")).toBe("false");
  });

  it("a click opens a menu, hovering switches, a second click closes", () => {
    render();
    click(top("File"));
    expect(menus()).toEqual(["File"]);
    expect(top("File").getAttribute("aria-expanded")).toBe("true");
    const file = q('[role="menu"]')!;
    expect(file.querySelectorAll('[role="separator"]').length).toBe(3);
    const save = byId("file.save");
    expect(save.getAttribute("role")).toBe("menuitem");
    expect(save.getAttribute("aria-keyshortcuts")).toBe("Ctrl+S");
    expect(save.textContent).toBe("SaveCtrl+S");
    expect(byId("menu/File/Export").getAttribute("aria-haspopup")).toBe("menu");
    hover(top("Edit"));
    expect(menus()).toEqual(["Edit"]);
    expect(byId("native.copy").textContent).toBe("CopyCtrl+C");
    // Hovering a submenu item opens it; hovering a plain item closes it again.
    hover(byId("menu/Edit/Fill"));
    expect(menus()).toEqual(["Edit", "Fill"]);
    expect(byId("menu/Edit/Fill").getAttribute("aria-expanded")).toBe("true");
    hover(byId("edit.find"));
    expect(menus()).toEqual(["Edit"]);
    click(top("Edit"));
    expect(menus()).toEqual([]);
    // Without an open menu hovering does nothing.
    hover(top("View"));
    expect(menus()).toEqual([]);
  });

  it("choosing an item reports its id and gives the focus back", () => {
    render();
    outside.focus();
    click(top("Format"));
    click(byId("format.bold"));
    expect(chosen).toEqual(["format.bold"]);
    expect(menus()).toEqual([]);
    expect(document.activeElement).toBe(outside);
    // An item of a submenu.
    click(top("File"));
    click(byId("menu/File/Export"));
    expect(menus()).toEqual(["File", "Export"]);
    click(byId("file.exportPdf"));
    expect(chosen).toEqual(["format.bold", "file.exportPdf"]);
  });

  it("the keyboard walks the bar, the menus and the submenus", () => {
    render();
    outside.focus();
    act(() => top("File").focus());
    key("ArrowRight");
    expect(focused()).toBe("menu/Edit");
    expect(menus()).toEqual([]);
    key("End");
    expect(focused()).toBe("menu/Help");
    key("ArrowRight");
    expect(focused()).toBe("menu/File");
    expect(top("File").tabIndex).toBe(0);
    key("ArrowDown");
    expect(menus()).toEqual(["File"]);
    expect(focused()).toBe("file.new");
    key("ArrowDown");
    expect(focused()).toBe("file.templates");
    key("ArrowUp");
    key("ArrowUp");
    expect(focused()).toBe("file.close");
    key("Home");
    expect(focused()).toBe("file.new");
    // Right on a plain item goes to the next menu; Left comes back.
    key("ArrowRight");
    expect(menus()).toEqual(["Edit"]);
    expect(focused()).toBe("edit.undo");
    key("ArrowLeft");
    expect(menus()).toEqual(["File"]);
    // A letter jumps to the next item starting with it.
    key("s");
    expect(focused()).toBe("file.save");
    key("s");
    expect(focused()).toBe("file.saveAs");
    key("e");
    expect(focused()).toBe("menu/File/Export");
    key("ArrowRight");
    expect(menus()).toEqual(["File", "Export"]);
    expect(focused()).toBe("file.exportXlsx");
    key("End");
    expect(focused()).toBe("file.exportPdf");
    key("ArrowLeft");
    expect(menus()).toEqual(["File"]);
    expect(focused()).toBe("menu/File/Export");
    key("Enter");
    expect(menus()).toEqual(["File", "Export"]);
    key("Escape");
    expect(menus()).toEqual(["File"]);
    key("Escape");
    expect(menus()).toEqual([]);
    expect(focused()).toBe("menu/File");
    key("Escape");
    expect(document.activeElement).toBe(outside);
    // Up opens with the last item; Space chooses.
    act(() => top("Help").focus());
    key("ArrowUp");
    expect(focused()).toBe("help.about");
    key(" ");
    expect(chosen).toEqual(["help.about"]);
    expect(document.activeElement).toBe(outside);
  });

  it("Alt and F10 move the focus to the bar and back", () => {
    render();
    outside.focus();
    key("Alt");
    keyUp("Alt");
    expect(focused()).toBe("menu/File");
    expect(menus()).toEqual([]);
    key("e");
    expect(menus()).toEqual(["Edit"]);
    expect(focused()).toBe("edit.undo");
    key("Alt");
    keyUp("Alt");
    expect(menus()).toEqual([]);
    expect(document.activeElement).toBe(outside);
    // Alt with another key (Alt+1: a tab shortcut) is not the menu key.
    key("Alt");
    key("1", { altKey: true });
    keyUp("Alt");
    expect(document.activeElement).toBe(outside);
    key("F10");
    expect(focused()).toBe("menu/Edit");
    key("F10");
    expect(document.activeElement).toBe(outside);
    // Shift+F10 is the context menu key.
    key("F10", { shiftKey: true });
    expect(document.activeElement).toBe(outside);
  });

  it("keys never reach the app while the bar has the focus; Tab leaves", () => {
    render();
    const seen: string[] = [];
    const listen = (e: KeyboardEvent) => void seen.push(e.key);
    window.addEventListener("keydown", listen);
    outside.focus();
    key("b", { ctrlKey: true });
    click(top("File"));
    key("ArrowDown");
    key("b", { ctrlKey: true });
    key("x");
    expect(menus()).toEqual(["File"]);
    key("Tab");
    expect(menus()).toEqual([]);
    expect(document.activeElement).toBe(outside);
    window.removeEventListener("keydown", listen);
    expect(seen).toEqual(["b"]);
  });

  it("a click outside closes the menus", () => {
    render();
    click(top("Data"));
    expect(menus()).toEqual(["Data"]);
    act(() => void outside.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
    expect(menus()).toEqual([]);
    click(top("Data"));
    act(() => void window.dispatchEvent(new Event("blur")));
    expect(menus()).toEqual([]);
  });

  it("shows check marks, and items the shell or the app turns off", () => {
    const off = new Set(["edit.undo"]);
    const model = MODEL.map((m) => (m.label === "Edit" ? { ...m, items: m.items.map((n) => (n.id === "native.paste" ? { ...n, enabled: false } : n)) } : m));
    render(model, (id) => (off.has(id) ? { enabled: false, reason: "Nothing to undo" } : { enabled: true }));
    click(top("Window"));
    const docs = [...host.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]')];
    expect(docs.map((d) => [d.textContent, d.getAttribute("aria-checked")])).toEqual([
      ["✓Untitled 1", "true"],
      ["Budget & Plan.ods", "false"],
    ]);
    click(docs[1]!);
    expect(chosen).toEqual(["window.focus.doc-2"]);
    click(top("Edit"));
    const undo = byId("edit.undo");
    expect(undo.getAttribute("aria-disabled")).toBe("true");
    expect(undo.title).toBe("Nothing to undo");
    expect(byId("native.paste").getAttribute("aria-disabled")).toBe("true");
    expect(byId("edit.redo").hasAttribute("aria-disabled")).toBe(false);
    click(undo);
    click(byId("native.paste"));
    expect(chosen).toEqual(["window.focus.doc-2"]);
    expect(menus()).toEqual(["Edit"]);
    // The keyboard still stops on an item that is off, but Enter does nothing.
    act(() => undo.focus());
    key("Enter");
    expect(chosen).toEqual(["window.focus.doc-2"]);
  });

  it("follows a new model (language, open documents)", () => {
    render();
    click(top("Window"));
    const de = MODEL.map((m) => (m.label === "Window" ? { ...m, label: "Fenster", items: m.items.filter((n) => n.id !== "window.focus.doc-2") } : m));
    render(de);
    expect(menus()).toEqual(["Fenster"]);
    expect(host.querySelectorAll('[role="menuitemcheckbox"]').length).toBe(1);
    // A menu that is gone closes.
    render(de.filter((m) => m.label !== "Fenster"));
    expect(menus()).toEqual([]);
  });

  it("typeAhead finds the next label starting with a letter", () => {
    const labels = ["New", "Open…", "Save", "Save As…", "Close"];
    expect(typeAhead(labels, 0, "s")).toBe(2);
    expect(typeAhead(labels, 2, "S")).toBe(3);
    expect(typeAhead(labels, 3, "s")).toBe(2);
    expect(typeAhead(labels, 0, "n")).toBe(0);
    expect(typeAhead(labels, 0, "x")).toBe(-1);
    expect(typeAhead([], 0, "x")).toBe(-1);
  });
});

describe("the menu shell", () => {
  it("the mock does what the shell does: the model, activation by id, refusals", async () => {
    const actions: string[] = [];
    const shell = mockMenuShell({ emitMenuAction: (id) => void actions.push(id) });
    const model = await shell.menuModel();
    expect(model.map((m) => m.label)).toEqual(["File", "Edit", "Insert", "Format", "Data", "View", "Sheet", "Tools", "Window", "Help"]);
    await shell.menuActivate("format.bold");
    // The OS's clipboard items become the webview's commands; the window items stay in the shell.
    await shell.menuActivate("native.copy");
    await shell.menuActivate("native.minimize");
    expect(actions).toEqual(["format.bold", "edit.copy"]);
    for (const id of ["", "not.a.command", "menu/File", "palette.toggle", "window.focus.doc-9"]) {
      await expect(shell.menuActivate(id)).rejects.toMatchObject({ code: "invalid_argument" });
    }
    const seen: MenuNode[][] = [];
    const off = shell.onMenuChanged((m) => void seen.push(m));
    await shell.menuActivate("window.focus.doc-2");
    const docs = seen[0]!.find((m) => m.label === "Window")!.items.filter((n) => n.kind === "check");
    expect(docs.map((d) => d.checked)).toEqual([false, true]);
    off();
    shell.setModel(model);
    expect(seen.length).toBe(1);
    expect(shell.activated).toEqual(["format.bold", "native.copy", "native.minimize", "window.focus.doc-2"]);
  });

  it("useMenuModel asks once and follows menu-changed", async () => {
    let emit: (m: MenuNode[]) => void = () => undefined;
    const unsub = vi.fn();
    const shell: MenuShell = {
      menuModel: () => Promise.resolve(MODEL),
      menuActivate: () => Promise.resolve(),
      onMenuChanged: (cb) => {
        emit = cb;
        return unsub;
      },
    };
    const seen: number[] = [];
    function Probe() {
      seen.push(useMenuModel(shell).length);
      return null;
    }
    act(() => root.render(createElement(Probe)));
    await act(async () => {
      await Promise.resolve();
    });
    expect(seen.at(-1)).toBe(10);
    act(() => emit(MODEL.slice(0, 3)));
    expect(seen.at(-1)).toBe(3);
    act(() => root.unmount());
    expect(unsub).toHaveBeenCalledTimes(1);
    root = createRoot(host);
  });
});
