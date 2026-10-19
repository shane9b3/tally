/** UI-only state: sidebar, panels, palette, theme, zoom, toasts, menus. Never undoable. */
import type { CellPos } from "@tally/bridge";
import type { ContextArea } from "@tally/grid";
import { create } from "zustand";
import { clampZoom, nextZoom } from "../formats";
import {
  applyThemeAttr,
  nextThemePref,
  readThemePref,
  resolveTheme,
  systemTheme,
  writeThemePref,
  type ResolvedTheme,
  type ThemePref,
} from "../theme/theme";

export interface Toast {
  id: number;
  kind: "error" | "info";
  message: string;
}

/** An open right-click menu. */
export interface ContextMenuState {
  x: number;
  y: number;
  area: ContextArea;
  cell: CellPos;
}

/** A small modal dialog. */
export type DialogState =
  | { kind: "resize"; axis: "col" | "row"; start: number; end: number; px: number }
  | { kind: "insert"; axis: "row" | "col"; where: "before" | "after" }
  | { kind: "numberFormat"; code: string };

export interface UiState {
  sidebarOpen: boolean;
  paletteOpen: boolean;
  /** "At a glance" right panel. */
  glanceOpen: boolean;
  /** Keyboard shortcut reference (⌘/). */
  shortcutsOpen: boolean;
  themePref: ThemePref;
  theme: ResolvedTheme;
  zoom: number;
  toasts: Toast[];
  contextMenu: ContextMenuState | null;
  dialog: DialogState | null;
  toggleSidebar(): void;
  setSidebar(open: boolean): void;
  setPalette(open: boolean): void;
  setGlance(open: boolean): void;
  toggleGlance(): void;
  setShortcuts(open: boolean): void;
  setThemePref(p: ThemePref): void;
  cycleTheme(): void;
  /** Re-resolves "system" after a `prefers-color-scheme` change. */
  syncSystemTheme(): void;
  setZoom(z: number): void;
  zoomBy(dir: 1 | -1): void;
  pushToast(kind: Toast["kind"], message: string): number;
  dismissToast(id: number): void;
  openContextMenu(m: ContextMenuState | null): void;
  openDialog(d: DialogState | null): void;
  reset(): void;
}

let nextToast = 1;
/** Toasts kept on screen at once. */
export const MAX_TOASTS = 3;

const initialPref = readThemePref();
const initialTheme = resolveTheme(initialPref);
applyThemeAttr(initialTheme);

export const useUiStore = create<UiState>()((set, get) => ({
  sidebarOpen: true,
  paletteOpen: false,
  glanceOpen: false,
  shortcutsOpen: false,
  themePref: initialPref,
  theme: initialTheme,
  zoom: 1,
  toasts: [],
  contextMenu: null,
  dialog: null,
  toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
  setSidebar: (sidebarOpen) => set({ sidebarOpen }),
  setPalette: (paletteOpen) => set({ paletteOpen }),
  setGlance: (glanceOpen) => set({ glanceOpen }),
  toggleGlance: () => set((s) => ({ glanceOpen: !s.glanceOpen })),
  setShortcuts: (shortcutsOpen) => set({ shortcutsOpen }),
  setThemePref: (themePref) => {
    writeThemePref(themePref);
    const theme = resolveTheme(themePref);
    applyThemeAttr(theme);
    set({ themePref, theme });
  },
  cycleTheme: () => get().setThemePref(nextThemePref(get().themePref)),
  syncSystemTheme: () => {
    const { themePref } = get();
    const theme = resolveTheme(themePref, systemTheme());
    applyThemeAttr(theme);
    if (theme !== get().theme) set({ theme });
  },
  setZoom: (z) => set({ zoom: clampZoom(z) }),
  zoomBy: (dir) => set((s) => ({ zoom: nextZoom(s.zoom, dir) })),
  pushToast: (kind, message) => {
    const id = nextToast++;
    set((s) => {
      // Collapse repeats of the same message.
      const rest = s.toasts.filter((t) => t.message !== message);
      return { toasts: [...rest, { id, kind, message }].slice(-MAX_TOASTS) };
    });
    return id;
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  openContextMenu: (contextMenu) => set({ contextMenu }),
  openDialog: (dialog) => set({ dialog }),
  reset: () =>
    set({ sidebarOpen: true, paletteOpen: false, glanceOpen: false, shortcutsOpen: false, zoom: 1, toasts: [], contextMenu: null, dialog: null }),
}));

/**
 * Shows the engine's one-shot document warnings (`WorkbookInfo.warnings`,
 * e.g. "Recalculation stopped after N formulas…" on open) as info toasts.
 */
export function toastWarnings(warnings: readonly string[] | undefined): void {
  for (const w of warnings ?? []) useUiStore.getState().pushToast("info", w);
}
