/**
 * W5-6E: the UI language tables shipped with the app are copies of
 * `crates/tally-l10n/ui/*.json`, the function-name table matches the Rust
 * tables, every `t()` key used by the app exists in `en.json`, and
 * switching the language re-labels the chrome.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { t } from "../src/i18n";
import { KIND_KEYS, setUiLanguage, uiLanguage } from "../src/i18n/locales";
import functionNames from "../src/i18n/functionNames.json";
import { useW56Store } from "../src/data/store";

const ROOT = fileURLToPath(new URL("../../../", import.meta.url));
const read = (p: string) => readFileSync(join(ROOT, p), "utf8");

function sources(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...sources(p));
    else if (/\.tsx?$/.test(name)) out.push(p);
  }
  return out;
}

describe("UI language tables", () => {
  it("ship as exact copies of the crate's ui/*.json", () => {
    for (const lang of ["de", "fr", "es"]) {
      const crate = JSON.parse(read(`crates/tally-l10n/ui/${lang}.json`)) as Record<string, string>;
      const app = JSON.parse(read(`packages/app/src/i18n/${lang}.json`)) as Record<string, string>;
      expect(app, lang).toEqual(crate);
    }
    const en = JSON.parse(read("crates/tally-l10n/ui/en.json")) as Record<string, string>;
    expect(Object.keys(en).length).toBeGreaterThanOrEqual(600);
  });

  it("function-name tables match crates/tally-l10n/src/names/*.rs", () => {
    for (const lang of ["de", "fr", "es"] as const) {
      const rs = read(`crates/tally-l10n/src/names/${lang}.rs`);
      const body = /pub const NAMES[^=]*=\s*&\[([\s\S]*?)\];/.exec(rs)?.[1] ?? "";
      const pairs = [...body.matchAll(/\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)/g)].map((m) => [m[1], m[2]]);
      expect(pairs.length, lang).toBeGreaterThan(300);
      expect(functionNames[lang], lang).toEqual(Object.fromEntries(pairs));
    }
  });

  it("every t() key in the app exists in en.json", () => {
    const en = JSON.parse(read("crates/tally-l10n/ui/en.json")) as Record<string, string>;
    const used = new Set<string>();
    for (const file of sources(fileURLToPath(new URL("../src", import.meta.url)))) {
      for (const m of readFileSync(file, "utf8").matchAll(/\bt\(\s*"([A-Za-z0-9_.]+)"/g)) used.add(m[1] ?? "");
    }
    expect(used.size).toBeGreaterThan(150);
    const missing = [...used].filter((k) => !(k in en)).sort();
    expect(missing).toEqual([]);
    for (const key of Object.values(KIND_KEYS)) expect(key in en, key).toBe(true);
  });

  it("switching to German re-labels menus and bumps the revision", async () => {
    const rev = useW56Store.getState().i18nRev;
    await setUiLanguage("de");
    expect(uiLanguage()).toBe("de");
    expect(t("menu.file", "File")).toBe("Datei");
    expect(t("menu.edit", "Edit")).toBe("Bearbeiten");
    expect(t("cmd.edit.undo", "Undo")).toBe("Rückgängig");
    expect(useW56Store.getState().i18nRev).toBe(rev + 1);
    await setUiLanguage("fr");
    expect(t("dialog.cancel", "Cancel")).toBe("Annuler");
    await setUiLanguage("en");
    expect(t("menu.file", "File")).toBe("File");
  });
});
