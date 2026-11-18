/**
 * Dev/e2e only (`?titlebar=1` in the browser build): a stand-in for the
 * desktop shell's menu commands, so the Windows title bar and its menu bar
 * can be seen and driven without the desktop app. The model is the shell's
 * own (`menuModel.json` is written by a Rust test of
 * `apps/desktop/src-tauri/src/menu.rs`, which fails when it is stale), and
 * an activation does what the shell does: unknown ids are refused, the OS
 * items map to the webview's commands, every other command id comes back
 * as a `menu-action`.
 */
import type { MenuNode } from "@tally/bridge";
import type { MenuShell } from "../chrome/menuShell";
import model from "./menuModel.json";

/** The clipboard items the OS handles in a native menu (`Native::in_app`, menu.rs). */
const NATIVE: Readonly<Record<string, string>> = {
  "native.cut": "edit.cut",
  "native.copy": "edit.copy",
  "native.paste": "edit.paste",
  "native.selectAll": "edit.selectAll",
};
const FOCUS = "window.focus.";

function find(nodes: readonly MenuNode[], id: string): MenuNode | undefined {
  for (const n of nodes) {
    if (n.kind === "submenu") {
      const hit = find(n.items, id);
      if (hit) return hit;
    } else if (n.kind !== "separator" && n.id === id) return n;
  }
  return undefined;
}

const withFront = (nodes: readonly MenuNode[], front: string): MenuNode[] => nodes.map((n) => ({ ...n, checked: n.kind === "check" ? n.id === front : n.checked, items: withFront(n.items, front) }));

export interface MockMenuShell extends MenuShell {
  /** Every id the bar activated, in order (e2e). */
  readonly activated: string[];
  /** Replaces the model as a `menu-changed` would (e2e). */
  setModel(next: MenuNode[]): void;
}

/** A mock of `menu_model` / `menu_activate` / `menu-changed` over `client`'s `menu-action` listeners. */
export function mockMenuShell(client: { emitMenuAction(id: string): void }): MockMenuShell {
  let current = model as MenuNode[];
  const listeners = new Set<(m: MenuNode[]) => void>();
  const activated: string[] = [];
  const setModel = (next: MenuNode[]): void => {
    current = next;
    for (const cb of listeners) cb(current);
  };
  return {
    activated,
    setModel,
    menuModel: () => Promise.resolve(current),
    menuActivate: (id) => {
      const node = find(current, id);
      if (!node?.enabled) return Promise.reject({ code: "invalid_argument", message: "this menu item is not available" });
      activated.push(id);
      // The shell shows that document and puts the check marks right.
      if (id.startsWith(FOCUS)) setModel(withFront(current, id));
      // The window items act on the window (nothing to do in a browser tab).
      else if (id !== "native.minimize" && id !== "native.maximize") client.emitMenuAction(NATIVE[id] ?? id);
      return Promise.resolve();
    },
    onMenuChanged: (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
}
