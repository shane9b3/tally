/**
 * Web entry: mounts @tally/app with `TauriClient` inside the desktop app and
 * `MockClient` in a plain browser (`pnpm dev:web`, e2e). Both implement
 * `EngineClient` and `ShellClient`; templates are built by the engine (or
 * the mock) from their id.
 */
import { App, type AppController } from "@tally/app";
import { MockClient, type EngineClient, type ShellClient } from "@tally/bridge";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { isTauri, makeTauriClient, setWindowTitle } from "./tauri";

const tauri = isTauri();
const mac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
const params = new URLSearchParams(location.search);

const client: EngineClient & ShellClient = tauri ? makeTauriClient() : new MockClient();

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
        initialTemplate={params.get("template")}
        platform={{ trafficLights: tauri && mac, nativeMenu: tauri }}
        onReady={onReady}
      />
    </StrictMode>,
  );
}
