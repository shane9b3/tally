import { useEffect } from "react";
import type { AppController } from "./controller";
import { isMacPlatform, isReloadKey, resolveKey } from "./keymap";
import { useEditStore } from "./stores/editStore";

/** True for text fields other than the grid's own (idle) textarea. */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.closest(".tally-grid")) return false;
  return target.isContentEditable || target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
}

/**
 * Window-level keymap → CommandRegistry. `desktop` (the desktop app, not the
 * browser build): the webview's reload keys never reach it, also when no
 * command takes them (F5 or Ctrl+R while typing in a cell).
 */
export function useKeymap(ctl: AppController, nativeMenu: boolean, desktop = false): void {
  useEffect(() => {
    const mac = isMacPlatform();
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      const typing = isTypingTarget(e.target) || useEditStore.getState().edit !== null;
      const key = { key: e.key, code: e.code, metaKey: e.metaKey, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, altGraph: e.getModifierState("AltGraph") };
      const id = resolveKey(key, { typing, nativeMenu, mac });
      if (!id) {
        if (desktop && isReloadKey(key)) e.preventDefault();
        return;
      }
      e.preventDefault();
      ctl.registry.run(id);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ctl, nativeMenu, desktop]);
}
