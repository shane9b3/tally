/**
 * Web entry: mounts @tally/app with `TauriClient` inside the desktop app and
 * `MockClient` in a plain browser (`pnpm dev:web`, e2e).
 */
import { App, createBrowserShell, type AppController, type BrowserTemplate, type ShellApi } from "@tally/app";
import { MockClient, type EngineClient } from "@tally/bridge";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createTauriShell, isTauri, makeTauriClient, setWindowTitle } from "./tauri";
import budget from "../src-tauri/templates/budget.csv?raw";
import invoices from "../src-tauri/templates/invoices.csv?raw";
import stock from "../src-tauri/templates/stock.csv?raw";

const BROWSER_TEMPLATES: BrowserTemplate[] = [
  { id: "invoices", title: "Invoice tracker", description: "Customers, amounts and payment status", csv: invoices },
  { id: "budget", title: "Monthly budget", description: "Planned versus actual spending", csv: budget },
  { id: "stock", title: "Stock count", description: "Items on hand and reorder levels", csv: stock },
];

const tauri = isTauri();
const mac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
const params = new URLSearchParams(location.search);

let client: EngineClient;
let shell: ShellApi;
if (tauri) {
  const tc = makeTauriClient();
  client = tc;
  shell = createTauriShell(tc);
} else {
  client = new MockClient();
  shell = createBrowserShell(client, BROWSER_TEMPLATES);
}

declare global {
  interface Window {
    /** e2e/dev hook (browser builds only). */
    __tally?: { client: EngineClient; controller: AppController };
  }
}

function onReady(ctl: AppController): void {
  if (!tauri) window.__tally = { client, controller: ctl };
  if (tauri) setWindowTitle(ctl);
}

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App
        client={client}
        shell={shell}
        initialTemplate={params.get("template")}
        platform={{ trafficLights: tauri && mac, nativeMenu: tauri }}
        onReady={onReady}
      />
    </StrictMode>,
  );
}
