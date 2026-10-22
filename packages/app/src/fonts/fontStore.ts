/**
 * The font list of the pickers: the installed families (`list_fonts`,
 * loaded once per window and on demand; the browser build gets
 * `FALLBACK_FONTS`) and the recently used fonts (per user, in
 * localStorage, like the sidebar's per-viewer data).
 */
import { FALLBACK_FONTS, type FontFamilyInfo } from "@tally/bridge";
import { create } from "zustand";
import { MAX_RECENT_FONTS, cleanFontName, withRecent } from "./fontModel";

/** The shell command used here (`TauriClient.listFonts`, `MockClient.listFonts`). */
export interface FontShell {
  listFonts?(refresh?: boolean): Promise<FontFamilyInfo[]>;
}

const RECENT_KEY = "tally.recentFonts";

export function readRecentFonts(): string[] {
  try {
    const raw = typeof localStorage === "undefined" ? null : localStorage.getItem(RECENT_KEY);
    const v: unknown = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(v)) return [];
    return v.filter((x): x is string => typeof x === "string").map(cleanFontName).filter((x) => x !== "").slice(0, MAX_RECENT_FONTS);
  } catch {
    return [];
  }
}

function writeRecentFonts(list: readonly string[]): void {
  try {
    if (typeof localStorage !== "undefined") localStorage.setItem(RECENT_KEY, JSON.stringify(list));
  } catch {
    // Storage unavailable: the list lives for the session.
  }
}

interface FontState {
  families: FontFamilyInfo[];
  /** `list_fonts` answered (or failed: the fallback list is used). */
  loaded: boolean;
  recent: string[];
  /** Loads the installed fonts once (again with `refresh`, rescanning the font directories). */
  load(shell: FontShell, refresh?: boolean): Promise<void>;
  /** Puts `name` at the top of the recently used fonts. */
  remember(name: string): void;
}

/**
 * Single flight: one list request at a time per kind. A load while one is
 * running joins it; a refresh while a refresh runs joins that one; a
 * refresh while a plain load runs starts after it, and only the newest
 * request's answer is kept (a slow plain load cannot overwrite a refresh).
 */
let inflight: { p: Promise<void>; refresh: boolean } | null = null;
let seq = 0;

export const useFontStore = create<FontState>()((set, get) => ({
  families: [...FALLBACK_FONTS],
  loaded: false,
  recent: readRecentFonts(),
  load(shell, refresh = false) {
    if (inflight && (!refresh || inflight.refresh)) return inflight.p;
    if (get().loaded && !refresh) return Promise.resolve();
    const before = inflight?.p ?? Promise.resolve();
    const mine = ++seq;
    const p = (async () => {
      await before.catch(() => undefined);
      let families: FontFamilyInfo[] = [...FALLBACK_FONTS];
      try {
        if (shell.listFonts) {
          const list = await shell.listFonts(refresh);
          if (Array.isArray(list) && list.length > 0) families = list;
        }
      } catch {
        // Keep the fallback list.
      }
      if (mine === seq) set({ families, loaded: true });
    })();
    const entry = { p, refresh };
    inflight = entry;
    void p.finally(() => {
      if (inflight === entry) inflight = null;
    });
    return p;
  },
  remember(name) {
    const recent = withRecent(get().recent, name);
    writeRecentFonts(recent);
    set({ recent });
  },
}));
