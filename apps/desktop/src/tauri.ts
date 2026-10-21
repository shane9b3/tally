/** Desktop-only wiring: TauriClient and the window title. */
import { useDocStore, type AppController, type PickedImage, type W5Shell, type WindowChrome } from "@tally/app";
import { TauriClient } from "@tally/bridge/tauri";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** The desktop client: `TauriClient` plus the W5-5 shell extras (pictures, full screen, window chrome). */
export function makeTauriClient(): TauriClient & W5Shell & { windowChrome(): Promise<WindowChrome> } {
  // Each document window listens to its own window's events: the shell sends
  // `menu-action` and `close-requested` to one window (the global `listen`
  // would also receive the other windows' events).
  const win = getCurrentWebviewWindow();
  const client = new TauriClient({ invoke, listen: (event, handler) => win.listen(event, handler) });
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

/**
 * Mirrors the document title (and dirty state) into the native window title,
 * which also names the window in the Window menu, Mission Control and the
 * macOS window tabs. On macOS the red button's dot (and the dot AppKit puts
 * in the Window menu and on the tab) already says "edited", so the title is
 * the document name only; elsewhere it gets " — Edited".
 */
export function setWindowTitle(_ctl: AppController, mac: boolean): void {
  let last = "";
  const update = (): void => {
    const info = useDocStore.getState().info;
    const title = info ? `${info.title}${info.dirty && !mac ? " — Edited" : ""}` : "Tally";
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
