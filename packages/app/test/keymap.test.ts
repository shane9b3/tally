import { describe, expect, it } from "vitest";
import { resolveKey, shortcutFor, type KeyLike } from "../src/keymap";
import { CommandRegistry } from "../src/registry";

const key = (k: string, mods: Partial<KeyLike> = {}): KeyLike => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
});
const idle = { typing: false, nativeMenu: false };

describe("keymap", () => {
  it("maps the core shortcuts (Cmd or Ctrl)", () => {
    expect(resolveKey(key("z", { metaKey: true }), idle)).toBe("edit.undo");
    expect(resolveKey(key("Z", { metaKey: true, shiftKey: true }), idle)).toBe("edit.redo");
    expect(resolveKey(key("b", { ctrlKey: true }), idle)).toBe("format.bold");
    expect(resolveKey(key("i", { metaKey: true }), idle)).toBe("format.italic");
    expect(resolveKey(key("k", { metaKey: true }), idle)).toBe("palette.toggle");
    expect(resolveKey(key("\\", { metaKey: true }), idle)).toBe("view.toggleSidebar");
    expect(resolveKey(key("«", { metaKey: true, code: "Backslash" }), idle)).toBe("view.toggleSidebar");
    expect(resolveKey(key("s", { metaKey: true }), idle)).toBe("file.save");
    expect(resolveKey(key("n", { metaKey: true }), idle)).toBe("file.new");
    expect(resolveKey(key("o", { metaKey: true }), idle)).toBe("file.open");
  });
  it("ignores plain keys and F2 (the grid handles them)", () => {
    expect(resolveKey(key("b"), idle)).toBeNull();
    expect(resolveKey(key("F2"), idle)).toBeNull();
    expect(resolveKey(key("b", { metaKey: true, altKey: true }), idle)).toBeNull();
  });
  it("only global bindings work while typing", () => {
    const typing = { typing: true, nativeMenu: false };
    expect(resolveKey(key("z", { metaKey: true }), typing)).toBeNull();
    expect(resolveKey(key("b", { metaKey: true }), typing)).toBeNull();
    expect(resolveKey(key("k", { metaKey: true }), typing)).toBe("palette.toggle");
  });
  it("leaves menu accelerators to the native menu in Tauri", () => {
    const tauri = { typing: false, nativeMenu: true };
    expect(resolveKey(key("z", { metaKey: true }), tauri)).toBeNull();
    expect(resolveKey(key("s", { metaKey: true }), tauri)).toBeNull();
    // Format ▸ Bold and Edit ▸ Fill Down carry these accelerators natively.
    expect(resolveKey(key("b", { metaKey: true }), tauri)).toBeNull();
    expect(resolveKey(key("d", { metaKey: true }), tauri)).toBeNull();
    expect(resolveKey(key("l", { metaKey: true, shiftKey: true }), tauri)).toBe("format.alignLeft");
    expect(resolveKey(key("k", { metaKey: true }), tauri)).toBe("palette.toggle");
  });
  it("v0.2 bindings: find, fill, paste special, recalc", () => {
    const idle = { typing: false, nativeMenu: false };
    expect(resolveKey(key("f", { metaKey: true }), idle)).toBe("edit.find");
    expect(resolveKey(key("ƒ", { metaKey: true, altKey: true, code: "KeyF" }), idle)).toBe("edit.findReplace");
    expect(resolveKey(key("√", { metaKey: true, altKey: true, code: "KeyV" }), idle)).toBe("edit.pasteSpecial");
    expect(resolveKey(key("d", { ctrlKey: true }), idle)).toBe("edit.fillDown");
    expect(resolveKey(key("r", { metaKey: true }), idle)).toBe("edit.fillRight");
    expect(resolveKey(key("F9", { metaKey: true, shiftKey: true }), idle)).toBe("data.recalc");
    expect(resolveKey(key("Escape"), idle)).toBe("edit.cancelClip");
    // ⌘F works while typing in the formula bar.
    expect(resolveKey(key("f", { metaKey: true }), { typing: true, nativeMenu: false })).toBe("edit.find");
  });
  it("labels shortcuts", () => {
    expect(shortcutFor("format.bold")).toBe("⌘B");
    expect(shortcutFor("nope")).toBeUndefined();
  });
});

describe("CommandRegistry", () => {
  it("runs enabled commands only and hides hidden ones", () => {
    const reg = new CommandRegistry();
    let ran = 0;
    reg.register({ id: "a", title: "A", kind: "Edit", run: () => void ran++ });
    reg.register({ id: "b", title: "B", kind: "Edit", enabled: () => false, run: () => void ran++ });
    const off = reg.register({ id: "c", title: "C", kind: "View", hidden: true, run: () => void ran++ });
    expect(reg.run("a")).toBe(true);
    expect(reg.run("b")).toBe(false);
    expect(reg.run("zzz")).toBe(false);
    expect(reg.list().map((c) => c.id)).toEqual(["a", "b"]);
    expect(reg.run("c")).toBe(true);
    off();
    expect(reg.has("c")).toBe(false);
    expect(ran).toBe(2);
  });
});

describe("v0.2 bindings", () => {
  const ev = (key: string, mods: Partial<{ meta: boolean; shift: boolean; code: string }> = {}) => ({
    key,
    code: mods.code,
    metaKey: mods.meta ?? true,
    ctrlKey: false,
    shiftKey: mods.shift ?? false,
    altKey: false,
  });
  it("maps the new shortcuts", () => {
    expect(resolveKey(ev("u"), { typing: false, nativeMenu: false })).toBe("format.underline");
    expect(resolveKey(ev("G", { shift: true }), { typing: true, nativeMenu: false })).toBe("view.glance");
    expect(resolveKey(ev("/", { code: "Slash" }), { typing: true, nativeMenu: false })).toBe("help.shortcuts");
    expect(resolveKey(ev("$", { shift: true, code: "Digit4" }), { typing: false, nativeMenu: false })).toBe("format.currency");
    // Undo while typing belongs to the editor, not the document keymap.
    expect(resolveKey(ev("z"), { typing: true, nativeMenu: false })).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// Wave 5 (W5-6, §4 item 49): the shortcut table and docs/shortcuts.md
// ---------------------------------------------------------------------------
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { buildCommands } from "../src/commands";
import type { AppController } from "../src/controller";
import { BINDINGS, GRID_KEYS } from "../src/keymap";
import { renderShortcutsDoc } from "../src/options/shortcutsDoc";

describe("shortcut parity (W5-6)", () => {
  const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };
  const titles = new Map(buildCommands({} as AppController, ui).map((c) => [c.id, c.title]));
  const idle = { typing: false, nativeMenu: false };
  const ev = (key: string, mods: Partial<KeyLike> = {}): KeyLike => ({ key, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...mods });

  it.each([
    ["⌘1 Format Cells", ev("1", { metaKey: true, code: "Digit1" }), "format.cells"],
    ["⌘F2 Function Wizard", ev("F2", { metaKey: true }), "insert.function"],
    ["⌃` show formulas", ev("`", { ctrlKey: true, code: "Backquote" }), "view.showFormulas"],
    ["F5 Navigator", ev("F5"), "view.navigator"],
    ["⇧F5 dependents", ev("F5", { shiftKey: true }), "tools.traceDependents"],
    ["⇧F7 precedents", ev("F7", { shiftKey: true }), "tools.tracePrecedents"],
    ["⌘F8 value highlighting", ev("F8", { metaKey: true }), "view.valueHighlighting"],
    ["⌘F11 styles", ev("F11", { metaKey: true }), "view.stylesPanel"],
    ["⇧F11 save as template", ev("F11", { shiftKey: true }), "file.saveAsTemplate"],
    ["F12 group", ev("F12"), "data.group"],
    ["⌘F12 ungroup", ev("F12", { metaKey: true }), "data.ungroup"],
    ["⌘⇧J full screen", ev("J", { metaKey: true, shiftKey: true }), "view.fullScreen"],
    ["⌘⇧T name box", ev("T", { metaKey: true, shiftKey: true }), "edit.focusNameBox"],
    ["⌘⌥C note", ev("ç", { metaKey: true, altKey: true, code: "KeyC" }), "insert.note"],
    ["⌘⇧Y repeat", ev("Y", { metaKey: true, shiftKey: true }), "edit.repeat"],
    ["⌘, options", ev(",", { metaKey: true, code: "Comma" }), "tools.options"],
    ["F9 recalc", ev("F9"), "data.recalc"],
    ["⌘M clear formats", ev("m", { metaKey: true }), "edit.clearFormats"],
  ])("%s", (_label, e, command) => {
    expect(resolveKey(e, idle)).toBe(command);
  });

  it("every binding has a registered command and a unique key chord", () => {
    const chords = BINDINGS.map((b) => `${b.key}|${b.code ?? ""}|${b.mod}|${b.shift}|${b.alt ?? false}`);
    const dup = chords.filter((c, i) => chords.indexOf(c) !== i);
    expect(dup).toEqual([]);
    expect(BINDINGS.filter((b) => !titles.has(b.command)).map((b) => b.command)).toEqual([]);
  });

  it("docs/shortcuts.md is generated from the keymap (TALLY_WRITE_SHORTCUTS=1 rewrites it)", () => {
    const path = fileURLToPath(new URL("../../../docs/shortcuts.md", import.meta.url));
    const rendered = renderShortcutsDoc(BINDINGS, GRID_KEYS, (id) => titles.get(id) ?? id);
    if (process.env.TALLY_WRITE_SHORTCUTS) writeFileSync(path, rendered);
    let current = "";
    try {
      current = readFileSync(path, "utf8");
    } catch {
      current = "";
    }
    expect(current).toBe(rendered);
  });
});
