/**
 * AutoInput (§4 item 16): column completion while typing. The engine's
 * `autocomplete(doc, sheet, r, c, prefix)` finds the candidates (text cells
 * of the same data block, case-insensitive, first match in alphabetical
 * order); these helpers decide when to ask, pick the match from a candidate
 * list (the browser mock and tests) and build the ghost text.
 */

/** Whether AutoInput should look for a completion for `text` (plain text, not a formula or number). */
export function wantsCompletion(text: string): boolean {
  if (text === "" || text.startsWith("=") || text.startsWith("'")) return false;
  if (/^[-+\d.,\s%:/]+$/.test(text)) return false;
  return /\S/.test(text);
}

/**
 * The first case-insensitive match of `prefix` among `candidates`, in
 * alphabetical order (ties: shortest first), as the full text to offer with
 * the typed prefix kept as typed. `null` when nothing (longer) matches.
 */
export function completionFor(prefix: string, candidates: readonly string[]): string | null {
  const p = prefix.toLowerCase();
  if (p === "") return null;
  const matches = [...new Set(candidates)]
    .filter((c) => c.length > prefix.length && c.toLowerCase().startsWith(p))
    .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }) || a.length - b.length);
  const m = matches[0];
  return m === undefined ? null : prefix + m.slice(prefix.length);
}

/** Every candidate matching `prefix` (all of them for an empty prefix), sorted case-insensitively. */
export function matchingItems(prefix: string, candidates: readonly string[], max = 1000): string[] {
  const p = prefix.toLowerCase();
  return [...new Set(candidates)]
    .filter((c) => c !== "" && (p === "" || c.toLowerCase().startsWith(p)))
    .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }))
    .slice(0, max);
}

/** The part of a completion shown as ghost text after the typed text. */
export function ghostSuffix(typed: string, completion: string | null): string {
  if (!completion || !completion.toLowerCase().startsWith(typed.toLowerCase()) || completion.length <= typed.length) return "";
  return completion.slice(typed.length);
}
