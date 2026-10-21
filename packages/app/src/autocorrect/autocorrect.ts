/**
 * AutoCorrect (§4 item 17): applied to a text cell's input on commit, as
 * one undo step with the typing. Rules follow Tools ▸ AutoCorrect Options:
 * TWo INitial CApitals, capitalise the first letter of every sentence,
 * replace dashes, ordinal suffixes, smart quotes and the replacement table.
 * Formulas, numbers and inputs that start with `'` are never touched.
 */
import type { AutoCorrectPrefs } from "../options/prefs";
import { DEFAULT_AUTOCORRECT } from "../options/prefs";
import { REPLACEMENTS } from "./replacements";

const TABLE = new Map(REPLACEMENTS.map(([from, to]) => [from.toLowerCase(), to]));

/** Inputs AutoCorrect leaves alone: formulas, numbers, dates, booleans, quoted text. */
export function isPlainText(input: string): boolean {
  const t = input.trim();
  if (t === "" || t.startsWith("=") || t.startsWith("{") || t.startsWith("'")) return false;
  if (/^[-+]?[\d.,]+(e[-+]?\d+)?%?$/i.test(t)) return false;
  if (/^(true|false)$/i.test(t)) return false;
  if (/^\d{1,4}[-/.]\d{1,2}([-/.]\d{1,4})?$/.test(t) || /^\d{1,2}:\d{2}(:\d{2})?$/.test(t)) return false;
  if (/^[a-z]+:\/\/\S+$/i.test(t) || /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(t)) return false;
  return true;
}

/** Keeps the initial capital of `word` when replacing it with `to`. */
function matchCase(word: string, to: string): string {
  if (word === word.toUpperCase() && word.length > 1 && /[a-z]/i.test(word)) return to.toUpperCase();
  const first = word[0] ?? "";
  if (first !== first.toLowerCase()) return (to[0] ?? "").toUpperCase() + to.slice(1);
  return to;
}

/** The replacement table on whole words (letters, digits and a few symbols). */
export function applyReplacementTable(text: string): string {
  return text.replace(/\([a-z]{1,2}\)|-->|<--|\.\.\.|\b\d\/[24]\b|[A-Za-z']+/g, (word) => {
    const to = TABLE.get(word.toLowerCase());
    return to === undefined ? word : matchCase(word, to);
  });
}

/** TWo INitial CApitals → Two Initial Capitals (all-caps words and words with digits stay). */
export function fixTwoInitialCapitals(text: string): string {
  return text.replace(/\b([A-Z])([A-Z])([a-z]+)\b/g, (_, a: string, b: string, rest: string) => `${a}${b.toLowerCase()}${rest}`);
}

/** Capitalises the first letter of the text and of every sentence after `. `, `! ` or `? `. */
export function capitalizeSentences(text: string): string {
  const upFirst = (s: string): string => {
    const m = /^(\s*)([a-z])/.exec(s);
    return m ? `${m[1] ?? ""}${(m[2] ?? "").toUpperCase()}${s.slice(m[0].length)}` : s;
  };
  let out = upFirst(text);
  // After a sentence end; abbreviations (a word of at most 2 letters before the dot, e.g. "e.g.") do not end one.
  out = out.replace(/(^|\s)(\S*?)([.!?])(\s+)([a-z])/g, (whole, lead: string, word: string, punct: string, gap: string, ch: string) => {
    if (punct === "." && /^[A-Za-z]{1,2}$/.test(word)) return whole;
    if (word.includes(".") && punct === ".") return whole;
    return `${lead}${word}${punct}${gap}${ch.toUpperCase()}`;
  });
  return out;
}

/** ` - ` → ` – ` (en dash) and `--` between words → `—` (em dash). */
export function replaceDashes(text: string): string {
  return text.replace(/(\S) - (\S)/g, "$1 – $2").replace(/(\w)--(\w)/g, "$1—$2");
}

/** Repairs ordinal suffixes (`1th` → `1st`, `22th` → `22nd`, `11st` → `11th`); correct ones are kept. */
export function fixOrdinals(text: string): string {
  return text.replace(/\b(\d+)(st|nd|rd|th)\b/g, (_, num: string, suffix: string) => {
    const n = Number(num);
    const mod100 = n % 100;
    const want = mod100 >= 11 && mod100 <= 13 ? "th" : n % 10 === 1 ? "st" : n % 10 === 2 ? "nd" : n % 10 === 3 ? "rd" : "th";
    void suffix;
    return `${num}${want}`;
  });
}

/** Straight quotes → typographic quotes; apostrophes inside words → ’. */
export function smartQuotes(text: string): string {
  let out = text.replace(/(\w)'(\w)/g, "$1’$2");
  out = out.replace(/(^|[\s(\[{])"/g, "$1“").replace(/"/g, "”");
  out = out.replace(/(^|[\s(\[{])'/g, "$1‘").replace(/'/g, "’");
  return out;
}

/** Applies the enabled rules to a committed cell input (the text is returned unchanged when nothing applies). */
export function applyAutoCorrect(input: string, prefs: AutoCorrectPrefs = DEFAULT_AUTOCORRECT): string {
  if (!isPlainText(input)) return input;
  let out = input;
  if (prefs.replacementTable) out = applyReplacementTable(out);
  if (prefs.twoInitialCapitals) out = fixTwoInitialCapitals(out);
  if (prefs.capitalizeSentence) out = capitalizeSentences(out);
  if (prefs.replaceDashes) out = replaceDashes(out);
  if (prefs.ordinalSuffixes) out = fixOrdinals(out);
  if (prefs.smartQuotes) out = smartQuotes(out);
  return out;
}

export const AUTOCORRECT_OPTIONS: readonly { key: keyof AutoCorrectPrefs; label: string; example: string }[] = [
  { key: "twoInitialCapitals", label: "Correct TWo INitial CApitals", example: "TEst → Test" },
  { key: "capitalizeSentence", label: "Capitalise the first letter of every sentence", example: "hello. world → Hello. World" },
  { key: "replaceDashes", label: "Replace dashes", example: "a - b → a – b" },
  { key: "ordinalSuffixes", label: "Format ordinal number suffixes (1st, 2nd…)", example: "2th → 2nd" },
  { key: "smartQuotes", label: "Replace straight quotes with typographic quotes", example: "\"x\" → “x”" },
  { key: "replacementTable", label: "Use the replacement table", example: "teh → the" },
];
