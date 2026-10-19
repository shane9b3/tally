/**
 * Theme preference (system / light / dark), persisted per viewer in
 * localStorage, applied as `data-theme` on <html>. "system" follows
 * `prefers-color-scheme`. The canvas grid re-reads tokens when the resolved
 * theme changes (GridHost keys `readTheme()` on it).
 */
export type ThemePref = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

export const THEME_KEY = "tally.theme";
export const THEME_ORDER: readonly ThemePref[] = ["system", "light", "dark"];

/** The next preference in the View → Theme cycle. */
export function nextThemePref(p: ThemePref): ThemePref {
  return THEME_ORDER[(THEME_ORDER.indexOf(p) + 1) % THEME_ORDER.length] ?? "system";
}

export function readThemePref(): ThemePref {
  try {
    const v = localStorage.getItem(THEME_KEY);
    return v === "light" || v === "dark" || v === "system" ? v : "system";
  } catch {
    return "system";
  }
}

export function writeThemePref(p: ThemePref): void {
  try {
    localStorage.setItem(THEME_KEY, p);
  } catch {
    // Storage unavailable: the preference lasts for this session only.
  }
}

export function systemTheme(): ResolvedTheme {
  try {
    return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  } catch {
    return "light";
  }
}

export function resolveTheme(p: ThemePref, system: ResolvedTheme = systemTheme()): ResolvedTheme {
  return p === "system" ? system : p;
}

/** Sets `data-theme` on the document element (no-op outside a browser). */
export function applyThemeAttr(t: ResolvedTheme): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = t;
}
