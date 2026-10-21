/**
 * Function Wizard argument model (§4 item 19): parses a `FunctionInfo`
 * signature such as `SUMIF(range, criterion, [sum_range])` or
 * `SUM(number1, [number2], …)` into parameters, and builds the call text
 * from the argument fields. `FunctionInfo.params` is not in the DTO yet;
 * when the engine adds it the parsed list is replaced by it.
 */
import type { FunctionInfo } from "@tally/bridge";

export interface WizardParam {
  name: string;
  optional: boolean;
  /** The parameter repeats (`number1, [number2], …`). */
  variadic: boolean;
  description?: string;
}

/** Optional metadata the engine may add to `FunctionInfo` later (W5-4). */
interface FunctionMeta {
  params?: readonly { name: string; description?: string; optional?: boolean }[];
  example?: string;
}

/** The parameters of a function, from `params` when present, else from the signature. */
export function wizardParams(f: FunctionInfo): WizardParam[] {
  const meta = f as FunctionInfo & FunctionMeta;
  if (meta.params && meta.params.length > 0) {
    return meta.params.map((p, i, all) => ({ name: p.name, optional: p.optional ?? false, variadic: i === all.length - 1 && /\d$/.test(p.name), ...(p.description ? { description: p.description } : {}) }));
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

/** The number of argument fields to show: every parameter, plus free slots for a repeating last one. */
export function argumentSlots(params: readonly WizardParam[], values: readonly string[]): WizardParam[] {
  const last = params[params.length - 1];
  if (!last?.variadic) return [...params];
  const fixed = params.slice(0, -1);
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
  const missing = params.find((p, i) => !p.optional && !p.variadic && (values[i] ?? "").trim() === "");
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
