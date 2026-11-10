/**
 * Native numbers and calendars for the Format Cells preview: a port of
 * `crates/tally-numfmt/src/natnum.rs` and `calendar.rs` (themselves ports
 * of AOO 4.1 `nativenumbersupplier.cxx` and `calendar_*.cxx`). The engine
 * renders cells; this keeps the dialog's sample identical for `[NatNumN]`,
 * `[DBNumN]`, `[~calendar]` and the era codes. Both are tested against the
 * shared table `crates/tally-numfmt/tests/data/natnum_cases.json`.
 */

/** A Windows language id (16 bits). */
export type LangId = number;

export const LANG_JA = 0x411;
export const LANG_ZH_CN = 0x804;
export const LANG_ZH_TW = 0x404;
export const LANG_KO = 0x412;
export const LANG_HE = 0x40d;
export const LANG_AR_SA = 0x401;
export const LANG_TH = 0x41e;
export const LANG_EN_US = 0x409;

const primary = (l: LangId): number => l & 0x3ff;

/** BCP 47 language-country → LCID (the engine's `LCID_LANGUAGES`, subset used here). */
const TAGS: Record<string, number> = {
  "en-US": 0x409, "en-GB": 0x809, "de-DE": 0x407, "fr-FR": 0x40c, "es-ES": 0xc0a, "ja-JP": 0x411,
  "zh-CN": 0x804, "zh-TW": 0x404, "zh-HK": 0xc04, "zh-MO": 0x1404, "zh-SG": 0x1004, "ko-KR": 0x412,
  "ar-SA": 0x401, "ar-EG": 0xc01, "th-TH": 0x41e, "he-IL": 0x40d, "hi-IN": 0x439, "fa-IR": 0x429,
};

/** The language of a `[$-…]` tag: hex (calendar / numeral bytes dropped) or BCP 47. */
export function langOfTag(tag: string): LangId | null {
  const t = tag.trim();
  if (/^[0-9A-Fa-f]+$/.test(t)) {
    const v = Number.parseInt(t, 16) & 0xffff;
    return v === 0 ? null : v;
  }
  const [l, c] = t.split(/[-_]/);
  if (!l || !/^[A-Za-z]{2,3}$/.test(l)) return null;
  const key = Object.keys(TAGS).find((k) => {
    const [kl, kc] = k.split("-");
    return kl!.toLowerCase() === l.toLowerCase() && (c === undefined || !/^[A-Za-z]{2}$/.test(c) || kc!.toLowerCase() === c.toLowerCase());
  });
  return key ? TAGS[key]! : null;
}

// --- numberchar.h --------------------------------------------------------

const HALF = 0, FULL = 1, LOWER_ZH = 2, UPPER_ZH = 3, UPPER_ZH_TW = 4, MODERN_JA = 5, TRAD_JA = 6, LOWER_KO = 7, UPPER_KO = 8, HANGUL_KO = 9;
const INDIC_AR = 10, EAST_INDIC_AR = 11, HI = 12, TH = 13, OR = 14, MR = 15, BN = 16, PA = 17, GU = 18, TA = 19, TE = 20, KN = 21, ML = 22, LO = 23, BO = 24, MY = 25, KM = 26, MN = 27, HE = 28, NE = 29, DZ = 30;

const NUMBER_CHAR: number[][] = [
  [0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39],
  [0xff10, 0xff11, 0xff12, 0xff13, 0xff14, 0xff15, 0xff16, 0xff17, 0xff18, 0xff19],
  [0x3007, 0x4e00, 0x4e8c, 0x4e09, 0x56db, 0x4e94, 0x516d, 0x4e03, 0x516b, 0x4e5d],
  [0x96f6, 0x58f9, 0x8d30, 0x53c1, 0x8086, 0x4f0d, 0x9646, 0x67d2, 0x634c, 0x7396],
  [0x96f6, 0x58f9, 0x8cb3, 0x53c3, 0x8086, 0x4f0d, 0x9678, 0x67d2, 0x634c, 0x7396],
  [0x3007, 0x4e00, 0x4e8c, 0x4e09, 0x56db, 0x4e94, 0x516d, 0x4e03, 0x516b, 0x4e5d],
  [0x96f6, 0x58f1, 0x5f10, 0x53c2, 0x56db, 0x4f0d, 0x516d, 0x4e03, 0x516b, 0x4e5d],
  [0x3007, 0x4e00, 0x4e8c, 0x4e09, 0x56db, 0x4e94, 0x516d, 0x4e03, 0x516b, 0x4e5d],
  [0xf9b2, 0x58f9, 0x8cb3, 0x53c3, 0x56db, 0x4e94, 0x516d, 0x4e03, 0x516b, 0x4e5d],
  [0xc601, 0xc77c, 0xc774, 0xc0bc, 0xc0ac, 0xc624, 0xc721, 0xce60, 0xd314, 0xad6c],
  [0x660, 0x661, 0x662, 0x663, 0x664, 0x665, 0x666, 0x667, 0x668, 0x669],
  [0x6f0, 0x6f1, 0x6f2, 0x6f3, 0x6f4, 0x6f5, 0x6f6, 0x6f7, 0x6f8, 0x6f9],
  [0x966, 0x967, 0x968, 0x969, 0x96a, 0x96b, 0x96c, 0x96d, 0x96e, 0x96f],
  [0xe50, 0xe51, 0xe52, 0xe53, 0xe54, 0xe55, 0xe56, 0xe57, 0xe58, 0xe59],
  [0x866, 0x867, 0x868, 0x869, 0x86a, 0x86b, 0x86c, 0x86d, 0x86e, 0x86f],
  [0x966, 0x967, 0x968, 0x969, 0x96a, 0x96b, 0x96c, 0x96d, 0x96e, 0x96f],
  [0x9e6, 0x9e7, 0x9e8, 0x9e9, 0x9ea, 0x9eb, 0x9ec, 0x9ed, 0x9ee, 0x9ef],
  [0xa66, 0xa67, 0xa68, 0xa69, 0xa6a, 0xa6b, 0xa6c, 0xa6d, 0xa6e, 0xa6f],
  [0xae6, 0xae7, 0xae8, 0xae9, 0xaea, 0xaeb, 0xaec, 0xaed, 0xaee, 0xaef],
  [0x30, 0xbe7, 0xbe8, 0xbe9, 0xbea, 0xbeb, 0xbec, 0xbed, 0xbee, 0xbef],
  [0xc66, 0xc67, 0xc68, 0xc69, 0xc6a, 0xc6b, 0xc6c, 0xc6d, 0xc6e, 0xc6f],
  [0xce6, 0xce7, 0xce8, 0xce9, 0xcea, 0xceb, 0xcec, 0xced, 0xcee, 0xcef],
  [0xde6, 0xde7, 0xde8, 0xde9, 0xdea, 0xdeb, 0xdec, 0xded, 0xdee, 0xdef],
  [0xed0, 0xed1, 0xed2, 0xed3, 0xed4, 0xed5, 0xed6, 0xed7, 0xed8, 0xed9],
  [0xf20, 0xf21, 0xf22, 0xf23, 0xf24, 0xf25, 0xf26, 0xf27, 0xf28, 0xf29],
  [0x1040, 0x1041, 0x1042, 0x1043, 0x1044, 0x1045, 0x1046, 0x1047, 0x1048, 0x1049],
  [0x17e0, 0x17e1, 0x17e2, 0x17e3, 0x17e4, 0x17e5, 0x17e6, 0x17e7, 0x17e8, 0x17e9],
  [0x1810, 0x1811, 0x1812, 0x1813, 0x1814, 0x1815, 0x1816, 0x1817, 0x1818, 0x1819],
  [0x20, 0x5d0, 0x5d1, 0x5d2, 0x5d3, 0x5d4, 0x5d5, 0x5d6, 0x5d7, 0x5d8],
  [0x966, 0x967, 0x968, 0x969, 0x96a, 0x96b, 0x96c, 0x96d, 0x96e, 0x96f],
  [0xf20, 0xf21, 0xf22, 0xf23, 0xf24, 0xf25, 0xf26, 0xf27, 0xf28, 0xf29],
];
const DECIMAL_CHAR = [0x2e, 0xff0e, 0xff0e, 0x70b9, 0x9ede, 0x30fb, 0x30fb, 0xff0e, 0x9ede, 0xc810, 0x66b, 0x66b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const MINUS_CHAR = [0x2d, 0xff0d, 0xff0d, 0x8d1f, 0x5069, 0x2212, 0x2212, 0xff0d, 0x5069, 0xff0d, 0, 0x2212, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const SEPARATOR_CHAR = [0x2c, 0xff0c, 0x3001, 0x3001, 0x3001, 0x3001, 0x3001, 0x3001, 0x3001, 0x3001, 0, 0x66c, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

const EXP_6 = [12, 8, 4, 3, 2, 1];
const MULT_6 = [
  [0x5146, 0x4ebf, 0x4e07, 0x5343, 0x767e, 0x5341],
  [0x5146, 0x4ebf, 0x4e07, 0x4edf, 0x4f70, 0x62fe],
  [0x5146, 0x5104, 0x842c, 0x5343, 0x767e, 0x5341],
  [0x5146, 0x5104, 0x842c, 0x4edf, 0x4f70, 0x62fe],
  [0x5146, 0x5104, 0x4e07, 0x5343, 0x767e, 0x5341],
  [0xc870, 0xc5b5, 0xb9cc, 0xcc9c, 0xbc31, 0xc2ed],
  [0x5146, 0x5104, 0x4e07, 0x5343, 0x767e, 0x5341],
  [0x5146, 0x5104, 0x842c, 0x9621, 0x767e, 0x62fe],
];
const EXP_2 = [8, 4];
const MULT_2 = [[0x4ebf, 0x4e07], [0x4ebf, 0x4e07], [0x5104, 0x842c], [0x5104, 0x842c], [0x5104, 0x4e07], [0xc5b5, 0xb9cc], [0x5104, 0x4e07], [0x5104, 0x842c]];
const EXP_7 = [16, 12, 8, 4, 3, 2, 1];
const MULT_7 = [
  [0x4eac, 0x5146, 0x4ebf, 0x4e07, 0x5343, 0x767e, 0x5341],
  [0x4eac, 0x5146, 0x4ebf, 0x4e07, 0x4edf, 0x4f70, 0x62fe],
  [0x4eac, 0x5146, 0x5104, 0x842c, 0x5343, 0x767e, 0x5341],
  [0x4eac, 0x5146, 0x5104, 0x842c, 0x4edf, 0x4f70, 0x62fe],
  [0x4eac, 0x5146, 0x5104, 0x4e07, 0x5343, 0x767e, 0x5341],
  [0x4eac, 0xc870, 0xc5b5, 0xb9cc, 0xcc9c, 0xbc31, 0xc2ed],
  [0x4eac, 0x5146, 0x5104, 0x4e07, 0x5343, 0x767e, 0x5341],
  [0x4eac, 0x5146, 0x5104, 0x842c, 0x9621, 0x767e, 0x62fe],
];
const M_LOWER_ZH = 0, M_UPPER_ZH = 1, M_LOWER_ZH_TW = 2, M_UPPER_ZH_TW = 3, M_LOWER_KO = 4, M_HANGUL_KO = 5, M_MODERN_JA = 6, M_TRAD_JA = 7;
const OMIT_ZERO = 1, OMIT_ONLY_ZERO = 2, OMIT_ONE = 0x1fc;
const OMIT_ALL = OMIT_ZERO | OMIT_ONE | OMIT_ONLY_ZERO, OMIT_ZERO_ONE = OMIT_ZERO | OMIT_ONE, OMIT_ZERO_ONE_67 = OMIT_ZERO | (1 << 7) | (1 << 8);

interface NumberSpec { number: number; mult: number[]; flag: number; exp: number[] }
const n6 = (number: number, m: number, flag: number): NumberSpec => ({ number, mult: MULT_6[m]!, flag, exp: EXP_6 });
const n7 = (number: number, m: number, flag: number): NumberSpec => ({ number, mult: MULT_7[m]!, flag, exp: EXP_7 });
const n2 = (number: number, m: number, flag: number): NumberSpec => ({ number, mult: MULT_2[m]!, flag, exp: EXP_2 });
const NATNUM4 = [n6(LOWER_ZH, M_LOWER_ZH, 0), n6(LOWER_ZH, M_LOWER_ZH_TW, 0), n7(MODERN_JA, M_MODERN_JA, OMIT_ZERO_ONE_67), n6(LOWER_KO, M_LOWER_KO, OMIT_ZERO)];
const NATNUM5 = [n6(UPPER_ZH, M_UPPER_ZH, 0), n6(UPPER_ZH_TW, M_UPPER_ZH_TW, 0), n7(TRAD_JA, M_TRAD_JA, OMIT_ZERO_ONE_67), n6(UPPER_KO, M_UPPER_ZH_TW, OMIT_ZERO)];
const NATNUM6 = [n6(FULL, M_LOWER_ZH, 0), n6(FULL, M_LOWER_ZH_TW, 0), n7(FULL, M_MODERN_JA, OMIT_ZERO_ONE_67), n6(FULL, M_HANGUL_KO, OMIT_ZERO)];
const NATNUM7 = [n6(LOWER_ZH, M_LOWER_ZH, OMIT_ALL), n6(LOWER_ZH, M_LOWER_ZH_TW, OMIT_ALL), n2(MODERN_JA, M_MODERN_JA, OMIT_ZERO_ONE), n6(LOWER_KO, M_LOWER_KO, OMIT_ALL)];
const NATNUM8 = [n6(UPPER_ZH, M_UPPER_ZH, OMIT_ALL), n6(UPPER_ZH_TW, M_UPPER_ZH_TW, OMIT_ALL), n2(TRAD_JA, M_TRAD_JA, OMIT_ZERO_ONE), n6(UPPER_KO, M_UPPER_ZH_TW, OMIT_ALL)];
const NATNUM10 = n6(HANGUL_KO, M_HANGUL_KO, OMIT_ZERO);
const NATNUM11 = n6(HANGUL_KO, M_HANGUL_KO, OMIT_ALL);
const NATNUM1 = [LOWER_ZH, LOWER_ZH, MODERN_JA, LOWER_KO, HE, INDIC_AR, TH, HI, OR, MR, BN, PA, GU, TA, TE, KN, ML, LO, BO, MY, KM, MN, NE, DZ, EAST_INDIC_AR];
const NATNUM2 = [UPPER_ZH, UPPER_ZH_TW, TRAD_JA, UPPER_KO, HE];

function langNumber(lang: LangId): number | null {
  const p = primary(lang);
  if (p === 0x04) return lang === 0x404 || lang === 0xc04 || lang === 0x1404 ? 1 : 0;
  const table: Record<number, number> = { 0x11: 2, 0x12: 3, 0x0d: 4, 0x01: 5, 0x1e: 6, 0x39: 7, 0x48: 8, 0x4e: 9, 0x45: 10, 0x46: 11, 0x47: 12, 0x49: 13, 0x4a: 14, 0x4b: 15, 0x4c: 16, 0x54: 17, 0x55: 19, 0x53: 20, 0x50: 21, 0x61: 22, 0x29: 24 };
  if (p === 0x51) return lang === 0xc51 ? 23 : 18;
  return table[p] ?? null;
}

export function isValidNatNum(lang: LangId, mode: number): boolean {
  const n = langNumber(lang);
  if (mode === 0 || mode === 3) return true;
  if (mode === 1) return n !== null;
  if (mode === 2 && n === 4) return true;
  if (mode === 2 || (mode >= 4 && mode <= 8)) return n !== null && n < 4;
  if (mode >= 9 && mode <= 11) return n === 3;
  return false;
}

/** `SvNumberNatNum::MapDBNumToNatNum`. */
export function mapDbNum(db: number, lang: LangId, date: boolean): number {
  const p = primary(lang);
  if (date) return db === 4 && p === 0x12 ? 9 : db <= 3 ? db : 0;
  const t: Record<string, number> = { "1,4": 4, "1,17": 1, "1,18": 1, "2,4": 5, "2,17": 4, "2,18": 2, "3,4": 6, "3,17": 5, "3,18": 3, "4,17": 7, "4,18": 9 };
  return t[`${db},${p}`] ?? 0;
}

export interface Separators { decimal: string; group: string }

const isNum = (c: number): boolean => c >= 0x30 && c <= 0x39;
const orKeep = (x: number, c: number): number => (x !== 0 ? x : c);

class Chars {
  readonly decimal: [number, number];
  readonly separator: [number, number];
  constructor(sep: Separators) {
    const full = (c: number, dflt: number): number => (c >= 0x21 && c <= 0x7e ? c + 0xfee0 : dflt);
    const d = sep.decimal.charCodeAt(0);
    const g = sep.group.charCodeAt(0);
    this.decimal = [d, full(d, 0xff0e)];
    this.separator = [g, full(g, 0xff0c)];
  }
  isDecimal(c: number): boolean { return c === this.decimal[0]; }
  isSeparator(c: number): boolean { return c === this.separator[0]; }
  decimalOf(n: number): number { return n < 2 ? this.decimal[n as 0 | 1] : DECIMAL_CHAR[n]!; }
  separatorOf(n: number): number { return n < 2 ? this.separator[n as 0 | 1] : SEPARATOR_CHAR[n]!; }
}

function asciiToNativeChar(src: number[], num: number, ch: Chars): number[] {
  const n = src.length;
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    const c = src[i]!;
    if (isNum(c)) out.push(NUMBER_CHAR[num]![c - 0x30]!);
    else if (i + 1 < n && isNum(src[i + 1]!)) {
      if (i > 0 && isNum(src[i - 1]!) && ch.isSeparator(c)) out.push(orKeep(ch.separatorOf(num), c));
      else if (ch.isDecimal(c)) out.push(orKeep(ch.decimalOf(num), c));
      else if (c === 0x2d) out.push(orKeep(MINUS_CHAR[num]!, c));
      else out.push(c);
    } else out.push(c);
  }
  return out;
}

function numberMaker(s: number[], begin: number, len: number, dst: number[], multi: number | null, num: NumberSpec, nc: number[]): boolean {
  const multiChar = multi === null ? null : num.mult[multi]!;
  const lastExp = num.exp[num.exp.length - 1]!;
  if (len <= lastExp) {
    if (lastExp > 1) {
      let notZero = false;
      for (let k = 0; k < len; k++, begin++) {
        if (notZero || s[begin] !== 0x30) {
          dst.push(nc[s[begin]! - 0x30]!);
          notZero = true;
        }
      }
      if (notZero && multiChar !== null) dst.push(multiChar);
      return notZero;
    } else if (s[begin] !== 0x30) {
      const omitOne = multi !== null && (num.flag & (1 << (2 + multi))) !== 0 && s[begin] === 0x31;
      if (!omitOne) dst.push(nc[s[begin]! - 0x30]!);
      if (multiChar !== null) dst.push(multiChar);
    } else if ((num.flag & OMIT_ZERO) === 0 && dst.length > 0 && dst[dst.length - 1] !== nc[0]) {
      dst.push(nc[0]!);
    }
    return s[begin] !== 0x30;
  }
  const count = num.exp.length;
  let printPower = false;
  for (let i = 1; i <= count; i++) {
    const sub = i === count ? 0 : num.exp[i]!;
    if (len > sub) {
      const tmp = len - sub;
      printPower = numberMaker(s, begin, tmp, dst, i === count ? null : i, num, nc) || printPower;
      begin += tmp;
      len -= tmp;
    }
  }
  if (printPower) {
    if (lastExp === 1 && dst[dst.length - 1] === nc[0]) dst.pop();
    if (multiChar !== null) dst.push(multiChar);
  }
  return printPower;
}

function asciiToNative(src: number[], num: NumberSpec, ch: Chars): number[] {
  const nc = NUMBER_CHAR[num.number]!;
  const n = src.length;
  const out: number[] = [];
  let digits: number[] = [];
  let doDecimal = false;
  const lastExp = num.exp[num.exp.length - 1]!;
  const e0 = num.exp[0]!;
  const nextIsNum = (i: number): boolean => i + 1 < n && isNum(src[i + 1]!);
  for (let i = 0; i <= n; i++) {
    if (i < n && isNum(src[i]!)) {
      if (doDecimal) out.push(nc[src[i]! - 0x30]!);
      else digits.push(src[i]!);
      continue;
    }
    const len = digits.length;
    if (len > 0) {
      if (i < n && ch.isSeparator(src[i]!) && nextIsNum(i)) continue;
      let notZero = false;
      for (let begin = 0, end = len % e0; end <= len; begin = end, end += e0) {
        if (end === 0) continue;
        const before = out.length;
        notZero = numberMaker(digits, begin, end - begin, out, end !== len ? 0 : null, num, nc) || notZero;
        if (lastExp === 1 && out[out.length - 1] === nc[0]) out.pop();
        if (notZero && before === out.length && end !== len) out.push(num.mult[0]!);
      }
      if (!notZero && (num.flag & OMIT_ONLY_ZERO) === 0) out.push(nc[0]!);
      digits = [];
    }
    if (i < n) {
      const c = src[i]!;
      doDecimal = !doDecimal && ch.isDecimal(c) && nextIsNum(i);
      if (doDecimal) out.push(orKeep(ch.decimalOf(num.number), c));
      else if (c === 0x2d && nextIsNum(i)) out.push(orKeep(MINUS_CHAR[num.number]!, c));
      else if (ch.isSeparator(c) && nextIsNum(i)) out.push(orKeep(ch.separatorOf(num.number), c));
      else out.push(c);
    }
  }
  return out;
}

function nativeToAscii(src: number[], ch: Chars): number[] {
  const digitOf = (c: number): number | null => {
    for (let k = 0; k < NUMBER_CHAR.length; k++) {
      if (k === HE) continue;
      const d = NUMBER_CHAR[k]!.indexOf(c);
      if (d >= 0) return 0x30 + d;
    }
    return null;
  };
  const out: number[] = [];
  for (let i = 0; i < src.length; i++) {
    const c = src[i]!;
    const d = digitOf(c);
    if (d !== null) {
      out.push(d);
      continue;
    }
    const before = i + 1 < src.length && digitOf(src[i + 1]!) !== null;
    const inTable = (t: number[], from: number): boolean => c !== 0 && t.slice(from).includes(c);
    if (before && inTable(SEPARATOR_CHAR, 2)) out.push(ch.separator[0]);
    else if (before && inTable(DECIMAL_CHAR, 2)) out.push(ch.decimal[0]);
    else if (before && inTable(MINUS_CHAR, 1)) out.push(0x2d);
    else out.push(c);
  }
  return out;
}

const HEBREW: [number, number][] = [[0x5ea, 400], [0x5ea, 400], [0x5e9, 300], [0x5e8, 200], [0x5e7, 100], [0x5e6, 90], [0x5e4, 80], [0x5e2, 70], [0x5e1, 60], [0x5e0, 50], [0x5de, 40], [0x5dc, 30], [0x5db, 20], [0x5d9, 10], [0x5d8, 9], [0x5d7, 8], [0x5d6, 7], [0x5d5, 6], [0x5d4, 5], [0x5d3, 4], [0x5d2, 3], [0x5d1, 2], [0x5d0, 1]];

function makeHebrew(value: number, out: number[], isLast: boolean, geresh: boolean): void {
  let num = value % 1000;
  if (value > 1000) {
    makeHebrew(Math.floor(value / 1000), out, num !== 0, geresh);
    out.push(0x20);
  }
  if (num === 0) {
    const w = value === 1000 ? [0x5d0, 0x5dc, 0x5e3] : isLast ? [0x5d0, 0x5dc, 0x5e4, 0x5d9, 0x5dd] : [0x5d0, 0x5dc, 0x5e4, 0x5d9];
    out.push(...w);
    return;
  }
  let nChars = 0;
  for (let j = 0; num > 0 && j < HEBREW.length; j++) {
    if (num >= HEBREW[j]![1]) {
      nChars++;
      if (num === 15 || num === 16) j++;
      num -= HEBREW[j]![1];
      out.push(HEBREW[j]![0]);
    }
  }
  if (geresh) {
    if (nChars > 1) out.splice(out.length - 1, 0, 0x5f4);
    else if (nChars === 1) out.push(0x5f3);
  }
}

function hebrew(s: string, geresh: boolean, ch: Chars): string {
  let value = 0;
  let count = 0;
  let rest = s.length;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (isNum(c)) {
      if (++count >= 20) return s;
      value = value * 10 + (c - 0x30);
    } else if ((ch.isSeparator(c) && count > 0) || (c === 0x2d && count === 0)) continue;
    else {
      rest = i;
      break;
    }
  }
  if (value === 0) return s;
  const out: number[] = [];
  makeHebrew(value, out, true, geresh);
  return String.fromCharCode(...out) + s.slice(rest);
}

/** `NativeNumberSupplier::getNativeNumberString`. */
export function transliterate(s: string, lang: LangId, mode: number, sep: Separators): string {
  if (!isValidNatNum(lang, mode)) return s;
  if (mode !== 0 && !/[0-9]/.test(s)) return s;
  const ln = langNumber(lang) ?? 0;
  const ch = new Chars(sep);
  const src = Array.from({ length: s.length }, (_, i) => s.charCodeAt(i));
  let out: number[];
  if (mode === 0) out = nativeToAscii(src, ch);
  else if (mode === 1 || mode === 2 || mode === 3 || mode === 9) {
    const num = mode === 1 ? NATNUM1[ln]! : mode === 2 ? NATNUM2[Math.min(ln, 4)]! : mode === 3 ? FULL : HANGUL_KO;
    if (num === HE) return hebrew(s, mode === 2, ch);
    out = asciiToNativeChar(src, num, ch);
  } else {
    const i = Math.min(ln, 3);
    const spec = mode === 4 ? NATNUM4[i]! : mode === 5 ? NATNUM5[i]! : mode === 6 ? NATNUM6[i]! : mode === 7 ? NATNUM7[i]! : mode === 8 ? NATNUM8[i]! : mode === 10 ? NATNUM10 : NATNUM11;
    out = asciiToNative(src, spec, ch);
  }
  let r = "";
  for (const c of out) r += String.fromCharCode(c);
  return r;
}

// --- Calendars (calendar.rs) ---------------------------------------------

export type CalendarId = "gregorian" | "gengou" | "ROC" | "buddhist" | "hanja" | "hijri" | "jewish";

export function calendarFromName(name: string): CalendarId | null {
  const all: CalendarId[] = ["gregorian", "gengou", "ROC", "buddhist", "hanja", "hijri", "jewish"];
  return all.find((c) => c.toLowerCase() === name.trim().toLowerCase()) ?? null;
}

export function otherCalendar(lang: LangId): CalendarId | null {
  switch (primary(lang)) {
    case 0x11: return "gengou";
    case 0x04: return lang === 0x404 ? "ROC" : null;
    case 0x12: return "hanja";
    case 0x1e: return "buddhist";
    case 0x01: return lang === 0x2001 ? null : "hijri";
    case 0x0d: return "jewish";
    default: return null;
  }
}

export const hasCalendar = (lang: LangId, c: CalendarId): boolean => c === "gregorian" || otherCalendar(lang) === c;

const ERAS: Partial<Record<CalendarId, [number, number, number][]>> = {
  gengou: [[1868, 1, 1], [1912, 7, 30], [1926, 12, 25], [1989, 1, 8], [2019, 5, 1]],
  ROC: [[1912, 1, 1]],
  buddhist: [[-542, 1, 1]],
};

export interface Fields { era: number; year: number; month: number; day: number; weekday: number }

const lt = (a: [number, number, number], b: [number, number, number]): boolean => (a[0] !== b[0] ? a[0] < b[0] : a[1] !== b[1] ? a[1] < b[1] : a[2] < b[2]);

const RAD = 0.017453292519943295, SYN = 29.53058868, JD1900 = 2415020.75933;

function newMoon(n: number): number {
  const k = n, t = k / 1236.85, t2 = t * t, t3 = t2 * t;
  let jd = JD1900 + SYN * k - 0.0001178 * t2 - 0.000000155 * t3 + 0.00033 * Math.sin(RAD * (166.56 + 132.87 * t - 0.009173 * t2));
  const sa = RAD * (359.2242 + 29.10535608 * k - 0.0000333 * t2 - 0.00000347 * t3);
  const ma = RAD * (306.0253 + 385.81691806 * k + 0.0107306 * t2 + 0.00001236 * t3);
  const tf = RAD * 2.0 * (21.2964 + 390.67050646 * k - 0.0016528 * t2 - 0.00000239 * t3);
  const xtra = (0.1734 - 0.000393 * t) * Math.sin(sa) + 0.0021 * Math.sin(sa * 2) - 0.4068 * Math.sin(ma) + 0.0161 * Math.sin(2 * ma) - 0.0004 * Math.sin(3 * ma) + 0.0104 * Math.sin(tf) - 0.0051 * Math.sin(sa + ma) - 0.0074 * Math.sin(sa - ma) + 0.0004 * Math.sin(tf + sa) - 0.0004 * Math.sin(tf - sa) - 0.0006 * Math.sin(tf + ma) + 0.001 * Math.sin(tf - ma) + 0.0005 * Math.sin(sa + 2 * ma);
  jd += xtra - (0.41 + 1.2053 * t + 0.4992 * t2) / 1440;
  return jd;
}

function hijri(julday: number): [number, number, number] {
  let newsyn = Math.trunc(0.5 + (julday - JD1900) / SYN);
  const prevday = julday - 0.5;
  let newjd = 0;
  for (let guard = 0; guard < 65; guard++) {
    newjd = newMoon(newsyn);
    newsyn--;
    if (newjd <= prevday) break;
  }
  newsyn++;
  const syndiff = newsyn - 1252;
  const day = Math.trunc(julday - newjd + 0.5);
  let month = (syndiff % 12) + 1;
  let year = 1422 + Math.trunc(syndiff / 12);
  if (syndiff !== 0 && month <= 0) {
    month += 12;
    year--;
  }
  if (year <= 0) year--;
  return [day, month, year];
}

const hebLeap = (y: number): boolean => (((7 * y + 1) % 19) + 19) % 19 < 7;
function hebElapsed(year: number): number {
  const months = 235 * Math.trunc((year - 1) / 19) + 12 * ((year - 1) % 19) + Math.trunc((7 * ((year - 1) % 19) + 1) / 19);
  const parts = 204 + 793 * (months % 1080);
  const hours = 5 + 12 * months + 793 * Math.trunc(months / 1080) + Math.trunc(parts / 1080);
  const day = 1 + 29 * months + Math.trunc(hours / 24);
  const p = 1080 * (hours % 24) + (parts % 1080);
  const alt = p >= 19440 || (day % 7 === 2 && p >= 9924 && !hebLeap(year)) || (day % 7 === 1 && p >= 16789 && hebLeap(year - 1)) ? day + 1 : day;
  return [0, 3, 5].includes(alt % 7) ? alt + 1 : alt;
}
const hebDays = (y: number): number => hebElapsed(y + 1) - hebElapsed(y);
function hebMonthLen(m: number, y: number): number {
  const short = [2, 4, 6, 10, 13].includes(m) || (m === 8 && hebDays(y) % 10 !== 5) || (m === 9 && hebDays(y) % 10 === 3) || (m === 12 && !hebLeap(y));
  return short ? 29 : 30;
}
function hebAbs(m: number, d: number, y: number): number {
  let doy = d;
  if (m < 7) {
    for (let k = 7; k <= (hebLeap(y) ? 13 : 12); k++) doy += hebMonthLen(k, y);
    for (let k = 1; k < m; k++) doy += hebMonthLen(k, y);
  } else for (let k = 7; k < m; k++) doy += hebMonthLen(k, y);
  return doy + hebElapsed(y) - 1373429;
}
function hebrewDate(abs: number): [number, number, number] {
  let year = Math.trunc((abs - 1373429) / 366);
  while (abs >= hebAbs(7, 1, year + 1)) year++;
  let month = abs < hebAbs(1, 1, year) ? 7 : 1;
  while (abs > hebAbs(month, hebMonthLen(month, year), year)) month++;
  return [year, month, abs - hebAbs(month, 1, year) + 1];
}

/** The calendar fields of the Gregorian date y-m-d (`unixDays` after 1970-01-01). */
export function calendarFields(cal: CalendarId, y: number, m: number, d: number, unixDays: number, wd: number): Fields {
  const greg: Fields = { era: y > 0 ? 1 : 0, year: y > 0 ? y : 1 - y, month: m, day: d, weekday: wd };
  const eras = ERAS[cal];
  if (eras) {
    let e = eras.findIndex((x) => lt([y, m, d], x));
    if (e < 0) e = eras.length;
    return { ...greg, era: e, year: e === 0 ? eras[0]![0] - y : y - eras[e - 1]![0] + 1 };
  }
  if (cal === "hijri") {
    const [hd, hm, hy] = hijri(unixDays + 2440588);
    return { era: hy >= 1 ? 1 : 0, year: Math.abs(hy), month: hm, day: hd, weekday: wd };
  }
  if (cal === "jewish") {
    const [hy, hm, hd] = hebrewDate(unixDays + 719163);
    return { era: hy > 0 ? 1 : 0, year: hy <= 0 ? 1 - hy : hy, month: hm, day: hd, weekday: wd };
  }
  return greg;
}

type Names = [string, string][];
const JA_DAYS: Names = [["日", "日曜日"], ["月", "月曜日"], ["火", "火曜日"], ["水", "水曜日"], ["木", "木曜日"], ["金", "金曜日"], ["土", "土曜日"]];
const ZH_DAYS: Names = [["日", "星期日"], ["一", "星期一"], ["二", "星期二"], ["三", "星期三"], ["四", "星期四"], ["五", "星期五"], ["六", "星期六"]];
const ZH_MONTHS: Names = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十", "十一", "十二"].map((x) => [`${x}月`, `${x}月`]);
const HANJA_MONTHS: Names = Array.from({ length: 12 }, (_, i) => [String(i + 1), String(i + 1)]);
const TH_DAYS: Names = [["อา.", "อาทิตย์"], ["จ.", "จันทร์"], ["อ.", "อังคาร"], ["พ.", "พุธ"], ["พฤ.", "พฤหัสบดี"], ["ศ.", "ศุกร์"], ["ส.", "เสาร์"]];
const TH_MONTHS: Names = [["ม.ค.", "มกราคม"], ["ก.พ.", "กุมภาพันธ์"], ["มี.ค.", "มีนาคม"], ["เม.ย.", "เมษายน"], ["พ.ค.", "พฤษภาคม"], ["มิ.ย.", "มิถุนายน"], ["ก.ค.", "กรกฎาคม"], ["ส.ค.", "สิงหาคม"], ["ก.ย.", "กันยายน"], ["ต.ค.", "ตุลาคม"], ["พ.ย.", "พฤศจิกายน"], ["ธ.ค.", "ธันวาคม"]];
const AR_DAYS: Names = [["ح", "الأحد"], ["ن", "الاثنين"], ["ث", "الثلاثاء"], ["ر", "الأربعاء"], ["خ", "الخميس"], ["ج", "الجمعة"], ["س", "السبت"]];
const HIJRI_MONTHS: Names = ["محرم", "صفر", "ربيع الأول", "ربيع الآخر", "جمادى الأولى", "جمادى الآخرة", "رجب", "شعبان", "رمضان", "شوال", "ذو القعدة", "ذو الحجة"].map((x) => [x, x]);
const HE_DAYS: Names = [["א", "ראשון"], ["ב", "שני"], ["ג", "שלישי"], ["ד", "רביעי"], ["ה", "חמישי"], ["ו", "שישי"], ["ש", "שבת"]];
const JEWISH_MONTHS: Names = ["ניסן", "אייר", "סיון", "תמוז", "אב", "אלול", "תשרי", "חשון", "כסליו", "טבת", "שבט", "אדר", "אדר-ב"].map((x) => [x, x]);

const ERA_NAMES: Partial<Record<CalendarId, Names>> = {
  gengou: [["?.", "元号のサポート範囲外"], ["明", "明治"], ["大", "大正"], ["昭", "昭和"], ["平", "平成"], ["令", "令和"]],
  ROC: [["民國前", "中華民國前"], ["民國", "中華民國"]],
  buddhist: [["ปีก่อน พ.ศ. ที่", "ปีก่อนพุทธศักราชที่"], ["พ.ศ.", "พุทธศักราช"]],
  hanja: [["紀元前", "西曆紀元前"], ["西紀", "西曆紀元"]],
  hijri: [["ه‍", "قبل الهجرة"], ["", "هجري"]],
  jewish: [['לפנה"ס', 'לפנה"ס'], ['לסה"נ', 'לסה"נ']],
};

/** Gregorian era names by primary language (Tally locale data and AOO localedata). */
export function gregorianEras(lang: LangId): Names | null {
  switch (primary(lang)) {
    case 0x09: return [["BC", "Before Christ"], ["AD", "Anno Domini"]];
    case 0x11: return [["紀元前", "紀元前"], ["西暦", "西暦"]];
    case 0x04: return lang === 0x404 ? [["西元前", "西元前"], ["", "西元"]] : [["前", "公元前"], ["", "公元"]];
    case 0x12: return [["기원전", "서력기원전"], ["서기", "서력기원"]];
    case 0x1e: return [["ปีก่อน ค.ศ. ที่", "ปีก่อนคริสต์ศักราชที่"], ["ค.ศ.", "คริสต์ศักราช"]];
    case 0x01: return [["ق.م", "قبل الميلاد"], ["م", "ميلادي"]];
    case 0x0d: return [['לפנה"ס', 'לפנה"ס'], ['לסה"נ', 'לסה"נ']];
    case 0x07: return [["v. Chr.", "vor Christus"], ["n. Chr.", "nach Christus"]];
    case 0x0c: return [["av. J.-C.", "avant Jésus-Christ"], ["ap. J.-C.", "après Jésus-Christ"]];
    case 0x0a: return [["a. C.", "antes de Cristo"], ["d. C.", "después de Cristo"]];
    default: return null;
  }
}

export const eraNames = (cal: CalendarId, lang: LangId): Names | null => (cal === "gregorian" ? gregorianEras(lang) : ERA_NAMES[cal] ?? null);

export function dayNames(cal: CalendarId): Names | null {
  return cal === "gengou" || cal === "hanja" ? JA_DAYS : cal === "ROC" ? ZH_DAYS : cal === "buddhist" ? TH_DAYS : cal === "hijri" ? AR_DAYS : cal === "jewish" ? HE_DAYS : null;
}

export function monthNames(cal: CalendarId): Names | null {
  return cal === "ROC" ? ZH_MONTHS : cal === "hanja" ? HANJA_MONTHS : cal === "buddhist" ? TH_MONTHS : cal === "hijri" ? HIJRI_MONTHS : cal === "jewish" ? JEWISH_MONTHS : null;
}

export type NumCode = "shortDay" | "longDay" | "shortMonth" | "longMonth" | "shortYear" | "longYear" | "quarter";

function natNumForCalendar(lang: LangId, code: NumCode, mode: number, value: number): number {
  const short = ((code === "shortYear" || code === "longYear") && value >= 100) || code === "quarter";
  const p = primary(lang);
  if (p === 0x04 || p === 0x11 || p === 0x12) {
    const ja = p === 0x11, ko = p === 0x12;
    if (mode === 1 && !short) return ja ? 4 : 7;
    if (mode === 2 && !short) return ja ? 5 : 8;
    if (mode >= 1 && mode <= 3) return mode;
    if (mode === 4 && ko) return short ? 9 : 11;
    return 0;
  }
  return mode;
}

export function calendarNatNum(text: string, cal: CalendarId, lang: LangId, code: NumCode, value: number, mode: number, sep: Separators): string {
  const m = cal === "jewish" && code !== "quarter" ? 2 : mode;
  if (m === 0) return text;
  const mm = natNumForCalendar(lang, code, m, value);
  return mm === 0 ? text : transliterate(text, lang, mm, sep);
}

const pad2 = (v: number): string => String(v).padStart(2, "0");

/** `Calendar_gregorian::getDisplayString` for a numeric field. */
export function displayNumber(cal: CalendarId, lang: LangId, f: Fields, code: NumCode, mode: number, gannen: boolean, sep: Separators): string {
  const yearCode = code === "shortYear" || code === "longYear";
  if (cal === "gengou" && yearCode && f.year === 1 && (gannen || mode === 1 || mode === 2)) return "元";
  const value = code === "shortDay" || code === "longDay" ? f.day : yearCode ? f.year : f.month;
  let text: string;
  if (code === "shortDay" || code === "shortMonth" || code === "quarter") text = String(value);
  else if (code === "longDay" || code === "longMonth") text = pad2(value);
  else if (code === "longYear") text = cal === "gengou" ? pad2(value) : String(value);
  else if (cal === "jewish") text = String(value % 1000);
  else text = value < 100 || ERAS[cal] ? String(value) : pad2(value % 100);
  return calendarNatNum(text, cal, lang, code, value, mode, sep);
}
