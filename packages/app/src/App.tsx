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
import type { EngineClientV3, ShellClient } from "@tally/bridge";
import { useEffect, useMemo, useState } from "react";
import { buildCommands } from "./commands";
import { CommandPalette } from "./components/CommandPalette";
import { ContextMenu, openResizeDialog } from "./components/ContextMenu";
import { CellPopovers } from "./components/CellPopovers";
import { Dialogs } from "./components/Dialogs";
import { FilterPopover } from "./components/FilterPopover";
import { FindBar } from "./components/FindBar";
import { FormatBar } from "./components/FormatBar";
import { FormulaAssist } from "./components/FormulaAssist";
import { FormulaBar } from "./components/FormulaBar";
import { GlancePanel } from "./components/GlancePanel";
import { ShortcutsModal } from "./components/ShortcutsModal";
import { GridHost } from "./components/GridHost";
import { SheetTabs } from "./components/SheetTabs";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { Toasts } from "./components/Toasts";
import { Toolbar } from "./components/Toolbar";
import { ChartEditorPanel } from "./charts/ChartEditorPanel";
import { useChartStore } from "./charts/chartStore";
import { PrintPreview } from "./print/PrintPreview";
import { W56Chrome, W56Panels } from "./data/W56Chrome";
import { ControllerContext } from "./context";
import { AppController } from "./controller";
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
  /** Engine and shell (`TauriClient` in the desktop app, `MockClient` in a browser). */
  client: EngineClientV3 & ShellClient;
  /** Open this template instead of a blank workbook on start. */
  initialTemplate?: string | null;
  platform?: Partial<Platform>;
  /** Exposes the controller (e2e hooks, the desktop window title). */
  onReady?: (ctl: AppController) => void;
}

export function App({ client, initialTemplate, platform, onReady }: AppProps) {
  const trafficLights = platform?.trafficLights ?? false;
  const nativeMenu = platform?.nativeMenu ?? false;
  const ctl = useMemo(
    () => new AppController(client, client, { nativeMenu }),
    [client, nativeMenu],
  );
  const [renaming, setRenaming] = useState<number | null>(null);
  const [about, setAbout] = useState(false);
  const sidebarOpen = useUiStore((s) => s.sidebarOpen);
  const glanceOpen = useUiStore((s) => s.glanceOpen);
  const chartEditorOpen = useChartStore((s) => s.editorOpen);

  // "Match system": follow prefers-color-scheme changes.
  useEffect(() => {
    if (typeof matchMedia !== "function") return undefined;
    const mq = matchMedia("(prefers-color-scheme: dark)");
    const on = () => useUiStore.getState().syncSystemTheme();
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);

  useEffect(() => {
    const uiHooks = {
        renameActiveSheet: () => setRenaming(ctl.sheet),
        showAbout: () => setAbout(true),
        customNumberFormat: () => {
          void ctl.activeStyle().then(
            (st) => useUiStore.getState().openDialog({ kind: "numberFormat", code: st?.numberFormat ?? "General" }),
            () => useUiStore.getState().openDialog({ kind: "numberFormat", code: "General" }),
          );
        },
        resize: (axis: "row" | "col") => openResizeDialog(ctl, axis),
    };
    const offCommands = ctl.registry.registerAll(buildCommands(ctl, uiHooks));
    ctl.w56.setCommandRebuilder(() => buildCommands(ctl, uiHooks));
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
          <FormatBar />
          <FormulaBar />
          <div className={styles.work}>
            <GridHost />
            <FindBar />
            {glanceOpen ? <GlancePanel /> : null}
            {chartEditorOpen ? <ChartEditorPanel /> : null}
            <W56Panels />
          </div>
          <footer className={styles.footer}>
            <SheetTabs renaming={renaming} setRenaming={setRenaming} />
            <StatusBar />
          </footer>
        </main>
        <CommandPalette />
        <FormulaAssist />
        <ContextMenu />
        <FilterPopover />
        <CellPopovers />
        <Dialogs />
        <ShortcutsModal />
        <PrintPreview />
        <W56Chrome />
        <Toasts />
        {about ? (
          <div className={styles.aboutOverlay} onMouseDown={() => setAbout(false)}>
            <div className={styles.about} role="dialog" aria-label="About Tally">
              <div className={styles.logo}>T</div>
              <div className={styles.aboutTitle}>Tally</div>
              <div className={styles.aboutText}>Version 0.3.0 (preview)</div>
              <div className={styles.aboutText}>A modern, OpenOffice-compatible spreadsheet.</div>
            </div>
          </div>
        ) : null}
      </div>
    </ControllerContext.Provider>
  );
}
