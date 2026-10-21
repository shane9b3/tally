/**
 * Custom title bar for Windows (W5-5, §4 item 48): every document window runs
 * without OS decorations there (`src-tauri/src/windows.rs`), so this strip
 * provides the drag region, the document title and the caption buttons
 * (minimize, maximize/restore, close) through the window API, allowed by
 * `capabilities/windows-titlebar.json`. Not mounted on macOS (overlay title
 * bar) or Linux (native decorations).
 */
import { useDocStore } from "@tally/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import "./titlebar.css";

export const TITLE_BAR_HEIGHT = 32;

export function TitleBar() {
  const info = useDocStore((s) => s.info);
  const [maximized, setMaximized] = useState(false);
  const title = info ? `${info.title}${info.dirty ? " — Edited" : ""}` : "Tally";

  useEffect(() => {
    const win = getCurrentWindow();
    let off: (() => void) | undefined;
    const sync = () => void win.isMaximized().then(setMaximized, () => undefined);
    sync();
    void win.onResized(sync).then((f) => {
      off = f;
    });
    return () => off?.();
  }, []);

  const win = () => getCurrentWindow();
  return (
    <div className="tally-titlebar" data-tauri-drag-region data-testid="title-bar" onDoubleClick={() => void win().toggleMaximize()}>
      <span className="tally-titlebar-logo" data-tauri-drag-region>
        T
      </span>
      <span className="tally-titlebar-title" data-tauri-drag-region>
        {title}
      </span>
      <span className="tally-titlebar-spacer" data-tauri-drag-region />
      <button type="button" className="tally-caption" aria-label="Minimize" onClick={() => void win().minimize()}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 5h10" stroke="currentColor" strokeWidth="1" />
        </svg>
      </button>
      <button type="button" className="tally-caption" aria-label={maximized ? "Restore" : "Maximize"} onClick={() => void win().toggleMaximize()}>
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
      <button type="button" className="tally-caption tally-caption-close" aria-label="Close" onClick={() => void win().close()}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" stroke="currentColor" strokeWidth="1.1">
          <path d="M0 0l10 10M10 0L0 10" />
        </svg>
      </button>
    </div>
  );
}
