/**
 * Web entry: mounts @tally/app with `TauriClient` inside the desktop app.
 * In a plain browser (`pnpm dev:web`, e2e) a dev build loads `WebMockClient`
 * through a dynamic `import()` of `@tally/app/dev`, so the production bundle
 * contains neither the mock engine nor the `window.__tally` hook (security
 * review 3, MEDIUM-7; `scripts/check-bundle.sh` verifies the built output).
 * A production bundle opened outside Tauri shows a short notice instead.
 */
import { App, type AppController } from "@tally/app";
import type { EngineClientV3, ShellClient } from "@tally/bridge";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { isTauri, makeTauriClient, setWindowTitle } from "./tauri";
import { TitleBar } from "./TitleBar";

type Client = EngineClientV3 & ShellClient & { recorded?: unknown[] };

const tauri = isTauri();
const mac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
const params = new URLSearchParams(location.search);

declare global {
  interface Window {
    /** e2e/dev hook (dev browser builds only). */
    __tally?: { client: Client; controller: AppController };
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
    return client;
  }
  return null;
}

/** Windows (W5-5, §4 item 48): the custom title bar replaces the OS decorations. */
async function customTitleBar(client: Client): Promise<boolean> {
  if (!tauri) return false;
  const chrome = (client as { windowChrome?: () => Promise<{ customTitleBar: boolean }> }).windowChrome;
  return chrome ? await chrome().then((c) => c.customTitleBar, () => false) : false;
}

function mount(client: Client, titleBar = false): void {
  const onReady = (ctl: AppController): void => {
    if (import.meta.env.DEV && !tauri) window.__tally = { client, controller: ctl };
    if (tauri) setWindowTitle(ctl, mac);
  };
  const root = document.getElementById("root");
  if (!root) return;
  if (titleBar) root.dataset.titlebar = "custom";
  const app = <App client={client} initialTemplate={params.get("template")} platform={{ trafficLights: tauri && mac, nativeMenu: tauri }} onReady={onReady} />;
  createRoot(root).render(
    <StrictMode>
      {titleBar ? (
        <>
          <TitleBar />
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
    mount(client, await customTitleBar(client));
    return;
  }
  const root = document.getElementById("root");
  if (root) root.textContent = "Tally runs inside the desktop app.";
});
