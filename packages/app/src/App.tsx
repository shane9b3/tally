/**
 * The application chrome:
 *
 *   sidebar | toolbar
 *           | formula bar
 *           | grid
 *           | sheet tabs ........ status bar
 *
 * All engine access goes through the injected `EngineClient`.
 */
import "@tally/tokens/fonts";
import "@tally/tokens/tokens.css";
import "./global.css";
import type { EngineClient } from "@tally/bridge";
import { useEffect, useMemo, useState } from "react";
import { buildCommands } from "./commands";
import { CommandPalette } from "./components/CommandPalette";
import { FormulaBar } from "./components/FormulaBar";
import { GridHost } from "./components/GridHost";
import { SheetTabs } from "./components/SheetTabs";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { Toasts } from "./components/Toasts";
import { Toolbar } from "./components/Toolbar";
import { ControllerContext } from "./context";
import { AppController } from "./controller";
import { createBrowserShell, type ShellApi } from "./shell";
import { useUiStore } from "./stores/uiStore";
import { useKeymap } from "./useKeymap";
import styles from "./App.module.css";

export interface Platform {
  /** macOS overlay title bar: leave room for the traffic lights. */
  trafficLights: boolean;
  /** A native menu owns the menu accelerators (Tauri). */
  nativeMenu: boolean;
}

export interface AppProps {
  client: EngineClient;
  /** File dialogs, recent files, templates. Defaults to an in-browser stand-in. */
  shell?: ShellApi;
  /** Open this template instead of a blank workbook on start. */
  initialTemplate?: string | null;
  platform?: Partial<Platform>;
  /** Exposes the controller (e2e hooks, the desktop window title). */
  onReady?: (ctl: AppController) => void;
}

export function App({ client, shell, initialTemplate, platform, onReady }: AppProps) {
  const trafficLights = platform?.trafficLights ?? false;
  const nativeMenu = platform?.nativeMenu ?? false;
  const ctl = useMemo(
    () => new AppController(client, shell ?? createBrowserShell(client), { nativeMenu }),
    [client, shell, nativeMenu],
  );
  const [renaming, setRenaming] = useState<number | null>(null);
  const [about, setAbout] = useState(false);
  const sidebarOpen = useUiStore((s) => s.sidebarOpen);

  useEffect(() => {
    const offCommands = ctl.registry.registerAll(
      buildCommands(ctl, {
        renameActiveSheet: () => setRenaming(ctl.sheet),
        showAbout: () => setAbout(true),
      }),
    );
    const detach = ctl.attach();
    void ctl.start(initialTemplate);
    onReady?.(ctl);
    return () => {
      offCommands();
      detach();
    };
    // initialTemplate/onReady are read on mount only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctl]);

  useKeymap(ctl, nativeMenu);

  return (
    <ControllerContext.Provider value={ctl}>
      <div className={styles.app}>
        {sidebarOpen ? <Sidebar inset={trafficLights} /> : null}
        <main className={styles.main}>
          <Toolbar inset={trafficLights && !sidebarOpen} />
          <FormulaBar />
          <GridHost />
          <footer className={styles.footer}>
            <SheetTabs renaming={renaming} setRenaming={setRenaming} />
            <StatusBar />
          </footer>
        </main>
        <CommandPalette />
        <Toasts />
        {about ? (
          <div className={styles.aboutOverlay} onMouseDown={() => setAbout(false)}>
            <div className={styles.about} role="dialog" aria-label="About Tally">
              <div className={styles.logo}>T</div>
              <div className={styles.aboutTitle}>Tally</div>
              <div className={styles.aboutText}>Version 0.1.0 (Phase A)</div>
              <div className={styles.aboutText}>A modern, OpenOffice-compatible spreadsheet.</div>
            </div>
          </div>
        ) : null}
      </div>
    </ControllerContext.Provider>
  );
}
