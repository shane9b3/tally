/**
 * Optional desktop shell services W5-5 uses beyond `ShellClient`
 * (`apps/desktop/src/tauri.ts` adds them to the Tauri client; the browser
 * build falls back to the DOM where it can).
 */

import type { EngineClientV3, EngineClientV4 } from "@tally/bridge";

/** A picture chosen with `pick_image` (apps/desktop/src-tauri/src/images.rs). */
export interface PickedImage {
  /** File name (without directories). */
  name: string;
  /** `image/png` or `image/jpeg`. */
  mime: string;
  /** Base64 bytes (≤ 16 MiB decoded). */
  data: string;
  /** Pixel size from the file header, when known. */
  width: number | null;
  height: number | null;
}

/** What the desktop window reports about its chrome (`window_chrome`, windows.rs). */
export interface WindowChrome {
  /** The React title bar replaces the OS one (Windows, decorations off). */
  customTitleBar: boolean;
  platform: "macos" | "windows" | "linux" | "other";
}

export interface W5Shell {
  /** Native file dialog for Insert ▸ Image; `null` when cancelled. */
  pickImage?(): Promise<PickedImage | null>;
  /** Toggles the window's full-screen state; resolves with the new state. */
  toggleFullScreen?(): Promise<boolean>;
}

/**
 * The controller's client as `EngineClientV4` (the app's clients implement
 * it; the controller's declared type stays `EngineClientV3` until W5 closes).
 */
export function clientV4(ctl: { client: EngineClientV3 }): EngineClientV4 {
  return ctl.client as EngineClientV4;
}
