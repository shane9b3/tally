/**
 * A tiny client-side number formatter for previews only (the Custom number
 * format dialog, "At a glance" sums). The engine (`tally-numfmt`) is the
 * source of truth for cell text; this covers the common OOo codes:
 * sections (`pos;neg;zero`), colours (ignored), `[$$-409]` currency,
 * `0`/`#`/`,`/`.` digits, `%`, `E+00`, quoted literals, `@`, `General`,
 * and date/time codes (serial days since 1899-12-30).
 * Returns `null` for codes it does not understand.
 */

/** Separators and boolean names of the format's language (default en-US). */
export interface LiteLocale {
  decimal: string;
  group: string;
  trueName?: string;
  falseName?: string;
}

const EN_US: LiteLocale = { decimal: ".", group: ",", trueName: "TRUE", falseName: "FALSE" };

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/** Splits a format code into sections on `;` outside quotes and brackets. */
export function splitSections(code: string): string[] {
  const out: string[] = [];
  let cur = "";
  let q = false;
  let b = 0;
  for (let i = 0; i < code.length; i++) {
    const ch = code[i] ?? "";
    if (ch === "\\" && !q) {
      cur += ch + (code[i + 1] ?? "");
      i++;
      continue;
    }
    if (ch === '"') q = !q;
    else if (!q && ch === "[") b++;
    else if (!q && ch === "]") b = Math.max(0, b - 1);
    if (ch === ";" && !q && b === 0) {
      out.push(cur);
      cur = "";
    } else cur += ch;
  }
  out.push(cur);
  return out;
}

/** Turns brackets, quotes and escapes into plain literal text; drops colours and conditions. */
function literals(section: string): string {
  return section
    .replace(/\[\$([^\]-]*)(-[0-9A-Fa-f]+)?\]/g, (_, sym: string) => `"${sym}"`)
    .replace(/\[[^\]]*\]/g, "");
}

/** True when the section is a date/time pattern. */
export function isDateCode(code: string): boolean {
  const plain = literals(code).replace(/"[^"]*"/g, "").replace(/\\./g, "");
  return /[dyhs]/i.test(plain) || (/m/i.test(plain) && !/[0#]/.test(plain));
}

function generalText(v: number): string {
  if (!Number.isFinite(v)) return "#NUM!";
  const abs = Math.abs(v);
  if (abs !== 0 && (abs >= 1e11 || abs < 1e-9)) return v.toExponential(5).replace(/\.?0+e/, "E").replace("e", "E");
  return String(Number(v.toPrecision(10)));
}

function groupThousands(int: string, sep = ","): string {
  return int.replace(/\B(?=(\d{3})+(?!\d))/g, sep);
}

/** The closest fraction n/d to `x` (0 ≤ x < 1) with d ≤ `maxDen`. */
function bestRational(x: number, maxDen: number): [number, number] {
  let best: [number, number] = [0, 1];
  let err = Math.abs(x);
  for (let d = 1; d <= maxDen; d++) {
    const n = Math.round(x * d);
    const e = Math.abs(x - n / d);
    if (e < err - 1e-12) {
      best = [n, d];
      err = e;
    }
  }
  return best;
}

/** `# ?/?`, `# ??/16`, `?/?`: a whole part (when `intPh`) and a fraction. */
function formatFraction(v: number, intPh: string | undefined, denPh: string): string {
  const ax = Math.abs(v);
  const fixed = /^\d+$/.test(denPh) ? Number(denPh) : null;
  const maxDen = 10 ** denPh.length - 1;
  let whole = intPh !== undefined ? Math.floor(ax) : 0;
  const frac = ax - whole;
  let [n, d] = fixed ? [Math.round(frac * fixed), fixed] : bestRational(frac, maxDen);
  if (intPh === undefined) return `${n + whole * d}/${d}`;
  if (n === d) {
    whole += 1;
    n = 0;
  }
  if (n === 0) return String(whole);
  return whole === 0 && !intPh.includes("0") ? `${n}/${d}` : `${whole} ${n}/${d}`;
}

function formatDate(serial: number, section: string): string {
  const ms = Math.round((serial - 25569) * 86_400_000);
  const d = new Date(ms);
  const Y = d.getUTCFullYear();
  const M = d.getUTCMonth();
  const D = d.getUTCDate();
  const h = d.getUTCHours();
  const mi = d.getUTCMinutes();
  const s = d.getUTCSeconds();
  const ampm = /AM\/PM/i.test(section);
  const pad = (n: number, w = 2): string => String(n).padStart(w, "0");
  let out = "";
  let lastWasHour = false;
  // `[HH]`: elapsed hours (kept as a marker run through literals()).
  const src = literals(section.replace(/\[(h+)\]/gi, (_, x: string) => "\u0001".repeat(x.length)));
  let i = 0;
  while (i < src.length) {
    const rest = src.slice(i);
    const ch = src[i] ?? "";
    if (ch === '"') {
      const j = src.indexOf('"', i + 1);
      out += src.slice(i + 1, j < 0 ? src.length : j);
      i = j < 0 ? src.length : j + 1;
      continue;
    }
    if (ch === "\\") {
      out += src[i + 1] ?? "";
      i += 2;
      continue;
    }
    if (ch === "\u0001") {
      const n = /^\u0001+/.exec(rest)![0].length;
      out += pad(Math.floor(Math.abs(serial) * 24 + 1e-9), n);
      lastWasHour = true;
      i += n;
      continue;
    }
    if (/^AM\/PM/i.test(rest)) {
      out += h < 12 ? "AM" : "PM";
      i += 5;
      continue;
    }
    const run = /^([YyMmDdHhSs])\1*/.exec(rest)?.[0];
    if (run) {
      const k = run[0]!.toLowerCase();
      const n = run.length;
      if (k === "y") out += n <= 2 ? pad(Y % 100) : String(Y);
      else if (k === "d") out += n === 1 ? String(D) : n === 2 ? pad(D) : n === 3 ? DAYS[d.getUTCDay()]!.slice(0, 3) : DAYS[d.getUTCDay()]!;
      else if (k === "h") {
        const hh = ampm ? ((h + 11) % 12) + 1 : h;
        out += n >= 2 ? pad(hh) : String(hh);
      } else if (k === "s") {
        out += n >= 2 ? pad(s) : String(s);
        const fr = /^\.(0+)/.exec(src.slice(i + n));
        if (fr) {
          const digits = fr[1]!.length;
          out += "." + String(Math.floor(((ms % 1000) + 1000) % 1000 / 10 ** (3 - Math.min(3, digits)))).padStart(Math.min(3, digits), "0").padEnd(digits, "0");
          i += fr[0].length;
        }
      }
      else if (k === "m") {
        // Minutes right after hours (or before seconds); months otherwise.
        const minutes = lastWasHour || /^m+:?s/i.test(rest.replace(/^m+/i, (x) => x + ""));
        if (minutes && n <= 2) out += n === 2 ? pad(mi) : String(mi);
        else out += n === 1 ? String(M + 1) : n === 2 ? pad(M + 1) : n === 3 ? MONTHS[M]!.slice(0, 3) : MONTHS[M]!;
      }
      lastWasHour = k === "h" || (lastWasHour && k !== "s" && k !== "y" && k !== "d");
      i += n;
      continue;
    }
    if (ch !== ":") lastWasHour = lastWasHour && /\s/.test(ch);
    out += ch;
    i++;
  }
  return out;
}

function formatNumberSection(v: number, section: string, loc: LiteLocale): string | null {
  const src = literals(section);
  const unquoted = src.replace(/"[^"]*"/g, (x) => "\u0000".repeat(x.length));
  const frac = /(?:([0#?]+)\s+)?[0#?]+\/([0-9#?]+)/.exec(unquoted);
  if (frac) {
    const unq = (t: string): string => t.replace(/"([^"]*)"/g, "$1").replace(/\\(.)/g, "$1");
    return unq(src.slice(0, frac.index)) + formatFraction(v, frac[1], frac[2] ?? "?") + unq(src.slice(frac.index + frac[0].length));
  }
  const m = /[0#?][0#?,]*(\.[0#?]*)?(E[+-]0+)?|\.[0#?]+(E[+-]0+)?/i.exec(unquoted);
  const unquote = (t: string): string => t.replace(/"([^"]*)"/g, "$1").replace(/\\(.)/g, "$1");
  if (!m) return unquote(src);
  const pat = m[0];
  const before = unquote(src.slice(0, m.index));
  const after = unquote(src.slice(m.index + pat.length));
  const percent = (before + after).includes("%");
  let x = percent ? v * 100 : v;
  const [intPat = "", rest = ""] = pat.split(/[.]/);
  const expM = /E([+-])(0+)/i.exec(pat);
  const decPat = (rest.split(/E/i)[0] ?? "").replace(/[^0#?]/g, "");
  const minDec = (decPat.match(/0/g) ?? []).length;
  const maxDec = decPat.length;
  const grouping = intPat.includes(",") && !expM;
  const minInt = (intPat.replace(/,/g, "").match(/0/g) ?? []).length;
  let body: string;
  if (expM) {
    if (x === 0) body = `${(0).toFixed(maxDec).replace(".", loc.decimal)}E${expM[1] === "+" ? "+" : ""}${"0".repeat(expM[2]!.length)}`;
    else {
      const e = Math.floor(Math.log10(Math.abs(x)));
      let mant = x / 10 ** e;
      let exp = e;
      if (Number(Math.abs(mant).toFixed(maxDec)) >= 10) {
        mant /= 10;
        exp += 1;
      }
      const sign = exp < 0 ? "-" : expM[1] === "+" ? "+" : "";
      body = `${Math.abs(mant).toFixed(maxDec).replace(".", loc.decimal)}E${sign}${String(Math.abs(exp)).padStart(expM[2]!.length, "0")}`;
    }
  } else {
    x = Math.abs(x);
    let fixed = x.toFixed(maxDec);
    if (maxDec > minDec) fixed = fixed.replace(new RegExp(`0{1,${maxDec - minDec}}$`), "").replace(/\.$/, "");
    let [ip = "0", fp] = fixed.split(".");
    if (ip === "0" && minInt === 0) ip = "";
    ip = ip.padStart(minInt, "0");
    if (grouping) ip = groupThousands(ip, loc.group);
    body = fp !== undefined && fp !== "" ? `${ip}${loc.decimal}${fp}` : ip;
    if (body === "") body = "0";
    // `#,##0.--`: the decimal separator stays without decimals.
    if (maxDec === 0 && pat.includes(".")) body += loc.decimal;
  }
  return before + body + after;
}

/**
 * Formats `v` with an OOo format code, or `null` if the code is not
 * supported here. `loc` gives the separators of the format's language.
 */
export function formatNumberLite(v: number, code: string, loc: LiteLocale = EN_US): string | null {
  const trimmed = code.trim();
  const general = (x: number): string => generalText(x).replace(".", loc.decimal);
  if (trimmed === "" || /^general$/i.test(trimmed)) return general(v);
  const sections = splitSections(trimmed);
  let section = sections[0] ?? "";
  let value = v;
  let explicitNeg = false;
  if (v < 0 && sections.length >= 2) {
    section = sections[1] ?? section;
    value = -v;
    explicitNeg = true;
  } else if (v === 0 && sections.length >= 3) section = sections[2] ?? section;
  if (section.trim() === "@") return generalText(v);
  if (/BOOLEAN/i.test(literals(section).replace(/"[^"]*"/g, ""))) return v !== 0 ? (loc.trueName ?? "TRUE") : (loc.falseName ?? "FALSE");
  if (/^general$/i.test(literals(section).replace(/""/g, "").trim())) return general(value);
  try {
    if (isDateCode(section)) return formatDate(value, section);
    const out = formatNumberSection(value, section, loc);
    if (out === null) return null;
    return value < 0 && !explicitNeg ? `-${out}` : out;
  } catch {
    return null;
  }
}
