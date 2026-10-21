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
 * outside string literals and `'quoted sheet names'`) with `map`; anything
 * `map` does not know stays. With `numberSep`, the decimal separator of
 * numeric literals (`1,5` / `1.5`) is swapped `from` -> `to` as well.
 */
export function mapFunctionNames(formula: string, map: (name: string) => string | null, numberSep?: { from: string; to: string }): string {
  if (!formula.startsWith("=")) return formula;
  let out = "";
  let i = 0;
  const skipQuoted = (q: string): void => {
    // Doubled quotes escape the quote character inside both literal kinds.
    let j = i + 1;
    for (;;) {
      const end = formula.indexOf(q, j);
      if (end < 0) {
        j = formula.length;
        break;
      }
      if (formula[end + 1] === q) {
        j = end + 2;
        continue;
      }
      j = end + 1;
      break;
    }
    out += formula.slice(i, j);
    i = j;
  };
  while (i < formula.length) {
    const ch = formula[i] ?? "";
    if (ch === '"' || ch === "'") {
      skipQuoted(ch);
      continue;
    }
    const prev = i === 0 ? "" : (formula[i - 1] ?? "");
    const m = /^[\p{L}_][\p{L}\p{N}_.]*(?=\s*\()/u.exec(formula.slice(i));
    if (m && !/[\p{L}\p{N}_.$]/u.test(prev) && !/^[A-Za-z]{1,3}\d+\./.test(m[0])) {
      const name = m[0];
      out += map(name) ?? name;
      i += name.length;
      continue;
    }
    if (numberSep && /\d/.test(ch) && !/[\p{L}\p{N}_.$]/u.test(prev)) {
      const from = numberSep.from === "." ? "\\." : numberSep.from;
      const n = new RegExp(`^\\d+${from}\\d+`).exec(formula.slice(i));
      if (n) {
        out += n[0].replace(numberSep.from, numberSep.to);
        i += n[0].length;
        continue;
      }
      // A plain integer: consume it whole so its tail digits are not mistaken for a start.
      const d = /^\d+/.exec(formula.slice(i))![0];
      out += d;
      i += d.length;
      continue;
    }
    out += ch;
    i++;
  }
  return out;
}

/** Canonical formula → the locale's display spelling (names and decimal comma). */
export function toDisplayFormula(lang: FormulaLang | null, formula: string): string {
  if (!lang) return formula;
  return mapFunctionNames(formula, (n) => (TABLES[lang][n.toUpperCase()] ? localizedName(lang, n) : null), { from: ".", to: "," });
}

/** Display spelling → the canonical formula the engine parses (`;` arguments, `.` decimals). */
export function toCanonicalFormula(lang: FormulaLang | null, formula: string): string {
  if (!lang) return formula;
  return mapFunctionNames(formula, (n) => canonicalName(lang, n), { from: ",", to: "." });
}

/** The function list with localized names and signatures (for the autocomplete and the wizard). */
export function localizeFunctions(lang: FormulaLang | null, functions: readonly FunctionInfo[]): FunctionInfo[] {
  if (!lang) return [...functions];
  return functions.map((f) => {
    const name = localizedName(lang, f.name);
    return name === f.name ? f : { ...f, name, signature: f.signature.replace(new RegExp(`^${f.name}(?=\\()`), name) };
  });
}
