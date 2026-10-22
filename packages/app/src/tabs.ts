/**
 * Document tabs (desktop shell, `apps/desktop/src-tauri/src/windows.rs`).
 *
 * Every document runs in its own webview (`doc-<n>`, with its own stores and
 * `AppController`); several of them can share one native window (`win-<n>`)
 * as tabs, and only the active one is visible. The shell owns the list of
 * open documents: it hands it over in `window_init` and sends `tabs-changed`
 * to every document webview whenever something changes in any window (a
 * document opened, closed, moved, renamed, edited or shown). Each webview
 * lists them in the sidebar's "Open" section (`components/OpenDocuments.tsx`):
 * its own window's tabs first, then the other windows' documents. The
 * actions are shell commands naming a document by its label: `tab_activate`
 * and `tab_close` work for any open document (another window comes to the
 * front), `tab_move`, `tab_close_others` and `tab_detach` within this
 * window. The keyboard and Window-menu switching (next/previous, by number)
 * stays among this window's tabs, as in Safari, and works with the sidebar
 * hidden.
 *
 * The browser build (mock engine, e2e) has no shell; a dev-only `?tabs=<n>`
 * mock fills the store so the list can be looked at there.
 */
import type { TabInfo, TabsState } from "@tally/bridge/tauri";
import { create } from "zustand";
import type { AppController } from "./controller";

export type { TabInfo, TabsState };

/** Optional desktop-shell hooks for document tabs (`TauriClient`; the browser has none). */
export interface TabShell {
  /** `tab_activate`: show that tab of this webview's window. */
  tabActivate?(label: string): Promise<void>;
  /** `tab_close`: close that tab (the shell asks a dirty one first). */
  tabClose?(label: string): Promise<void>;
  /** `tab_close_others`: close every other tab of this window. */
  tabCloseOthers?(label: string): Promise<void>;
  /** `tab_move`: move that tab to `index` (0-based). */
  tabMove?(label: string, index: number): Promise<void>;
  /** `tab_detach`: Move Tab to New Window. */
  tabDetach?(label: string): Promise<void>;
  /** `merge_windows`: Merge All Windows into this window. */
  mergeWindows?(): Promise<void>;
  /** `tabs-changed`: the tabs of this window changed. */
  onTabsChanged?(cb: (state: TabsState) => void): () => void;
}

export interface TabsStoreState {
  /** This webview's own label (`doc-<n>`), once known. */
  self: string | null;
  /** The tabs of the window this webview is in. */
  state: TabsState | null;
  setSelf(self: string | null): void;
  setState(state: TabsState | null): void;
  reset(): void;
}

export const useTabsStore = create<TabsStoreState>()((set) => ({
  self: null,
  state: null,
  setSelf: (self) => set({ self }),
  setState: (state) => set({ state }),
  reset: () => set({ self: null, state: null }),
}));

/** Pure: every open document, this window's tabs first. */
export function openDocuments(state: TabsState | null): TabInfo[] {
  return state ? [...state.tabs, ...state.others] : [];
}

/**
 * Pure: when each edited document was first seen edited (`label → ms`):
 * kept while it stays edited, set to `now` when it becomes edited, dropped
 * when it is saved or closed.
 */
export function editedSince(prev: Readonly<Record<string, number>>, docs: readonly TabInfo[], now: number): Record<string, number> {
  const out: Record<string, number> = {};
  for (const d of docs) if (d.edited) out[d.label] = prev[d.label] ?? now;
  return out;
}

/** Pure: the tab `delta` steps from the active one, wrapping around; `null` with fewer than two tabs. */
export function cycleTab(state: TabsState | null, delta: number): string | null {
  if (!state || state.tabs.length < 2) return null;
  const n = state.tabs.length;
  const at = Math.max(0, state.tabs.findIndex((t) => t.label === state.active));
  return state.tabs[(((at + delta) % n) + n) % n]?.label ?? null;
}

/** Pure: the tab for ⌥⌘N / Alt+N: 1–8 by position, 9 is always the last tab (as in Safari). */
export function tabByNumber(state: TabsState | null, n: number): string | null {
  if (!state || state.tabs.length === 0 || n < 1 || n > 9) return null;
  const tab = n === 9 ? state.tabs[state.tabs.length - 1] : state.tabs[n - 1];
  return tab?.label ?? null;
}

/** Pure: Move Tab to New Window needs a second tab to leave behind. */
export function canDetach(state: TabsState | null): boolean {
  return (state?.tabs.length ?? 0) >= 2;
}

/** Pure: the index in this window's tabs for moving `label` before the tab `target` (or to the end with `null`); `null` when nothing moves or either is not in this window. */
export function moveIndex(state: TabsState, label: string, target: string | null): number | null {
  const from = state.tabs.findIndex((t) => t.label === label);
  if (from < 0) return null;
  let to = target === null ? state.tabs.length : state.tabs.findIndex((t) => t.label === target);
  if (to < 0) return null;
  // Removing the dragged tab first shifts the later positions left by one.
  if (to > from) to -= 1;
  return to === from ? null : to;
}

/** Pure: the tooltip of an open document (its file, else its title). */
export function tabTooltip(tab: TabInfo, untitled: string): string {
  return tab.path ?? (tab.title || untitled);
}

/** `ctl.tabs`: the tab actions, each a shell command (no-ops without a tab shell). */
export class TabsFeature {
  constructor(private readonly ctl: AppController) {}

  private get shell(): TabShell {
    return this.ctl.shell as unknown as TabShell;
  }

  /** Whether the desktop shell has document tabs. */
  get available(): boolean {
    return typeof this.shell.tabActivate === "function";
  }

  private get state(): TabsState | null {
    return useTabsStore.getState().state;
  }

  private run(p: Promise<void> | undefined): Promise<void> {
    return (p ?? Promise.resolve()).catch(this.ctl.notifyError);
  }

  activate = (label: string): Promise<void> => this.run(this.shell.tabActivate?.(label));
  close = (label: string): Promise<void> => this.run(this.shell.tabClose?.(label));
  closeOthers = (label: string): Promise<void> => this.run(this.shell.tabCloseOthers?.(label));
  move = (label: string, index: number): Promise<void> => this.run(this.shell.tabMove?.(label, index));
  detach = (label: string): Promise<void> => this.run(this.shell.tabDetach?.(label));
  merge = (): Promise<void> => this.run(this.shell.mergeWindows?.());

  /** The visible tab (this webview, while it has the keyboard). */
  private get current(): string | null {
    return this.state?.active ?? useTabsStore.getState().self;
  }

  next = (): Promise<void> => this.go(cycleTab(this.state, 1));
  prev = (): Promise<void> => this.go(cycleTab(this.state, -1));
  select = (n: number): Promise<void> => this.go(tabByNumber(this.state, n));

  /** Window ▸ Move Tab to New Window for the visible tab. */
  detachCurrent = (): Promise<void> => {
    const label = this.current;
    return label && canDetach(this.state) ? this.detach(label) : Promise.resolve();
  };

  private go(label: string | null): Promise<void> {
    return label && label !== this.state?.active ? this.activate(label) : Promise.resolve();
  }
}
