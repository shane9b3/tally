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
