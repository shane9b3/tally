/**
 * The preview of codes with `[NatNumN]`, `[DBNumN]`, `[~calendar]` or era
 * codes (`G`, `E`, `R`, `AAA`): the engine's `date_section_cal` and
 * NatNum transliteration (crates/tally-numfmt/src/format.rs), on top of
 * {@link ./natnum}. {@link ./numfmtLite} calls it for such codes only.
 */
import {
  calendarFields,
  calendarFromName,
  calendarNatNum,
  dayNames,
  displayNumber,
  eraNames,
  hasCalendar,
  isValidNatNum,
  LANG_EN_US,
  langOfTag,
  mapDbNum,
  monthNames,
  otherCalendar,
  transliterate,
  type CalendarId,
  type Fields,
  type LangId,
  type NumCode,
  type Separators,
} from "./natnum";

/** Whether a code needs the native formatter. */
export function needsNative(code: string): boolean {
  if (/\[(NatNum|DBNum)\d+\]|\[~/i.test(code)) return true;
  if (/\[\$-[A-Za-z]{2,3}-[^\]]*\]/.test(code)) return true;
  const plain = code
    .replace(/"[^"]*"/g, "")
    .replace(/\\./g, "")
    .replace(/\[[^\]]*\]/g, "")
    .replace(/AM\/PM|A\/P|general|standard|boolean/gi, "");
  // Era codes (R next to digit placeholders is text: `R 0.00`).
  return /[GgEeRr]|[Aa]{3}/.test(plain) && !/[0#?@]/.test(plain);
}

/** Whether a section is a date / time section for the native path. */
export function isNativeDate(section: string): boolean {
  const plain = section
    .replace(/"[^"]*"/g, "")
    .replace(/\\./g, "")
    .replace(/\[(?![HMSHhms]+\])[^\]]*\]/g, "")
    .replace(/general|standard|boolean/gi, "");
  if (/[0#?@]/.test(plain.replace(/[Ss]\.0+/g, ""))) return false;
  return /[YyDdHhSsMmGgEeRrQq]|[Nn]{2}|[Aa]{3}|[Ww]{2}|\[[HMShms]+\]/.test(plain);
}

interface Names {
  months: [string, string][];
  days: [string, string][];
  quarters: [string, string][];
  ampm: [string, string];
  eras: [string, string][];
  daySep: string;
}

const EN: Names = {
  months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"].map((m) => [m.slice(0, 3), m]),
  days: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"].map((d) => [d.slice(0, 3), d]),
  quarters: [["Q1", "1st quarter"], ["Q2", "2nd quarter"], ["Q3", "3rd quarter"], ["Q4", "4th quarter"]],
  ampm: ["AM", "PM"],
  eras: [["BC", "Before Christ"], ["AD", "Anno Domini"]],
  daySep: ", ",
};
const JA: Names = {
  months: Array.from({ length: 12 }, (_, i) => [`${i + 1}月`, `${i + 1}月`]),
  days: [["日", "日曜日"], ["月", "月曜日"], ["火", "火曜日"], ["水", "水曜日"], ["木", "木曜日"], ["金", "金曜日"], ["土", "土曜日"]],
  quarters: [1, 2, 3, 4].map((q) => [`Q${q}`, `第${q}四半期`]),
  ampm: ["午前", "午後"],
  eras: [["紀元前", "紀元前"], ["西暦", "西暦"]],
  daySep: " ",
};

/** The locale data Tally formats with (ja-JP or English names). */
const namesOf = (lang: LangId): Names => ((lang & 0x3ff) === 0x11 ? JA : EN);

export interface NativeLocale extends Separators {
  /** The document language (hex LCID). */
  lcid: string;
}

/** The first `[$-…]` language tag without a symbol. */
function tagLang(section: string): LangId | null {
  const re = /\[\$-([^\]]+)\]/g;
  for (let m = re.exec(section); m; m = re.exec(section)) {
    const l = langOfTag(m[1]!);
    if (l !== null) return l;
  }
  return null;
}

function natnumOf(section: string): { num: number; db: boolean } | null {
  const m = /\[(NatNum|DBNum)(\d+)\]/i.exec(section);
  return m ? { num: Number(m[2]), db: m[1]!.toUpperCase() === "DBNUM" } : null;
}

/** The format language: the code's first tag, else the document's. */
export function formatLang(code: string, loc: NativeLocale): LangId {
  return tagLang(code) ?? langOfTag(loc.lcid) ?? LANG_EN_US;
}

/** NatNum mode and language of a section. */
export function sectionNatNum(section: string, code: string, loc: NativeLocale, date: boolean): [LangId, number] | null {
  const nn = natnumOf(section);
  if (!nn) return null;
  const lang = tagLang(section) ?? formatLang(code, loc);
  return [lang, nn.db ? mapDbNum(nn.num, lang, date) : nn.num];
}

/** Transliterates a formatted number section's text (ImpTransliterate). */
export function transliterateNumber(text: string, section: string, code: string, loc: NativeLocale): string {
  const nn = sectionNatNum(section, code, loc, false);
  return nn ? transliterate(text, nn[0], nn[1], loc) : text;
}

type Tok =
  | { t: "lit"; s: string }
  | { t: "cal"; name: string }
  | { t: "key"; c: string; n: number }
  | { t: "ampm" }
  | { t: "ap"; lower: boolean }
  | { t: "elapsed"; c: string; n: number }
  | { t: "frac"; n: number };

function tokenize(section: string): Tok[] {
  const out: Tok[] = [];
  let i = 0;
  const s = section;
  while (i < s.length) {
    const c = s[i]!;
    if (c === '"') {
      const j = s.indexOf('"', i + 1);
      out.push({ t: "lit", s: s.slice(i + 1, j < 0 ? s.length : j) });
      i = j < 0 ? s.length : j + 1;
      continue;
    }
    if (c === "\\" || c === "_" || c === "*") {
      if (c === "\\") out.push({ t: "lit", s: s[i + 1] ?? "" });
      else if (c === "_") out.push({ t: "lit", s: " " });
      i += 2;
      continue;
    }
    if (c === "[") {
      const j = s.indexOf("]", i);
      const inner = s.slice(i + 1, j < 0 ? s.length : j);
      i = j < 0 ? s.length : j + 1;
      if (inner.startsWith("$")) {
        const body = inner.slice(1);
        const dash = body.indexOf("-");
        const sym = dash < 0 ? body : body.slice(0, dash);
        if (sym) out.push({ t: "lit", s: sym });
      } else if (inner.startsWith("~")) out.push({ t: "cal", name: inner.slice(1) });
      else if (/^([HMS])\1*$/i.test(inner)) out.push({ t: "elapsed", c: inner[0]!.toUpperCase(), n: inner.length });
      continue;
    }
    const rest = s.slice(i);
    if (/^AM\/PM/i.test(rest)) {
      out.push({ t: "ampm" });
      i += 5;
      continue;
    }
    if (/^A\/P/i.test(rest)) {
      out.push({ t: "ap", lower: c === "a" });
      i += 3;
      continue;
    }
    if (/[A-Za-z]/.test(c)) {
      const u = c.toUpperCase();
      let n = 1;
      while (i + n < s.length && s[i + n]!.toUpperCase() === u) n++;
      const cap: Record<string, number> = { G: 3, E: 2, R: 2, N: 4, A: 4, Q: 2 };
      const known = "YMDHSGERQ".includes(u) || (u === "N" && n >= 2) || (u === "A" && n >= 3) || (u === "W" && n >= 2);
      if (!known) {
        out.push({ t: "lit", s: s.slice(i, i + n) });
        i += n;
        continue;
      }
      const k = Math.min(n, cap[u] ?? n);
      out.push({ t: "key", c: u, n: k });
      i += k;
      continue;
    }
    if (c === "." && /^\.0+/.test(rest)) {
      const prev = [...out].reverse().find((x) => x.t !== "lit");
      if (prev && ((prev.t === "key" && prev.c === "S") || (prev.t === "elapsed" && prev.c === "S"))) {
        const n = /^\.(0+)/.exec(rest)![1]!.length;
        out.push({ t: "frac", n });
        i += n + 1;
        continue;
      }
    }
    out.push({ t: "lit", s: c });
    i++;
  }
  // M / MM after an hour or before seconds are minutes.
  const keys = out.map((x, k) => [x, k] as const).filter(([x]) => x.t === "key" || x.t === "elapsed" || x.t === "ampm" || x.t === "ap");
  keys.forEach(([x], k) => {
    if (x.t !== "key" || x.c !== "M" || x.n > 2) return;
    const prev = keys[k - 1]?.[0];
    const next = keys[k + 1]?.[0];
    const isH = (y: Tok | undefined): boolean => !!y && ((y.t === "key" && y.c === "H") || (y.t === "elapsed" && y.c === "H"));
    const isS = (y: Tok | undefined): boolean => !!y && ((y.t === "key" && y.c === "S") || (y.t === "elapsed" && y.c === "S"));
    if (isH(prev) || isS(next)) x.c = "MIN";
  });
  return out;
}

const DAY_MS = 86_400_000;

/** A date / time section in the engine's calendar path. */
export function formatDateNative(serial: number, section: string, code: string, loc: NativeLocale): string | null {
  const toks = tokenize(section);
  const hasDate = toks.some((x) => x.t === "key" && "YMDGERNAQW".includes(x.c[0]!) && x.c !== "MIN");
  const twelve = toks.some((x) => x.t === "ampm" || x.t === "ap");
  const fd = Math.min(6, Math.max(0, ...toks.map((x) => (x.t === "frac" ? x.n : 0))));
  let v = serial;
  let neg = false;
  if (v < 0 && !hasDate) {
    neg = true;
    v = -v;
  }
  const scale = 10 ** fd;
  const perDay = 86_400 * scale;
  let days = Math.floor(v);
  let ticks = Math.round((v - days) * perDay);
  if (ticks >= perDay) {
    days += 1;
    ticks -= perDay;
  }
  const elapsed = toks.some((x) => x.t === "elapsed");
  const t = elapsed ? Math.round(v * perDay) : ticks;
  const secs = Math.floor(t / scale);
  const frac = t % scale;
  const hour = Math.floor(secs / 3600) % 24;
  const minute = Math.floor(secs / 60) % 60;
  const second = secs % 60;
  const unix = days - 25569;
  const dt = new Date(unix * DAY_MS);
  const y = hasDate ? dt.getUTCFullYear() : 1899;
  const m = hasDate ? dt.getUTCMonth() + 1 : 12;
  const d = hasDate ? dt.getUTCDate() : 30;
  const wd = hasDate ? dt.getUTCDay() : 6;
  const sep: Separators = loc;
  const calLang = formatLang(code, loc);
  // The locale whose data formats (format_language: a tag Tally has data
  // for, else the document's).
  const docLang = langOfTag(loc.lcid) ?? LANG_EN_US;
  const tag = tagLang(code);
  const dataLang = tag !== null && [0x09, 0x11, 0x07, 0x0c, 0x0a].includes(tag & 0x3ff) ? tag : docLang;
  const names = namesOf(dataLang);
  const ownLocale = (dataLang & 0x3ff) === (calLang & 0x3ff);
  const gannen = /\[\$-[^\]]*-x-gannen\]/i.test(code);
  const nat = sectionNatNum(section, code, loc, hasDate);
  const mode = nat ? nat[1] : 0;
  const intNat = nat && isValidNatNum(nat[0], nat[1]) ? nat : null;
  const firstCal = toks.find((x) => x.t === "cal" || (x.t === "key" && (x.c === "E" || x.c === "R" || (x.c === "A" && x.n >= 3))));
  const implicit = !!firstCal && firstCal.t === "key";
  const at = (c: CalendarId): Fields => calendarFields(c, y, m, d, unix, wd);
  const greg = at("gregorian");
  let cur: CalendarId = "gregorian";
  let other = false;
  if (implicit && hasDate) {
    const o = otherCalendar(calLang);
    if (o) {
      cur = o;
      other = true;
    }
  }
  let f = cur === "gregorian" ? greg : at(cur);
  if (cur === "gengou" && f.era === 0) {
    cur = "gregorian";
    other = false;
    f = greg;
  }
  const int = (val: number, min: number): string => {
    const s = String(val).padStart(min, "0");
    return intNat ? transliterate(s, intNat[0], intNat[1], sep) : s;
  };
  const num = (cal: CalendarId, fl: Fields, c: NumCode): string => displayNumber(cal, calLang, fl, c, mode, gannen, sep);
  const dayName = (cal: CalendarId): [string, string] => (dayNames(cal) ?? names.days)[wd]!;
  const era = (cal: CalendarId, fl: Fields): [string, string] => {
    const table = cal === "gregorian" && ownLocale ? null : eraNames(cal, calLang);
    return (table ?? names.eras)[fl.era] ?? ["", ""];
  };
  let fracChars: string[] = [];
  if (fd > 0) {
    const s = String(frac).padStart(fd, "0");
    fracChars = [...(intNat ? transliterate(s, intNat[0], intNat[1], sep) : s)];
  }
  let out = "";
  for (const x of toks) {
    switch (x.t) {
      case "lit":
        out += x.s;
        break;
      case "frac":
        out += loc.decimal + fracChars.slice(0, x.n).join("");
        break;
      case "cal": {
        const c = calendarFromName(x.name);
        if (c && hasCalendar(calLang, c)) {
          cur = c;
          f = at(c);
          if (cur === "gengou" && f.era === 0) {
            cur = "gregorian";
            f = greg;
          }
        }
        break;
      }
      case "ampm":
        out += hour < 12 ? names.ampm[0] : names.ampm[1];
        break;
      case "ap":
        out += (hour < 12 ? "A" : "P")[x.lower ? "toLowerCase" : "toUpperCase"]();
        break;
      case "elapsed": {
        const val = x.c === "H" ? Math.floor(secs / 3600) : x.c === "M" ? Math.floor(secs / 60) : secs;
        out += int(val, x.n);
        break;
      }
      case "key": {
        const n = x.n;
        switch (x.c) {
          case "D":
            if (n <= 2) out += num(cur, f, n === 1 ? "shortDay" : "longDay");
            else out += dayName(other ? "gregorian" : cur)[n >= 4 ? 1 : 0];
            break;
          case "N": {
            const [a, l] = dayName(cur);
            out += n === 2 ? a : l + (n >= 4 ? names.daySep : "");
            break;
          }
          case "A": {
            const [a, l] = dayName(cur);
            out += n >= 4 ? l : a;
            break;
          }
          case "M":
            if (n <= 2) out += num(cur, f, n === 1 ? "shortMonth" : "longMonth");
            else {
              const [a, l] = (monthNames(cur) ?? names.months)[f.month - 1] ?? ["", ""];
              out += n === 3 ? a : n === 4 ? l : l.charAt(0);
            }
            break;
          case "MIN":
            out += int(minute, n);
            break;
          case "Y": {
            const cal = other ? "gregorian" : cur;
            const c: NumCode = n <= 2 ? "shortYear" : "longYear";
            if (cal === "gregorian") {
              const text = n <= 2 ? String(((y % 100) + 100) % 100).padStart(2, "0") : String(Math.max(0, y)).padStart(4, "0");
              out += calendarNatNum(text, cal, calLang, c, y, mode, sep);
            } else out += num(cal, f, c);
            break;
          }
          case "E":
            out += num(cur, f, n === 1 ? "shortYear" : "longYear");
            break;
          case "R": {
            if (n === 1) out += num(cur, f, "longYear");
            else {
              const [a, l] = era(cur, f);
              out += cur === "buddhist" && f.era === 0 ? num(cur, f, "shortYear") + a : l + num(cur, f, "longYear");
            }
            break;
          }
          case "G": {
            const [a, l] = era(cur, f);
            out += n === 1 && cur === "gengou" ? "?MTSHR"[Math.min(f.era, 5)] : n <= 2 ? a : l;
            break;
          }
          case "Q": {
            const q = Math.min(3, Math.floor((f.month - 1) / 3));
            out += calendarNatNum(names.quarters[q]![n >= 2 ? 1 : 0], cur, calLang, "quarter", 0, mode, sep);
            break;
          }
          case "W": {
            // ISO week number.
            const target = new Date(Date.UTC(y, m - 1, d));
            const dayNr = (target.getUTCDay() + 6) % 7;
            target.setUTCDate(target.getUTCDate() - dayNr + 3);
            const firstThursday = new Date(Date.UTC(target.getUTCFullYear(), 0, 4));
            const week = 1 + Math.round(((target.getTime() - firstThursday.getTime()) / DAY_MS - 3 + ((firstThursday.getUTCDay() + 6) % 7)) / 7);
            out += int(week, 0);
            break;
          }
          case "H": {
            const h = twelve ? (hour % 12 === 0 ? 12 : hour % 12) : hour;
            out += int(h, n);
            break;
          }
          case "S":
            out += int(second, n);
            break;
        }
        break;
      }
    }
  }
  if (neg && (secs > 0 || frac > 0)) out = `-${out}`;
  return out;
}
