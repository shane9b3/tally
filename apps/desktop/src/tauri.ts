/** Desktop-only wiring: TauriClient and the window title. */
import { useDocStore, type AppController } from "@tally/app";
import { TauriClient } from "@tally/bridge/tauri";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function makeTauriClient(): TauriClient {
  return new TauriClient({ invoke, listen });
}

/** Mirrors the document title (and dirty state) into the native window title. */
export function setWindowTitle(_ctl: AppController): void {
  let last = "";
  const update = (): void => {
    const info = useDocStore.getState().info;
    const title = info ? `${info.title}${info.dirty ? " — Edited" : ""}` : "Tally";
    if (title === last) return;
    last = title;
    document.title = title;
    getCurrentWindow()
      .setTitle(title)
      .catch(() => undefined);
  };
  update();
  useDocStore.subscribe(update);
}
