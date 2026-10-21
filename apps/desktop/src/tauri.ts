/** Desktop-only wiring: TauriClient and the window title. */
import { useDocStore, type AppController, type PickedImage, type W5Shell, type WindowChrome } from "@tally/app";
import { TauriClient } from "@tally/bridge/tauri";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** The desktop client: `TauriClient` plus the W5-5 shell extras (pictures, full screen, window chrome). */
export function makeTauriClient(): TauriClient & W5Shell & { windowChrome(): Promise<WindowChrome> } {
  const client = new TauriClient({ invoke, listen });
  return Object.assign(client, {
    /** `pick_image` (apps/desktop/src-tauri/src/images.rs): a PNG/JPEG picked in a native dialog. */
    pickImage: (): Promise<PickedImage | null> => invoke<PickedImage | null>("pick_image"),
    /** View ▸ Full Screen: toggles the window's full-screen state. */
    toggleFullScreen: async (): Promise<boolean> => {
      const win = getCurrentWindow();
      const next = !(await win.isFullscreen());
      await win.setFullscreen(next);
      return next;
    },
    /** `window_chrome` (windows.rs): whether to draw the custom title bar. */
    windowChrome: (): Promise<WindowChrome> => invoke<WindowChrome>("window_chrome"),
  });
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
