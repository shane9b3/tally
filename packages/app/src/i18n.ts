/**
 * `t()`: the UI-string hook point (wave 5, W5-6E wires the real tables).
 *
 * Keys are the entries of `crates/tally-l10n/ui/en.json` (`cmd.<command id>`,
 * `menu.*`, `dialog.*`, `wizard.*`, `a11y.*`, …). Until the locale tables are
 * loaded, `t(key, fallback)` returns `fallback` (the English label written at
 * the call site), so nothing changes visibly; `setTranslations()` swaps in a
 * table and every later `t()` call reads it.
 *
 * `{name}` placeholders are filled from `params`.
 */

export type Translations = Readonly<Record<string, string>>;

let table: Translations = {};

/** Installs a flat `key → string` table (e.g. the parsed `de.json`); `{}` resets to English. */
export function setTranslations(next: Translations): void {
  table = next;
}

/** The installed table (tests). */
export function translations(): Translations {
  return table;
}

/** The translated string for `key`, else `fallback`, with `{param}` placeholders filled. */
export function t(key: string, fallback: string, params?: Readonly<Record<string, string | number>>): string {
  const raw = table[key] ?? fallback;
  if (!params) return raw;
  return raw.replace(/\{(\w+)\}/g, (m, name: string) => (name in params ? String(params[name]) : m));
}
