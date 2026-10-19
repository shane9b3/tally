/** Desktop-only wiring: TauriClient, the shell adapter and the window title. */
import { useDocStore, type AppController, type ShellApi } from "@tally/app";
import { TauriClient } from "@tally/bridge/tauri";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function makeTauriClient(): TauriClient {
  return new TauriClient({ invoke, listen });
}

export function createTauriShell(client: TauriClient): ShellApi {
  return {
    native: true,
    openDialog: () => client.openDialog(),
    openRecent: (path) => client.open(path),
    save: (doc, saveAs, format) => client.saveWithDialog(doc, saveAs, format),
    recentFiles: () => client.recentFiles(),
    listTemplates: () => client.listTemplates(),
    newFromTemplate: (id) => client.newWorkbook(id),
  };
}

/** Mirrors the document title (and dirty state) into the native window title. */
export function setWindowTitle(_ctl: AppController): void {
  let last = "";
  const update = (): void => {
    const info = useDocStore.getState().info;
    const title = info ? `${info.title}${info.dirty ? " — Edited" : ""}` : "Tally";
    if (title === last) return;
    last = title;
    document.title = title;
    getCurrentWindow()
      .setTitle(title)
      .catch(() => undefined);
  };
  update();
  useDocStore.subscribe(update);
}
