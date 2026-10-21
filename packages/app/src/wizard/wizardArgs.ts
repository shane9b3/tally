/**
 * Function Wizard argument model (§4 item 19): the parameters come from
 * `FunctionInfo.params` (the engine's registry metadata: name, optional,
 * and `repeat` for the trailing repeating group, e.g. SUMIFS's
 * `range; criterion` pairs). Only when a function has no metadata is its
 * signature (`SUMIF(range, criterion, [sum_range])`) parsed instead. The
 * call text is built from the argument fields.
 */
import type { FunctionInfo } from "@tally/bridge";

export interface WizardParam {
  name: string;
  optional: boolean;
  /** Part of the trailing repeating group (`SUM(number; ...)`). */
  variadic: boolean;
  description?: string;
}

/** Most arguments of one call (AOO's limit). */
export const MAX_ARGS = 255;

/** The parameters of a function, from `params` when present, else from the signature. */
export function wizardParams(f: FunctionInfo): WizardParam[] {
  const params = (f.params ?? []) as readonly (FunctionInfo["params"][number] & { description?: string })[];
  if (params.length > 0) {
    return params.map((p) => ({ name: p.name, optional: p.optional, variadic: p.repeat, ...(p.description ? { description: p.description } : {}) }));
  }
  return parseSignature(f.signature);
}

/** `NAME(a, [b], …)` → the parameters (`…` or a trailing `…` marks the last one as repeating). */
export function parseSignature(signature: string): WizardParam[] {
  const open = signature.indexOf("(");
  const close = signature.lastIndexOf(")");
  if (open < 0 || close < open) return [];
  const inner = signature.slice(open + 1, close).trim();
  if (inner === "") return [];
  const parts = splitTopLevel(inner).map((s) => s.trim()).filter((s) => s !== "");
  const out: WizardParam[] = [];
  let variadic = false;
  for (const raw of parts) {
    if (raw === "…" || raw === "..." || /^\[?\.\.\.\]?$/.test(raw)) {
      variadic = true;
      continue;
    }
    const optional = raw.startsWith("[") && raw.endsWith("]");
    const name = raw.replace(/^\[|\]$/g, "").replace(/…$|\.\.\.$/, "").trim();
    if (name === "") continue;
    out.push({ name, optional, variadic: false });
  }
  const last = out[out.length - 1];
  if (variadic && last) last.variadic = true;
  return out;
}

/** Splits on `,` and `;` outside parentheses/brackets. */
function splitTopLevel(s: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = "";
  for (const ch of s) {
    if (ch === "(" || ch === "[") depth++;
    else if (ch === ")" || ch === "]") depth--;
    if ((ch === "," || ch === ";") && depth <= 0) {
      out.push(cur);
      cur = "";
      continue;
    }
    cur += ch;
  }
  out.push(cur);
  return out;
}

/**
 * The argument fields to show: the fixed parameters, then the repeating
 * group once more than the last filled repetition (AOO grows the list as
 * the fields fill), names suffixed `1, 2, …`; at most {@link MAX_ARGS}.
 */
export function argumentSlots(params: readonly WizardParam[], values: readonly string[]): WizardParam[] {
  const first = params.findIndex((p) => p.variadic);
  if (first < 0) return [...params];
  const fixed = params.slice(0, first);
  const group = params.slice(first);
  const last = group[group.length - 1];
  if (!last) return [...params];
  // Signature fallback: `number1, [number2], …` (one numbered repeating name).
  if (group.length === 1 && /\d$/.test(last.name)) return legacySlots(fixed, last, values);
  const g = group.length;
  const extra = values.slice(fixed.length);
  let filled = 0;
  for (let k = 0; k * g < extra.length; k++) {
    if (extra.slice(k * g, k * g + g).some((v) => v.trim() !== "")) filled = k + 1;
  }
  const maxReps = Math.max(1, Math.floor((MAX_ARGS - fixed.length) / g));
  const reps = Math.min(maxReps, Math.max(1, filled + 1));
  const slots: WizardParam[] = [...fixed];
  for (let k = 0; k < reps; k++) {
    for (const p of group) slots.push({ name: `${p.name}${k + 1}`, optional: k > 0 || p.optional, variadic: true, ...(p.description ? { description: p.description } : {}) });
  }
  return slots;
}

function legacySlots(fixed: readonly WizardParam[], last: WizardParam, values: readonly string[]): WizardParam[] {
  const filled = values.slice(fixed.length).filter((v) => v.trim() !== "").length;
  const base = last.name.replace(/\d+$/, "");
  const startIndex = Number(/\d+$/.exec(last.name)?.[0] ?? "1");
  const slots: WizardParam[] = [...fixed];
  const n = Math.min(30, Math.max(1, filled + 1));
  for (let i = 0; i < n; i++) slots.push({ name: `${base}${startIndex + i}`, optional: i > 0 || last.optional, variadic: true });
  return slots;
}

/** Which arguments are missing (`null` = fine). */
export function argumentError(params: readonly WizardParam[], values: readonly string[]): string | null {
  const missing = params.find((p, i) => !p.optional && (values[i] ?? "").trim() === "");
  return missing ? `${missing.name} is required.` : null;
}

/** `NAME(a; b; c)` with trailing empty optional arguments dropped (`sep` is the display separator). */
export function buildCall(name: string, values: readonly string[], sep: ";" | "," = ";"): string {
  const vals = values.map((v) => v.trim());
  let end = vals.length;
  while (end > 0 && vals[end - 1] === "") end--;
  return `${name}(${vals.slice(0, end).join(sep === ";" ? ";" : ",")})`;
}

/** Splits the arguments of a call text (`SUM(A1;B1)` → `["A1", "B1"]`), for re-opening the wizard on an existing call. */
export function parseCall(text: string): { name: string; args: string[] } | null {
  const m = /^=?\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\((.*)\)\s*$/s.exec(text.trim());
  if (!m) return null;
  const inner = m[2] ?? "";
  return { name: (m[1] ?? "").toUpperCase(), args: inner.trim() === "" ? [] : splitArgs(inner) };
}

function splitArgs(s: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let quoted = false;
  let cur = "";
  for (const ch of s) {
    if (ch === '"') quoted = !quoted;
    if (!quoted) {
      if (ch === "(") depth++;
      else if (ch === ")") depth--;
      if ((ch === ";" || ch === ",") && depth === 0) {
        out.push(cur.trim());
        cur = "";
        continue;
      }
    }
    cur += ch;
  }
  out.push(cur.trim());
  return out;
}
