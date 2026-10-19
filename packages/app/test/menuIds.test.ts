import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { buildCommands } from "../src/commands";
import type { AppController } from "../src/controller";
import { BINDINGS } from "../src/keymap";

const MENU_RS = fileURLToPath(new URL("../../../apps/desktop/src-tauri/src/menu.rs", import.meta.url));

describe("native menu ↔ command registry", () => {
  const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };
  const ids = new Set(buildCommands({} as AppController, ui).map((c) => c.id));
  const menuIds = [...readFileSync(MENU_RS, "utf8").matchAll(/(?:Item\(|with_id\()"([a-z]+\.[A-Za-z.]+)"/g)].map((m) => m[1] ?? "");

  it("every menu item is a registered command", () => {
    expect(menuIds.length).toBeGreaterThan(60);
    expect(menuIds.filter((id) => !ids.has(id))).toEqual([]);
  });

  it("keymap bindings point at registered commands", () => {
    expect(BINDINGS.filter((b) => !ids.has(b.command)).map((b) => b.command)).toEqual([]);
  });
});
