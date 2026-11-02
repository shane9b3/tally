/** Desktop-only wiring: TauriClient and the window title. */
import { displayTitle, useDocStore, type AppController, type PickedImage, type W5Shell, type WindowChrome } from "@tally/app";
import { TauriClient } from "@tally/bridge/tauri";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** The desktop client: `TauriClient` plus the W5-5 shell extras (pictures, full screen, window chrome). */
export function makeTauriClient(): TauriClient & W5Shell & { windowChrome(): Promise<WindowChrome> } {
  // Each document webview listens to its own events: the shell sends
  // `menu-action`, `close-requested` and `tabs-changed` to document webviews
  // by label (the global `listen` would also receive the others' events).
  // The webview's label never changes, also when its tab moves to another
  // window.
  const win = getCurrentWebviewWindow();
  const client = new TauriClient({ invoke, listen: (event, handler) => win.listen(event, handler) });
  return Object.assign(client, {
    /** `pick_image` (apps/desktop/src-tauri/src/images.rs): a PNG/JPEG picked in a native dialog. */
    pickImage: (): Promise<PickedImage | null> => invoke<PickedImage | null>("pick_image"),
    /**
     * View ▸ Full Screen: toggles the window's full-screen state. The window
     * plugin is called without a label, so Tauri acts on the window this
     * webview is in now (the label baked into `getCurrentWindow()` goes stale
     * when the tab moves to another window).
     */
    toggleFullScreen: async (): Promise<boolean> => {
      const next = !(await invoke<boolean>("plugin:window|is_fullscreen"));
      await invoke("plugin:window|set_fullscreen", { value: next });
      return next;
    },
    /** `window_chrome` (windows.rs): whether to draw the custom title bar. */
    windowChrome: (): Promise<WindowChrome> => invoke<WindowChrome>("window_chrome"),
  });
}

/**
 * Mirrors the document title (and dirty state) into `document.title`. The
 * native window title belongs to the shell, which sets it to the active
 * tab's document (windows.rs; it names the window in the Dock, the Window
 * menu and Mission Control): a hidden tab must not rename its window.
 */
export function setWindowTitle(_ctl: AppController, mac: boolean): void {
  let last = "";
  const update = (): void => {
    const info = useDocStore.getState().info;
    const title = info ? `${displayTitle(info)}${info.dirty && !mac ? " — Edited" : ""}` : "Tally";
    if (title === last) return;
    last = title;
    document.title = title;
  };
  update();
  useDocStore.subscribe(update);
}
