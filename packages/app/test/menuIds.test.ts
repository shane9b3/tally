import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { MenuNode } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { buildCommands } from "../src/commands";
import type { AppController } from "../src/controller";
import fixture from "../src/dev/menuModel.json";
import { BINDINGS, resolveBinding, type KeyLike } from "../src/keymap";

const MENU_RS = fileURLToPath(new URL("../../../apps/desktop/src-tauri/src/menu.rs", import.meta.url));

/** Every item of the shell's menu model (Windows/Linux, written from menu.rs). */
function items(nodes: readonly MenuNode[]): MenuNode[] {
  return nodes.flatMap((n) => (n.kind === "submenu" ? items(n.items) : n.kind === "separator" ? [] : [n]));
}

const KEYS: Readonly<Record<string, [string, string]>> = { ",": [",", "Comma"], "\\": ["\\", "Backslash"], "=": ["=", "Equal"], "-": ["-", "Minus"], "/": ["/", "Slash"], "`": ["`", "Backquote"] };

/** The key event of an accelerator as the menu shows it on Windows ("Ctrl+Shift+S"). */
function keyOf(accelerator: string): KeyLike {
  const parts = accelerator.split("+");
  const k = parts.pop() ?? "";
  const [key, code] = KEYS[k] ?? (/^[A-Z]$/.test(k) ? [k.toLowerCase(), `Key${k}`] : /^[0-9]$/.test(k) ? [k, `Digit${k}`] : [k, k]);
  return { key, code, metaKey: false, ctrlKey: parts.includes("Ctrl"), shiftKey: parts.includes("Shift"), altKey: parts.includes("Alt") };
}

describe("native menu ↔ command registry", () => {
  const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };
  const ids = new Set(buildCommands({} as AppController, ui).map((c) => c.id));
  const menuIds = [...readFileSync(MENU_RS, "utf8").matchAll(/(?:Item\(|item\(\s*|with_id\()"([a-z]+\.[A-Za-z.]+)"/g)].map((m) => m[1] ?? "");

  it("every menu item is a registered command", () => {
    expect(menuIds.length).toBeGreaterThan(60);
    expect(menuIds.filter((id) => !ids.has(id))).toEqual([]);
  });

  it("keymap bindings point at registered commands", () => {
    expect(BINDINGS.filter((b) => !ids.has(b.command)).map((b) => b.command)).toEqual([]);
  });
});

describe("in-app menu bar (Windows) ↔ commands and keymap", () => {
  const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };
  const ids = new Set(buildCommands({} as AppController, ui).map((c) => c.id));
  const all = items(fixture as MenuNode[]);
  // What the shell sends for the items the OS handles in a native menu (`Native::in_app`, menu.rs).
  const NATIVE: Readonly<Record<string, string>> = { "native.cut": "edit.cut", "native.copy": "edit.copy", "native.paste": "edit.paste", "native.selectAll": "edit.selectAll" };
  // Handled by the shell itself, never sent to the webview.
  const shell = (id: string) => id.startsWith("window.") || id === "native.minimize" || id === "native.maximize";

  it("every item the bar can send is a registered command", () => {
    expect(all.length).toBeGreaterThan(180);
    const sent = all.filter((n) => !shell(n.id)).map((n) => NATIVE[n.id] ?? n.id);
    expect(sent.filter((id) => !ids.has(id))).toEqual([]);
    expect(Object.values(NATIVE).every((id) => ids.has(id))).toBe(true);
  });

  it("every accelerator the bar shows works in the webview's keymap (there is no native menu on Windows)", () => {
    // The grid's own key handling: clipboard, Select All and Paste Values (GRID_KEYS).
    const grid = new Set(["native.cut", "native.copy", "native.paste", "native.selectAll", "edit.pasteValues"]);
    const shown = all.filter((n) => n.accelerator !== null && !grid.has(n.id));
    expect(shown.length).toBeGreaterThan(35);
    const wrong = shown.filter((n) => resolveBinding(keyOf(n.accelerator ?? ""), { typing: false, nativeMenu: false, mac: false })?.command !== n.id).map((n) => `${n.id} ${n.accelerator}`);
    expect(wrong).toEqual([]);
    // With a native menu (macOS, Linux) the same keys are left to it.
    const left = shown.filter((n) => resolveBinding(keyOf(n.accelerator ?? ""), { typing: false, nativeMenu: true, mac: false }) !== null).map((n) => `${n.id} ${n.accelerator}`);
    expect(left).toEqual([]);
  });
});
