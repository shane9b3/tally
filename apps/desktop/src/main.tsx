/**
 * Web entry: mounts @tally/app with `TauriClient` inside the desktop app and
 * `MockClient` in a plain browser (`pnpm dev:web`, e2e). Both implement
 * `EngineClient` and `ShellClient`; templates are built by the engine (or
 * the mock) from their id.
 */
import { App, WebMockClient, type AppController } from "@tally/app";
import type { EngineClientV2, ShellClient } from "@tally/bridge";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { isTauri, makeTauriClient, setWindowTitle } from "./tauri";

const tauri = isTauri();
const mac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
const params = new URLSearchParams(location.search);

// The browser build uses MockClient plus emulated row/column insert/delete.
const client: EngineClientV2 & ShellClient = tauri ? makeTauriClient() : new WebMockClient();

declare global {
  interface Window {
    /** e2e/dev hook (browser builds only). */
    __tally?: { client: EngineClientV2 & { recorded?: unknown[] }; controller: AppController };
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
        initialTemplate={params.get("template")}
        platform={{ trafficLights: tauri && mac, nativeMenu: tauri }}
        onReady={onReady}
      />
    </StrictMode>,
  );
}
