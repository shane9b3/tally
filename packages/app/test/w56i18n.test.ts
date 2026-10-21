/**
 * W5-6E: the UI language tables shipped with the app are copies of
 * `crates/tally-l10n/ui/*.json`, the function-name table matches the Rust
 * tables, every `t()` key used by the app exists in `en.json` (or is on the
 * documented pending list), and switching the language re-labels the chrome.
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

/**
 * `t()` keys the app uses that `crates/tally-l10n/ui/en.json` does not have
 * yet (the UI falls back to the English literal). Owner action: add them to
 * the four UI tables (crates/** is the engine agent's).
 */
const PENDING_KEYS = new Set([
  "chart.appearance", "chart.axes", "chart.closeEditor", "chart.data", "chart.editor", "chart.labels", "chart.statistics", "chart.titles",
  "cmd.chart.copyImage", "cmd.data.clearOutline", "cmd.data.createNamesFromLabels", "cmd.data.defineDatabaseRange", "cmd.data.resetFilter",
  "cmd.data.showOutlineLevel", "cmd.data.sortOptions", "cmd.edit.deleteContents", "cmd.edit.fillSelection", "cmd.edit.fillSeries",
  "cmd.edit.focusNameBox", "cmd.edit.repeat", "cmd.edit.selectArray", "cmd.edit.selectData", "cmd.file.clearRecent", "cmd.file.documentStats",
  "cmd.file.saveAsTemplate", "cmd.format.cells", "cmd.insert.image", "cmd.sheet.deleteCells", "cmd.sheet.hide", "cmd.sheet.insertCells",
  "cmd.sheet.show", "cmd.tools.markInvalid", "cmd.view.removeSplit", "cmd.view.showFormulas", "cmd.view.split", "cmd.view.valueHighlighting",
  "cmd.view.zeroValues", "cmd.view.zoomOptimal", "cmd.view.zoomPageWidth", "cmd.view.zoomWholePage",
]);

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

  it("every t() key in the app exists in en.json or is on the pending list", () => {
    const en = JSON.parse(read("crates/tally-l10n/ui/en.json")) as Record<string, string>;
    const used = new Set<string>();
    for (const file of sources(fileURLToPath(new URL("../src", import.meta.url)))) {
      for (const m of readFileSync(file, "utf8").matchAll(/\bt\(\s*"([A-Za-z0-9_.]+)"/g)) used.add(m[1] ?? "");
    }
    expect(used.size).toBeGreaterThan(150);
    const missing = [...used].filter((k) => !(k in en) && !PENDING_KEYS.has(k)).sort();
    expect(missing).toEqual([]);
    // Pending keys that have since been added should leave the list.
    expect([...PENDING_KEYS].filter((k) => k in en)).toEqual([]);
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
