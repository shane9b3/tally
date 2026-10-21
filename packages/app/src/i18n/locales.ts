/**
 * UI language tables (W5-6E): `de.json`, `fr.json` and `es.json` are copies
 * of `crates/tally-l10n/ui/*.json` (a vitest keeps them in sync). Loading a
 * language installs its table through `setTranslations()` and bumps the
 * `i18nRev` so components re-read `t()`; the desktop shell also rebuilds
 * the native menu (`set_ui_language`).
 */
import { setTranslations, type Translations } from "../i18n";
import type { UiLanguage } from "../options/prefs";
import { useW56Store } from "../data/store";

const loaders: Record<Exclude<UiLanguage, "en">, () => Promise<{ default: Record<string, string> }>> = {
  de: () => import("./de.json"),
  fr: () => import("./fr.json"),
  es: () => import("./es.json"),
};

const cache = new Map<UiLanguage, Translations>();
let current: UiLanguage = "en";

/** The installed UI language. */
export function uiLanguage(): UiLanguage {
  return current;
}

/** Loads (once) and installs the table of `lang`; `en` resets to the fallbacks. */
export async function setUiLanguage(lang: UiLanguage): Promise<void> {
  let table: Translations = {};
  if (lang !== "en") {
    const cached = cache.get(lang);
    if (cached) table = cached;
    else {
      const mod = await loaders[lang]();
      table = mod.default;
      cache.set(lang, table);
    }
  }
  current = lang;
  setTranslations(table);
  useW56Store.getState().bumpI18n();
}

/** The `t()` key of a palette / menu group (`kind`) so group labels follow the UI language. */
export const KIND_KEYS: Readonly<Record<string, string>> = {
  File: "menu.file",
  Edit: "menu.edit",
  View: "menu.view",
  Insert: "menu.insert",
  Format: "menu.format",
  Sheet: "menu.sheet",
  Data: "menu.data",
  Tools: "menu.tools",
  Help: "menu.help",
  Filter: "menu.data.filter",
  Chart: "chart.title",
  Sort: "sort.title",
  Formula: "toolbar.formula",
};
