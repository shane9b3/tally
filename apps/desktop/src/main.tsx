/**
 * Web entry: mounts @tally/app with `TauriClient` inside the desktop app.
 * In a plain browser (`pnpm dev:web`, e2e) a dev build loads `WebMockClient`
 * through a dynamic `import()` of `@tally/app/dev`, so the production bundle
 * contains neither the mock engine nor the `window.__tally` hook (security
 * review 3, MEDIUM-7; `scripts/check-bundle.sh` verifies the built output).
 * A production bundle opened outside Tauri shows a short notice instead.
 */
import { App, type AppController, type MenuItemState } from "@tally/app";
import type { EngineClientV3, ShellClient } from "@tally/bridge";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { isTauri, makeTauriClient, makeTitleBarShell, setWindowTitle } from "./tauri";
import { TitleBar, type TitleBarShell } from "./TitleBar";

type Client = EngineClientV3 & ShellClient & { recorded?: unknown[] };

const tauri = isTauri();
const mac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
const params = new URLSearchParams(location.search);

declare global {
  interface Window {
    /** e2e/dev hook (dev browser builds only). */
    __tally?: { client: Client; controller: AppController; titleBar?: TitleBarShell & { activated: string[]; windowCalls: string[] } };
  }
}

async function pickClient(): Promise<Client | null> {
  if (tauri) return makeTauriClient();
  if (import.meta.env.DEV) {
    // The browser build uses MockClient plus emulated row/column insert/delete.
    const dev = await import("@tally/app/dev");
    const client = new dev.WebMockClient();
    // `?tabs=<n>`: a mock tab shell, to see and drive the sidebar's "Open" list.
    const tabs = Number(params.get("tabs") ?? "0");
    if (tabs > 0) dev.installMockTabs(client, tabs);
    // `?readonly=1`: workbooks open like files flagged read-only (e2e).
    if (params.get("readonly") === "1") client.readOnlyHooks.openReadOnly = true;
    // `?macros=1`: workbooks open like an `.xlsm` with macros (e2e).
    if (params.get("macros") === "1") client.macroHooks.openWithMacros = true;
    // `?update=<version>`: the simulated update server offers that release (e2e).
    const update = params.get("update");
    if (update) client.updateHooks.available = { version: update, notes: params.get("updateNotes") ?? "Faster recalculation.\nFixes:\n- <b>Save As</b> keeps the format", date: "2026-11-20" };
    if (params.get("updateError")) client.updateHooks.checkError = params.get("updateError");
    return client;
  }
  return null;
}

/**
 * Windows (W5-5, §4 item 48): the custom title bar (with the menu bar)
 * replaces the OS decorations. `?titlebar=1` in a dev browser build mounts
 * it over mocks (e2e).
 */
async function titleBarShell(client: Client): Promise<TitleBarShell | null> {
  if (tauri) {
    const chrome = (client as { windowChrome?: () => Promise<{ customTitleBar: boolean }> }).windowChrome;
    const custom = chrome ? await chrome().then((c) => c.customTitleBar, () => false) : false;
    return custom ? makeTitleBarShell() : null;
  }
  if (import.meta.env.DEV && params.get("titlebar") === "1") {
    const dev = await import("@tally/app/dev");
    const menu = dev.mockMenuShell(client as unknown as { emitMenuAction(id: string): void });
    const windowCalls: string[] = [];
    let maximized = false;
    const call = (name: string) => () => Promise.resolve(void windowCalls.push(name));
    const shell = Object.assign(menu, {
      windowCalls,
      minimize: call("minimize"),
      close: call("close"),
      toggleMaximize: () => {
        maximized = !maximized;
        windowCalls.push("toggleMaximize");
        window.dispatchEvent(new Event("resize"));
        return Promise.resolve();
      },
      isMaximized: () => Promise.resolve(maximized),
    });
    devTitleBar = shell;
    return shell;
  }
  return null;
}
let devTitleBar: (TitleBarShell & { activated: string[]; windowCalls: string[] }) | undefined;

function mount(client: Client, titleBar: TitleBarShell | null): void {
  let controller: AppController | null = null;
  const onReady = (ctl: AppController): void => {
    controller = ctl;
    if (import.meta.env.DEV && !tauri) window.__tally = { client, controller: ctl, titleBar: devTitleBar };
    if (tauri) setWindowTitle(ctl, mac);
  };
  // The menu shows what the command registry knows: a command that is off now, and why.
  const itemState = (id: string): MenuItemState => {
    const registry = controller?.registry;
    if (!registry?.has(id) || registry.isEnabled(id)) return { enabled: true };
    return { enabled: false, reason: registry.reasonFor(id) };
  };
  const root = document.getElementById("root");
  if (!root) return;
  if (titleBar) root.dataset.titlebar = "custom";
  // With the title bar's menu there is no native menu: the webview's keymap owns every shortcut.
  const app = <App client={client} initialTemplate={params.get("template")} platform={{ trafficLights: tauri && mac, nativeMenu: tauri && !titleBar }} onReady={onReady} />;
  createRoot(root).render(
    <StrictMode>
      {titleBar ? (
        <>
          <TitleBar shell={titleBar} itemState={itemState} />
          <div className="tally-app-host">{app}</div>
        </>
      ) : (
        app
      )}
    </StrictMode>,
  );
}

void pickClient().then(async (client) => {
  if (client) {
    mount(client, await titleBarShell(client));
    return;
  }
  const root = document.getElementById("root");
  if (root) root.textContent = "Tally runs inside the desktop app.";
});
