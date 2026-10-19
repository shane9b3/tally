/** UI-only state: sidebar, palette, zoom, toasts. Never undoable. */
import { create } from "zustand";
import { clampZoom, nextZoom } from "../formats";

export interface Toast {
  id: number;
  kind: "error" | "info";
  message: string;
}

export interface UiState {
  sidebarOpen: boolean;
  paletteOpen: boolean;
  zoom: number;
  toasts: Toast[];
  toggleSidebar(): void;
  setSidebar(open: boolean): void;
  setPalette(open: boolean): void;
  setZoom(z: number): void;
  zoomBy(dir: 1 | -1): void;
  pushToast(kind: Toast["kind"], message: string): number;
  dismissToast(id: number): void;
  reset(): void;
}

let nextToast = 1;
/** Toasts kept on screen at once. */
export const MAX_TOASTS = 3;

export const useUiStore = create<UiState>()((set) => ({
  sidebarOpen: true,
  paletteOpen: false,
  zoom: 1,
  toasts: [],
  toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
  setSidebar: (sidebarOpen) => set({ sidebarOpen }),
  setPalette: (paletteOpen) => set({ paletteOpen }),
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
  reset: () => set({ sidebarOpen: true, paletteOpen: false, zoom: 1, toasts: [] }),
}));
