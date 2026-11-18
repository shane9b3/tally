/**
 * The shell side of the in-app menu bar (MenuBar.tsx): the menu is defined
 * once, in the desktop shell (`apps/desktop/src-tauri/src/menu.rs`), which
 * hands out its model and carries out a chosen item exactly as it does a
 * native menu selection.
 */
import type { MenuNode } from "@tally/bridge";
import { useEffect, useState } from "react";

export interface MenuShell {
  /** `menu_model`: the application menu as it is now. */
  menuModel(): Promise<MenuNode[]>;
  /** `menu_activate`: the item with this id was chosen in this window's bar. */
  menuActivate(id: string): Promise<void>;
  /** `menu-changed`: the new model (language, open documents, check marks). */
  onMenuChanged(cb: (model: MenuNode[]) => void): () => void;
}

/** The shell's menu model, kept up to date (empty until it arrives). */
export function useMenuModel(shell: MenuShell): MenuNode[] {
  const [model, setModel] = useState<MenuNode[]>([]);
  useEffect(() => {
    let live = true;
    // A `menu-changed` that arrives first is newer than the answer to the request.
    let changed = false;
    const off = shell.onMenuChanged((m) => {
      changed = true;
      if (live) setModel(m);
    });
    shell.menuModel().then(
      (m) => {
        if (live && !changed) setModel(m);
      },
      (e: unknown) => console.warn("menu_model failed", e),
    );
    return () => {
      live = false;
      off();
    };
  }, [shell]);
  return model;
}
