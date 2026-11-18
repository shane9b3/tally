/**
 * Custom title bar for Windows (W5-5, §4 item 48): every document window runs
 * without OS decorations there (`src-tauri/src/windows.rs`), so this strip
 * provides the menu bar, the drag region, the document title and the caption
 * buttons (minimize, maximize/restore, close). Not mounted on macOS (overlay
 * title bar, native app menu) or Linux (native decorations and menu bar).
 *
 * The menu bar is in the window because an undecorated Win32 window has no
 * non-client area for a native one. It draws the shell's menu model
 * (`menu_model`, `menu-changed`) and hands a chosen item to `menu_activate`,
 * which takes the native menu's path (`src-tauri/src/menu.rs`). Every
 * document is a webview that fills its window and mounts its own title bar,
 * so the shown tab's bar is the window's and its pop-ups paint over the
 * sheet (no second webview could be drawn over a document webview).
 *
 * The caption buttons use the window API, allowed by
 * `capabilities/windows-titlebar.json`. The shell is passed in so the
 * browser build can mount the bar over mocks (`?titlebar=1`, e2e).
 */
import { MenuBar, useDocStore, useMenuModel, type MenuItemState, type MenuShell } from "@tally/app";
import { useEffect, useState } from "react";
import "./titlebar.css";

export const TITLE_BAR_HEIGHT = 32;

/** The window and menu services the title bar drives. */
export interface TitleBarShell extends MenuShell {
  minimize(): Promise<unknown>;
  toggleMaximize(): Promise<unknown>;
  close(): Promise<unknown>;
  isMaximized(): Promise<boolean>;
}

export interface TitleBarProps {
  shell: TitleBarShell;
  /** The app's state of a menu item (the command registry's enabled state and reason). */
  itemState?: (id: string) => MenuItemState;
}

export function TitleBar({ shell, itemState }: TitleBarProps) {
  const info = useDocStore((s) => s.info);
  const [maximized, setMaximized] = useState(false);
  const model = useMenuModel(shell);
  const title = info ? `${info.title}${info.dirty ? " — Edited" : ""}` : "Tally";

  // The webview is resized with its window, so the DOM resize event follows
  // maximize/restore too (and keeps working after the tab moves to another
  // window, unlike a listener on a window label).
  useEffect(() => {
    const sync = () => void shell.isMaximized().then(setMaximized, () => undefined);
    sync();
    window.addEventListener("resize", sync);
    return () => window.removeEventListener("resize", sync);
  }, [shell]);

  return (
    <div className="tally-titlebar" data-tauri-drag-region data-testid="title-bar" onDoubleClick={() => void shell.toggleMaximize()}>
      <span className="tally-titlebar-logo" data-tauri-drag-region>
        T
      </span>
      <MenuBar model={model} label="Menu" itemState={itemState} onActivate={(id) => void shell.menuActivate(id).catch((e: unknown) => console.warn(`menu_activate ${id} failed`, e))} />
      <span className="tally-titlebar-title" data-tauri-drag-region>
        {title}
      </span>
      <button type="button" className="tally-caption" aria-label="Minimize" onClick={() => void shell.minimize()}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 5h10" stroke="currentColor" strokeWidth="1" />
        </svg>
      </button>
      <button type="button" className="tally-caption" aria-label={maximized ? "Restore" : "Maximize"} onClick={() => void shell.toggleMaximize()}>
        {maximized ? (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1">
            <rect x="0.5" y="2.5" width="7" height="7" />
            <path d="M2.5 2.5v-2h7v7h-2" />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1">
            <rect x="0.5" y="0.5" width="9" height="9" />
          </svg>
        )}
      </button>
      <button type="button" className="tally-caption tally-caption-close" aria-label="Close" onClick={() => void shell.close()}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" stroke="currentColor" strokeWidth="1.1">
          <path d="M0 0l10 10M10 0L0 10" />
        </svg>
      </button>
    </div>
  );
}
