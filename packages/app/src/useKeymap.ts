import { useEffect } from "react";
import type { AppController } from "./controller";
import { isMacPlatform, resolveKey } from "./keymap";
import { useEditStore } from "./stores/editStore";

/** True for text fields other than the grid's own (idle) textarea. */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.closest(".tally-grid")) return false;
  return target.isContentEditable || target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
}

/** Window-level keymap → CommandRegistry. */
export function useKeymap(ctl: AppController, nativeMenu: boolean): void {
  useEffect(() => {
    const mac = isMacPlatform();
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      const typing = isTypingTarget(e.target) || useEditStore.getState().edit !== null;
      const id = resolveKey(e, { typing, nativeMenu, mac });
      if (!id) return;
      e.preventDefault();
      ctl.registry.run(id);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ctl, nativeMenu]);
}
