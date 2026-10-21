/**
 * Display-level localisation of function names (§4 item 47, UI part): when
 * the document locale is de/fr/es the formula editor, the autocomplete
 * list and the Function Wizard show localized names (`SUMME`, `SOMME`,
 * `SUMA`) and the UI translates them back to the canonical names the engine
 * parses when a formula is committed. `functionNames.json` is generated
 * from `crates/tally-l10n/src/names/*.rs` (a vitest checks the sync).
 */
import type { FunctionInfo } from "@tally/bridge";
import tables from "./functionNames.json";

export type FormulaLang = "de" | "fr" | "es";

const TABLES = tables as Readonly<Record<FormulaLang, Readonly<Record<string, string>>>>;

/** The language of a document locale tag, or `null` for English and unknown locales. */
export function formulaLangOf(tag: string): FormulaLang | null {
  const lang = tag.split(/[-_]/)[0]?.toLowerCase();
  return lang === "de" || lang === "fr" || lang === "es" ? lang : null;
}

const reverse = new Map<FormulaLang, Map<string, string>>();

function reverseTable(lang: FormulaLang): Map<string, string> {
  let m = reverse.get(lang);
  if (!m) {
    m = new Map(Object.entries(TABLES[lang]).map(([canonical, local]) => [local.toUpperCase(), canonical]));
    reverse.set(lang, m);
  }
  return m;
}

/** The localized spelling of a canonical name (the name itself when the locale keeps it). */
export function localizedName(lang: FormulaLang, canonical: string): string {
  return TABLES[lang][canonical.toUpperCase()] ?? canonical;
}

/** The canonical name of a localized (or English) spelling, or `null`. */
export function canonicalName(lang: FormulaLang, name: string): string | null {
  return reverseTable(lang).get(name.toUpperCase()) ?? null;
}

/**
 * Rewrites the function names of a formula (identifiers followed by `(`,
 * outside string literals) with `map`; anything `map` does not know stays.
 */
export function mapFunctionNames(formula: string, map: (name: string) => string | null): string {
  if (!formula.startsWith("=")) return formula;
  let out = "";
  let i = 0;
  while (i < formula.length) {
    const ch = formula[i] ?? "";
    if (ch === '"') {
      const end = formula.indexOf('"', i + 1);
      const stop = end < 0 ? formula.length : end + 1;
      out += formula.slice(i, stop);
      i = stop;
      continue;
    }
    const m = /^[A-Za-z_][A-Za-z0-9_.À-ÖØ-öø-ÿÑñ]*(?=\s*\()/.exec(formula.slice(i));
    if (m && (i === 0 || !/[A-Za-z0-9_.$]/.test(formula[i - 1] ?? "")) && !/^[A-Za-z]{1,3}\d+\./.test(m[0])) {
      const name = m[0];
      out += map(name) ?? name;
      i += name.length;
      continue;
    }
    out += ch;
    i++;
  }
  return out;
}

/** Canonical formula → the locale's display spelling. */
export function toDisplayFormula(lang: FormulaLang | null, formula: string): string {
  if (!lang) return formula;
  return mapFunctionNames(formula, (n) => (TABLES[lang][n.toUpperCase()] ? localizedName(lang, n) : null));
}

/** Display spelling → the canonical formula the engine parses. */
export function toCanonicalFormula(lang: FormulaLang | null, formula: string): string {
  if (!lang) return formula;
  return mapFunctionNames(formula, (n) => canonicalName(lang, n));
}

/** The function list with localized names and signatures (for the autocomplete and the wizard). */
export function localizeFunctions(lang: FormulaLang | null, functions: readonly FunctionInfo[]): FunctionInfo[] {
  if (!lang) return [...functions];
  return functions.map((f) => {
    const name = localizedName(lang, f.name);
    return name === f.name ? f : { ...f, name, signature: f.signature.replace(new RegExp(`^${f.name}(?=\\()`), name) };
  });
}
