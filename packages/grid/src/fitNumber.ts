/**
 * Client-side General "shrink to fit" for numbers (docs/phase-a.md: General
 * shrinks to fit or shows ###). The engine formats for the default width;
 * when the column is narrower, the grid re-fits with OOo General rules:
 *
 * 1. the engine text, if it fits;
 * 2. fewer decimals (rounded, trailing zeros dropped), as long as some
 *    significant digit survives;
 * 3. scientific notation with fewer mantissa digits (`1.23E+10`, exponent
 *    at least two digits);
 * 4. `###`.
 *
 * Pure: `measure` returns the width of a string in the cell's font.
 */

export type Measure = (text: string) => number;

/** Most significant digits General shows. */
const MAX_DIGITS = 10;

/** `num` as OOo General would print it with no width limit (en-US). */
export function generalText(num: number): string {
  if (!Number.isFinite(num)) return "###";
  if (num === 0) return "0";
  const abs = Math.abs(num);
  if (abs >= 1e10 || abs < 1e-4) return sci(num, MAX_DIGITS - 1);
  const intDigits = abs >= 1 ? Math.floor(Math.log10(abs)) + 1 : 0;
  return fixed(num, Math.max(0, MAX_DIGITS - Math.max(1, intDigits)));
}

/** Rounds to `decimals` and drops trailing zeros (`1.50` → `1.5`). */
export function fixed(num: number, decimals: number): string {
  let s = num.toFixed(Math.min(20, Math.max(0, decimals)));
  if (s.includes(".")) s = s.replace(/0+$/, "").replace(/\.$/, "");
  if (s === "-0") s = "0";
  return s;
}

/** Scientific with up to `mantissaDigits` decimals: `1.23E+10`, `5E-07`. */
export function sci(num: number, mantissaDigits: number): string {
  const [m = "0", e = "0"] = num.toExponential(Math.min(20, Math.max(0, mantissaDigits))).split("e");
  const mant = m.includes(".") ? m.replace(/0+$/, "").replace(/\.$/, "") : m;
  const exp = Number(e);
  const sign = exp < 0 ? "-" : "+";
  return `${mant}E${sign}${String(Math.abs(exp)).padStart(2, "0")}`;
}

/** Fits `num` into `widthPx`. `text` is the engine's text (defaults to {@link generalText}). */
export function fitNumber(num: number, widthPx: number, measure: Measure, text?: string): string {
  const full = text ?? generalText(num);
  if (measure(full) <= widthPx) return full;
  if (!Number.isFinite(num)) return "###";

  const isSci = /E[+-]\d+$/i.test(full);
  if (!isSci && num !== 0) {
    const dot = full.indexOf(".");
    const decimals = dot >= 0 ? full.length - dot - 1 : 0;
    for (let d = decimals - 1; d >= 0; d--) {
      const s = fixed(num, d);
      if (Number(s) === 0) break; // every significant digit is gone
      if (measure(s) <= widthPx) return s;
    }
  }
  if (num !== 0) {
    let start = MAX_DIGITS - 1;
    if (isSci) {
      const m = /^-?\d(?:\.(\d+))?E/i.exec(full);
      start = Math.max(0, (m?.[1]?.length ?? 0) - 1);
    }
    for (let m = start; m >= 0; m--) {
      const s = sci(num, m);
      if (measure(s) <= widthPx) return s;
    }
  }
  return "###";
}

/** True if `text` looks like General output (so re-fitting cannot drop format characters). */
export function isGeneralNumberText(text: string): boolean {
  return /^-?\d+(\.\d+)?(E[+-]\d+)?$/i.test(text);
}
