//! Native number transliteration (`[NatNumN]`, `[DBNumN]`).
//!
//! A port of AOO 4.1 `i18npool/source/nativenumber/nativenumbersupplier.cxx`
//! (`NativeNumberSupplier::getNativeNumberString`, `AsciiToNativeChar`,
//! `AsciiToNative`, `getHebrewNativeNumberString`, `isValidNatNum`,
//! `convertToXmlAttributes` / `convertFromXmlAttributes`) with the tables of
//! `data/numberchar.h`, and of the DBNum ↔ NatNum mapping of
//! `svl/source/numbers/zformat.cxx` (`SvNumberNatNum::MapDBNumToNatNum`).
//!
//! The modes per language (`isValidNatNum`):
//!
//! | NatNum | meaning | languages |
//! |---|---|---|
//! | 0 | native digits back to ASCII | all |
//! | 1 | native digits (lower) | zh, ja, ko, he, ar, th, hi, … (25) |
//! | 2 | native digits (upper / financial; Hebrew with geresh) | zh, ja, ko, he |
//! | 3 | full-width digits | all |
//! | 4 / 5 / 6 | numbers spelled with units (lower / upper / full-width) | zh, ja, ko |
//! | 7 / 8 | the same, short form (`一万二千` instead of `一万二千〇`) | zh, ja, ko |
//! | 9 / 10 / 11 | Hangul digits / long / short spelled | ko |
//!
//! Any other combination leaves the text unchanged.

use std::borrow::Cow;

/// A Windows language id (LCID, 16 bits) as AOO's `LanguageType`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct LangId(pub u16);

impl LangId {
    /// Japanese (ja-JP).
    pub const JA: LangId = LangId(0x0411);
    /// Simplified Chinese (zh-CN).
    pub const ZH_CN: LangId = LangId(0x0804);
    /// Traditional Chinese (zh-TW).
    pub const ZH_TW: LangId = LangId(0x0404);
    /// Korean (ko-KR).
    pub const KO: LangId = LangId(0x0412);
    /// Hebrew (he-IL).
    pub const HE: LangId = LangId(0x040D);
    /// Arabic (ar-SA).
    pub const AR_SA: LangId = LangId(0x0401);
    /// Thai (th-TH).
    pub const TH: LangId = LangId(0x041E);
    /// English (en-US).
    pub const EN_US: LangId = LangId(0x0409);

    /// The 10-bit primary language.
    pub fn primary(self) -> u16 {
        self.0 & 0x03FF
    }

    /// Parses a hex locale id as in `[$-411]`; Excel's calendar and numeral
    /// bytes above the low 16 bits are ignored here.
    pub fn from_hex(hex: &str) -> Option<LangId> {
        let v = u32::from_str_radix(hex.trim(), 16).ok()?;
        let id = (v & 0xFFFF) as u16;
        (id != 0).then_some(LangId(id))
    }

    /// The upper-case hex spelling (`411`).
    pub fn hex(self) -> String {
        format!("{:X}", self.0)
    }

    /// Whether this is Chinese, Japanese or Korean.
    pub fn is_cjk(self) -> bool {
        matches!(self.primary(), 0x04 | 0x11 | 0x12)
    }
}

/// Primary language ids used below.
const P_ZH: u16 = 0x04;
const P_JA: u16 = 0x11;
const P_KO: u16 = 0x12;

/// A `[NatNumN]` or `[DBNumN]` modifier of a section.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct NatNum {
    /// The number after `NatNum` / `DBNum`.
    pub num: u8,
    /// Written as `[DBNumN]` (mapped to a NatNum per language).
    pub dbnum: bool,
}

impl NatNum {
    /// The effective NatNum mode for a language (`SvNumberNatNum::GetNatNum`):
    /// a DBNum is mapped with [`map_dbnum_to_natnum`].
    pub fn mode(self, lang: LangId, date: bool) -> u8 {
        if self.dbnum {
            map_dbnum_to_natnum(self.num, lang, date)
        } else {
            self.num
        }
    }
}

/// `SvNumberNatNum::MapDBNumToNatNum` (zformat.cxx).
pub fn map_dbnum_to_natnum(dbnum: u8, lang: LangId, date: bool) -> u8 {
    let p = lang.primary();
    if date {
        if dbnum == 4 && p == P_KO {
            9
        } else if dbnum <= 3 {
            dbnum
        } else {
            0
        }
    } else {
        match (dbnum, p) {
            (1, P_ZH) => 4,
            (1, P_JA | P_KO) => 1,
            (2, P_ZH) => 5,
            (2, P_JA) => 4,
            (2, P_KO) => 2,
            (3, P_ZH) => 6,
            (3, P_JA) => 5,
            (3, P_KO) => 3,
            (4, P_JA) => 7,
            (4, P_KO) => 9,
            _ => 0,
        }
    }
}

/// `SvNumberNatNum::MapNatNumToDBNum` (zformat.cxx); 0 means no DBNum.
pub fn map_natnum_to_dbnum(natnum: u8, lang: LangId, date: bool) -> u8 {
    let p = lang.primary();
    if date {
        if natnum == 9 && p == P_KO {
            4
        } else if natnum <= 3 {
            natnum
        } else {
            0
        }
    } else {
        match (natnum, p) {
            (1, P_JA | P_KO) => 1,
            (2, P_KO) => 2,
            (3, P_KO) => 3,
            (4, P_ZH) => 1,
            (4, P_JA) => 2,
            (5, P_ZH) => 2,
            (5, P_JA) => 3,
            (6, P_ZH) => 3,
            (7, P_JA) => 4,
            (9, P_KO) => 4,
            _ => 0,
        }
    }
}

// --- numberchar.h ---------------------------------------------------------

const HALF_WIDTH: usize = 0;
const FULL_WIDTH: usize = 1;
const LOWER_ZH: usize = 2;
const UPPER_ZH: usize = 3;
const UPPER_ZH_TW: usize = 4;
const MODERN_JA: usize = 5;
const TRADITIONAL_JA: usize = 6;
const LOWER_KO: usize = 7;
const UPPER_KO: usize = 8;
const HANGUL_KO: usize = 9;
const INDIC_AR: usize = 10;
const EAST_INDIC_AR: usize = 11;
const HI: usize = 12;
const TH: usize = 13;
const OR: usize = 14;
const MR: usize = 15;
const BN: usize = 16;
const PA: usize = 17;
const GU: usize = 18;
const TA: usize = 19;
const TE: usize = 20;
const KN: usize = 21;
const ML: usize = 22;
const LO: usize = 23;
const BO: usize = 24;
const MY: usize = 25;
const KM: usize = 26;
const MN: usize = 27;
const HE: usize = 28;
const NE: usize = 29;
const DZ: usize = 30;

/// `NumberChar[][10]`, verbatim (including AOO's Oriya and Malayalam code
/// points and the ASCII zero of Tamil).
const NUMBER_CHAR: [[u16; 10]; 31] = [
    [
        0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037, 0x0038, 0x0039,
    ],
    [
        0xFF10, 0xFF11, 0xFF12, 0xFF13, 0xFF14, 0xFF15, 0xFF16, 0xFF17, 0xFF18, 0xFF19,
    ],
    [
        0x3007, 0x4E00, 0x4E8C, 0x4E09, 0x56DB, 0x4E94, 0x516D, 0x4E03, 0x516B, 0x4E5D,
    ],
    [
        0x96F6, 0x58F9, 0x8D30, 0x53C1, 0x8086, 0x4F0D, 0x9646, 0x67D2, 0x634C, 0x7396,
    ],
    [
        0x96F6, 0x58F9, 0x8CB3, 0x53C3, 0x8086, 0x4F0D, 0x9678, 0x67D2, 0x634C, 0x7396,
    ],
    [
        0x3007, 0x4E00, 0x4E8C, 0x4E09, 0x56DB, 0x4E94, 0x516D, 0x4E03, 0x516B, 0x4E5D,
    ],
    [
        0x96F6, 0x58F1, 0x5F10, 0x53C2, 0x56DB, 0x4F0D, 0x516D, 0x4E03, 0x516B, 0x4E5D,
    ],
    [
        0x3007, 0x4E00, 0x4E8C, 0x4E09, 0x56DB, 0x4E94, 0x516D, 0x4E03, 0x516B, 0x4E5D,
    ],
    [
        0xF9B2, 0x58F9, 0x8CB3, 0x53C3, 0x56DB, 0x4E94, 0x516D, 0x4E03, 0x516B, 0x4E5D,
    ],
    [
        0xC601, 0xC77C, 0xC774, 0xC0BC, 0xC0AC, 0xC624, 0xC721, 0xCE60, 0xD314, 0xAD6C,
    ],
    [
        0x0660, 0x0661, 0x0662, 0x0663, 0x0664, 0x0665, 0x0666, 0x0667, 0x0668, 0x0669,
    ],
    [
        0x06F0, 0x06F1, 0x06F2, 0x06F3, 0x06F4, 0x06F5, 0x06F6, 0x06F7, 0x06F8, 0x06F9,
    ],
    [
        0x0966, 0x0967, 0x0968, 0x0969, 0x096A, 0x096B, 0x096C, 0x096D, 0x096E, 0x096F,
    ],
    [
        0x0E50, 0x0E51, 0x0E52, 0x0E53, 0x0E54, 0x0E55, 0x0E56, 0x0E57, 0x0E58, 0x0E59,
    ],
    [
        0x0866, 0x0867, 0x0868, 0x0869, 0x086A, 0x086B, 0x086C, 0x086D, 0x086E, 0x086F,
    ],
    [
        0x0966, 0x0967, 0x0968, 0x0969, 0x096A, 0x096B, 0x096C, 0x096D, 0x096E, 0x096F,
    ],
    [
        0x09E6, 0x09E7, 0x09E8, 0x09E9, 0x09EA, 0x09EB, 0x09EC, 0x09ED, 0x09EE, 0x09EF,
    ],
    [
        0x0A66, 0x0A67, 0x0A68, 0x0A69, 0x0A6A, 0x0A6B, 0x0A6C, 0x0A6D, 0x0A6E, 0x0A6F,
    ],
    [
        0x0AE6, 0x0AE7, 0x0AE8, 0x0AE9, 0x0AEA, 0x0AEB, 0x0AEC, 0x0AED, 0x0AEE, 0x0AEF,
    ],
    [
        0x0030, 0x0BE7, 0x0BE8, 0x0BE9, 0x0BEA, 0x0BEB, 0x0BEC, 0x0BED, 0x0BEE, 0x0BEF,
    ],
    [
        0x0C66, 0x0C67, 0x0C68, 0x0C69, 0x0C6A, 0x0C6B, 0x0C6C, 0x0C6D, 0x0C6E, 0x0C6F,
    ],
    [
        0x0CE6, 0x0CE7, 0x0CE8, 0x0CE9, 0x0CEA, 0x0CEB, 0x0CEC, 0x0CED, 0x0CEE, 0x0CEF,
    ],
    [
        0x0DE6, 0x0DE7, 0x0DE8, 0x0DE9, 0x0DEA, 0x0DEB, 0x0DEC, 0x0DED, 0x0DEE, 0x0DEF,
    ],
    [
        0x0ED0, 0x0ED1, 0x0ED2, 0x0ED3, 0x0ED4, 0x0ED5, 0x0ED6, 0x0ED7, 0x0ED8, 0x0ED9,
    ],
    [
        0x0F20, 0x0F21, 0x0F22, 0x0F23, 0x0F24, 0x0F25, 0x0F26, 0x0F27, 0x0F28, 0x0F29,
    ],
    [
        0x1040, 0x1041, 0x1042, 0x1043, 0x1044, 0x1045, 0x1046, 0x1047, 0x1048, 0x1049,
    ],
    [
        0x17E0, 0x17E1, 0x17E2, 0x17E3, 0x17E4, 0x17E5, 0x17E6, 0x17E7, 0x17E8, 0x17E9,
    ],
    [
        0x1810, 0x1811, 0x1812, 0x1813, 0x1814, 0x1815, 0x1816, 0x1817, 0x1818, 0x1819,
    ],
    [
        0x0020, 0x05D0, 0x05D1, 0x05D2, 0x05D3, 0x05D4, 0x05D5, 0x05D6, 0x05D7, 0x05D8,
    ],
    [
        0x0966, 0x0967, 0x0968, 0x0969, 0x096A, 0x096B, 0x096C, 0x096D, 0x096E, 0x096F,
    ],
    [
        0x0F20, 0x0F21, 0x0F22, 0x0F23, 0x0F24, 0x0F25, 0x0F26, 0x0F27, 0x0F28, 0x0F29,
    ],
];

/// `DecimalChar[]` (0 = keep); entries 0 and 1 depend on the locale.
const DECIMAL_CHAR: [u16; 31] = [
    0x002E, 0xFF0E, 0xFF0E, 0x70B9, 0x9EDE, 0x30FB, 0x30FB, 0xFF0E, 0x9EDE, 0xC810, 0x066B, 0x066B,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// `MinusChar[]` (0 = keep).
const MINUS_CHAR: [u16; 31] = [
    0x002D, 0xFF0D, 0xFF0D, 0x8D1F, 0x5069, 0x2212, 0x2212, 0xFF0D, 0x5069, 0xFF0D, 0, 0x2212, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// `SeparatorChar[]` (0 = keep); entries 0 and 1 depend on the locale.
const SEPARATOR_CHAR: [u16; 31] = [
    0x002C, 0xFF0C, 0x3001, 0x3001, 0x3001, 0x3001, 0x3001, 0x3001, 0x3001, 0x3001, 0, 0x066C, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

const MULT_LOWER_ZH: usize = 0;
const MULT_UPPER_ZH: usize = 1;
const MULT_LOWER_ZH_TW: usize = 2;
const MULT_UPPER_ZH_TW: usize = 3;
const MULT_LOWER_KO: usize = 4;
const MULT_HANGUL_KO: usize = 5;
const MULT_MODERN_JA: usize = 6;
const MULT_TRADITIONAL_JA: usize = 7;

const EXP_6: [usize; 6] = [12, 8, 4, 3, 2, 1];
const MULT_6: [[u16; 6]; 8] = [
    [0x5146, 0x4EBF, 0x4E07, 0x5343, 0x767E, 0x5341],
    [0x5146, 0x4EBF, 0x4E07, 0x4EDF, 0x4F70, 0x62FE],
    [0x5146, 0x5104, 0x842C, 0x5343, 0x767E, 0x5341],
    [0x5146, 0x5104, 0x842C, 0x4EDF, 0x4F70, 0x62FE],
    [0x5146, 0x5104, 0x4E07, 0x5343, 0x767E, 0x5341],
    [0xC870, 0xC5B5, 0xB9CC, 0xCC9C, 0xBC31, 0xC2ED],
    [0x5146, 0x5104, 0x4E07, 0x5343, 0x767E, 0x5341],
    [0x5146, 0x5104, 0x842C, 0x9621, 0x767E, 0x62FE],
];
const EXP_2: [usize; 2] = [8, 4];
const MULT_2: [[u16; 2]; 8] = [
    [0x4EBF, 0x4E07],
    [0x4EBF, 0x4E07],
    [0x5104, 0x842C],
    [0x5104, 0x842C],
    [0x5104, 0x4E07],
    [0xC5B5, 0xB9CC],
    [0x5104, 0x4E07],
    [0x5104, 0x842C],
];
const EXP_7: [usize; 7] = [16, 12, 8, 4, 3, 2, 1];
const MULT_7: [[u16; 7]; 8] = [
    [0x4EAC, 0x5146, 0x4EBF, 0x4E07, 0x5343, 0x767E, 0x5341],
    [0x4EAC, 0x5146, 0x4EBF, 0x4E07, 0x4EDF, 0x4F70, 0x62FE],
    [0x4EAC, 0x5146, 0x5104, 0x842C, 0x5343, 0x767E, 0x5341],
    [0x4EAC, 0x5146, 0x5104, 0x842C, 0x4EDF, 0x4F70, 0x62FE],
    [0x4EAC, 0x5146, 0x5104, 0x4E07, 0x5343, 0x767E, 0x5341],
    [0x4EAC, 0xC870, 0xC5B5, 0xB9CC, 0xCC9C, 0xBC31, 0xC2ED],
    [0x4EAC, 0x5146, 0x5104, 0x4E07, 0x5343, 0x767E, 0x5341],
    [0x4EAC, 0x5146, 0x5104, 0x842C, 0x9621, 0x767E, 0x62FE],
];

const OMIT_ZERO: u16 = 1 << 0;
const OMIT_ONLY_ZERO: u16 = 1 << 1;
const OMIT_ONE_6: u16 = 1 << 7;
const OMIT_ONE_7: u16 = 1 << 8;
const OMIT_ONE: u16 = 0b1_1111_1100;
const OMIT_ALL: u16 = OMIT_ZERO | OMIT_ONE | OMIT_ONLY_ZERO;
const OMIT_ZERO_ONE: u16 = OMIT_ZERO | OMIT_ONE;
const OMIT_ZERO_ONE_67: u16 = OMIT_ZERO | OMIT_ONE_6 | OMIT_ONE_7;

/// The `Number` struct of nativenumbersupplier.cxx.
struct Number {
    number: usize,
    mult: &'static [u16],
    flag: u16,
    exp: &'static [usize],
}

const fn n6(number: usize, mult: usize, flag: u16) -> Number {
    Number {
        number,
        mult: &MULT_6[mult],
        flag,
        exp: &EXP_6,
    }
}
const fn n7(number: usize, mult: usize, flag: u16) -> Number {
    Number {
        number,
        mult: &MULT_7[mult],
        flag,
        exp: &EXP_7,
    }
}
const fn n2(number: usize, mult: usize, flag: u16) -> Number {
    Number {
        number,
        mult: &MULT_2[mult],
        flag,
        exp: &EXP_2,
    }
}

static NATNUM4: [Number; 4] = [
    n6(LOWER_ZH, MULT_LOWER_ZH, 0),
    n6(LOWER_ZH, MULT_LOWER_ZH_TW, 0),
    n7(MODERN_JA, MULT_MODERN_JA, OMIT_ZERO_ONE_67),
    n6(LOWER_KO, MULT_LOWER_KO, OMIT_ZERO),
];
static NATNUM5: [Number; 4] = [
    n6(UPPER_ZH, MULT_UPPER_ZH, 0),
    n6(UPPER_ZH_TW, MULT_UPPER_ZH_TW, 0),
    n7(TRADITIONAL_JA, MULT_TRADITIONAL_JA, OMIT_ZERO_ONE_67),
    n6(UPPER_KO, MULT_UPPER_ZH_TW, OMIT_ZERO),
];
static NATNUM6: [Number; 4] = [
    n6(FULL_WIDTH, MULT_LOWER_ZH, 0),
    n6(FULL_WIDTH, MULT_LOWER_ZH_TW, 0),
    n7(FULL_WIDTH, MULT_MODERN_JA, OMIT_ZERO_ONE_67),
    n6(FULL_WIDTH, MULT_HANGUL_KO, OMIT_ZERO),
];
static NATNUM7: [Number; 4] = [
    n6(LOWER_ZH, MULT_LOWER_ZH, OMIT_ALL),
    n6(LOWER_ZH, MULT_LOWER_ZH_TW, OMIT_ALL),
    n2(MODERN_JA, MULT_MODERN_JA, OMIT_ZERO_ONE),
    n6(LOWER_KO, MULT_LOWER_KO, OMIT_ALL),
];
static NATNUM8: [Number; 4] = [
    n6(UPPER_ZH, MULT_UPPER_ZH, OMIT_ALL),
    n6(UPPER_ZH_TW, MULT_UPPER_ZH_TW, OMIT_ALL),
    n2(TRADITIONAL_JA, MULT_TRADITIONAL_JA, OMIT_ZERO_ONE),
    n6(UPPER_KO, MULT_UPPER_ZH_TW, OMIT_ALL),
];
static NATNUM10: Number = n6(HANGUL_KO, MULT_HANGUL_KO, OMIT_ZERO);
static NATNUM11: Number = n6(HANGUL_KO, MULT_HANGUL_KO, OMIT_ALL);

/// `natnum1[]`, indexed by [`lang_number`].
const NATNUM1: [usize; 25] = [
    LOWER_ZH,
    LOWER_ZH,
    MODERN_JA,
    LOWER_KO,
    HE,
    INDIC_AR,
    TH,
    HI,
    OR,
    MR,
    BN,
    PA,
    GU,
    TA,
    TE,
    KN,
    ML,
    LO,
    BO,
    MY,
    KM,
    MN,
    NE,
    DZ,
    EAST_INDIC_AR,
];

/// `natnum2[]`, indexed by [`lang_number`] (CJK and Hebrew only).
const NATNUM2: [usize; 5] = [UPPER_ZH, UPPER_ZH_TW, TRADITIONAL_JA, UPPER_KO, HE];

/// `getLanguageNumber`: the index into `natnum1Locales` (zh_CN, zh_TW, ja,
/// ko, he, ar, th, hi, or, mr, bn, pa, gu, ta, te, kn, ml, lo, bo, my, km,
/// mn, ne, dz, fa), by Windows language id.
fn lang_number(lang: LangId) -> Option<usize> {
    let p = lang.primary();
    Some(match p {
        P_ZH => match lang.0 {
            // zh-TW, zh-HK, zh-MO
            0x0404 | 0x0C04 | 0x1404 => 1,
            _ => 0,
        },
        P_JA => 2,
        P_KO => 3,
        0x0D => 4,                      // he
        0x01 => 5,                      // ar
        0x1E => 6,                      // th
        0x39 => 7,                      // hi
        0x48 => 8,                      // or
        0x4E => 9,                      // mr
        0x45 => 10,                     // bn
        0x46 => 11,                     // pa
        0x47 => 12,                     // gu
        0x49 => 13,                     // ta
        0x4A => 14,                     // te
        0x4B => 15,                     // kn
        0x4C => 16,                     // ml
        0x54 => 17,                     // lo
        0x51 if lang.0 == 0x0C51 => 23, // dz-BT
        0x51 => 18,                     // bo
        0x55 => 19,                     // my
        0x53 => 20,                     // km
        0x50 => 21,                     // mn
        0x61 => 22,                     // ne
        0x29 => 24,                     // fa
        _ => return None,
    })
}

/// `NativeNumberSupplier::isValidNatNum`.
pub fn is_valid_natnum(lang: LangId, mode: u8) -> bool {
    let n = lang_number(lang);
    match mode {
        0 | 3 => true,
        1 => n.is_some(),
        2 if n == Some(4) => true,
        2 | 4..=8 => n.is_some_and(|n| n < 4),
        9..=11 => n == Some(3),
        _ => false,
    }
}

/// Decimal and grouping separators of the text being transliterated (AOO
/// takes them from the transliteration locale's data; Tally passes the
/// separators the text was formatted with, see `docs/numfmt-natnum.md`).
#[derive(Copy, Clone, Debug)]
pub struct Separators {
    /// Decimal separator.
    pub decimal: char,
    /// Thousands separator.
    pub group: char,
}

impl Separators {
    /// `.` and `,`.
    pub const EN: Separators = Separators {
        decimal: '.',
        group: ',',
    };
}

/// The locale-dependent `DecimalChar` / `SeparatorChar` entries (0 and 1).
struct Chars {
    decimal: [u16; 2],
    separator: [u16; 2],
}

impl Chars {
    fn new(sep: Separators) -> Chars {
        let full = |c: char, dflt: u16| -> u16 {
            let u = c as u32;
            if (0x21..=0x7E).contains(&u) {
                (u + 0xFEE0) as u16
            } else {
                dflt
            }
        };
        let d = sep.decimal as u32;
        let g = sep.group as u32;
        Chars {
            decimal: [d.min(0xFFFF) as u16, full(sep.decimal, 0xFF0E)],
            separator: [g.min(0xFFFF) as u16, full(sep.group, 0xFF0C)],
        }
    }
    fn is_decimal(&self, c: u16) -> bool {
        c == self.decimal[0]
    }
    fn is_separator(&self, c: u16) -> bool {
        c == self.separator[0]
    }
    fn decimal_of(&self, number: usize) -> u16 {
        if number < 2 {
            self.decimal[number]
        } else {
            DECIMAL_CHAR[number]
        }
    }
    fn separator_of(&self, number: usize) -> u16 {
        if number < 2 {
            self.separator[number]
        } else {
            SEPARATOR_CHAR[number]
        }
    }
}

fn is_number(c: u16) -> bool {
    (0x30..=0x39).contains(&c)
}

fn is_minus(c: u16) -> bool {
    c == 0x2D
}

fn or_keep(x: u16, c: u16) -> u16 {
    if x != 0 {
        x
    } else {
        c
    }
}

/// Transliterates `s` (`NativeNumberSupplier::getNativeNumberString`) in
/// language `lang` with NatNum `mode`. Returns `s` unchanged (borrowed) when
/// the mode is not valid for the language or there is nothing to change.
pub fn transliterate<'a>(s: &'a str, lang: LangId, mode: u8, sep: Separators) -> Cow<'a, str> {
    if !is_valid_natnum(lang, mode) {
        return Cow::Borrowed(s);
    }
    // NatNum0 and the digit-only modes leave text without ASCII digits alone.
    if mode != 0 && !s.bytes().any(|b| b.is_ascii_digit()) {
        return Cow::Borrowed(s);
    }
    let langnum = lang_number(lang);
    let chars = Chars::new(sep);
    let src: Vec<u16> = s.encode_utf16().collect();
    let out = match mode {
        0 => native_to_ascii(&src, &chars),
        1 | 2 | 3 | 9 => {
            let num = match mode {
                1 => NATNUM1[langnum.unwrap_or(0)],
                2 => NATNUM2[langnum.unwrap_or(0).min(4)],
                3 => FULL_WIDTH,
                _ => HANGUL_KO,
            };
            if num == HE {
                return Cow::Owned(hebrew(s, mode == 2, &chars));
            }
            ascii_to_native_char(&src, num, &chars)
        }
        _ => {
            let i = langnum.unwrap_or(0).min(3);
            let number = match mode {
                4 => &NATNUM4[i],
                5 => &NATNUM5[i],
                6 => &NATNUM6[i],
                7 => &NATNUM7[i],
                8 => &NATNUM8[i],
                10 => &NATNUM10,
                _ => &NATNUM11,
            };
            ascii_to_native(&src, number, &chars)
        }
    };
    Cow::Owned(String::from_utf16_lossy(&out))
}

/// `AsciiToNativeChar`: digit by digit.
fn ascii_to_native_char(src: &[u16], number: usize, chars: &Chars) -> Vec<u16> {
    let n = src.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let ch = src[i];
        let c = if is_number(ch) {
            NUMBER_CHAR[number][usize::from(ch - 0x30)]
        } else if i + 1 < n && is_number(src[i + 1]) {
            if i > 0 && is_number(src[i - 1]) && chars.is_separator(ch) {
                or_keep(chars.separator_of(number), ch)
            } else if chars.is_decimal(ch) {
                or_keep(chars.decimal_of(number), ch)
            } else if is_minus(ch) {
                or_keep(MINUS_CHAR[number], ch)
            } else {
                ch
            }
        } else {
            ch
        };
        out.push(c);
    }
    out
}

/// `AsciiToNative_numberMaker`.
#[allow(clippy::too_many_arguments)]
fn number_maker(
    s: &[u16],
    mut begin: usize,
    mut len: usize,
    dst: &mut Vec<u16>,
    multi_index: Option<usize>,
    number: &Number,
    nc: &[u16; 10],
) -> bool {
    let multi_char = multi_index.map(|i| number.mult[i]);
    let last_exp = number.exp[number.exp.len() - 1];
    let digit = |c: u16| nc[usize::from(c - 0x30)];
    if len <= last_exp {
        if last_exp > 1 {
            let mut not_zero = false;
            for _ in 0..len {
                if not_zero || s[begin] != 0x30 {
                    dst.push(digit(s[begin]));
                    not_zero = true;
                }
                begin += 1;
            }
            if let (true, Some(m)) = (not_zero, multi_char) {
                dst.push(m);
            }
            return not_zero;
        } else if s[begin] != 0x30 {
            let omit_one =
                multi_index.is_some_and(|i| number.flag & (1 << (2 + i)) != 0) && s[begin] == 0x31;
            if !omit_one {
                dst.push(digit(s[begin]));
            }
            if let Some(m) = multi_char {
                dst.push(m);
            }
        } else if number.flag & OMIT_ZERO == 0 && dst.last().is_some_and(|&l| l != nc[0]) {
            dst.push(nc[0]);
        }
        s[begin] != 0x30
    } else {
        let count = number.exp.len();
        let mut print_power = false;
        for i in 1..=count {
            let sub = if i == count { 0 } else { number.exp[i] };
            if len > sub {
                let tmp = len - sub;
                print_power |=
                    number_maker(s, begin, tmp, dst, (i != count).then_some(i), number, nc);
                begin += tmp;
                len -= tmp;
            }
        }
        if print_power {
            if last_exp == 1 && dst.last() == Some(&nc[0]) {
                dst.pop();
            }
            if let Some(m) = multi_char {
                dst.push(m);
            }
        }
        print_power
    }
}

/// `AsciiToNative`: numbers spelled with multipliers.
fn ascii_to_native(src: &[u16], number: &Number, chars: &Chars) -> Vec<u16> {
    let nc = &NUMBER_CHAR[number.number];
    let n = src.len();
    let mut out: Vec<u16> = Vec::with_capacity(n * 2);
    let mut digits: Vec<u16> = Vec::with_capacity(n);
    let mut do_decimal = false;
    let last_exp = number.exp[number.exp.len() - 1];
    let e0 = number.exp[0];
    let next_is_number = |i: usize| i + 1 < n && is_number(src[i + 1]);
    for i in 0..=n {
        if i < n && is_number(src[i]) {
            if do_decimal {
                out.push(nc[usize::from(src[i] - 0x30)]);
            } else {
                digits.push(src[i]);
            }
            continue;
        }
        let len = digits.len();
        if len > 0 {
            if i < n && chars.is_separator(src[i]) && next_is_number(i) {
                continue; // skip a grouping separator inside the number
            }
            let mut not_zero = false;
            let mut begin = 0;
            let mut end = len % e0;
            while end <= len {
                if end != 0 {
                    let before = out.len();
                    not_zero |= number_maker(
                        &digits,
                        begin,
                        end - begin,
                        &mut out,
                        (end != len).then_some(0),
                        number,
                        nc,
                    );
                    if last_exp == 1 && out.last() == Some(&nc[0]) {
                        out.pop();
                    }
                    if not_zero && before == out.len() && end != len {
                        out.push(number.mult[0]);
                    }
                }
                begin = end;
                end += e0;
            }
            if !not_zero && number.flag & OMIT_ONLY_ZERO == 0 {
                out.push(nc[0]);
            }
            digits.clear();
        }
        if i < n {
            let c = src[i];
            do_decimal = !do_decimal && chars.is_decimal(c) && next_is_number(i);
            let o = if do_decimal {
                or_keep(chars.decimal_of(number.number), c)
            } else if is_minus(c) && next_is_number(i) {
                or_keep(MINUS_CHAR[number.number], c)
            } else if chars.is_separator(c) && next_is_number(i) {
                or_keep(chars.separator_of(number.number), c)
            } else {
                c
            };
            out.push(o);
        }
    }
    out
}

/// `NativeToAscii` for NatNum0, without the multiplier parsing: native
/// digits become ASCII digits and native decimal / minus / grouping
/// characters before a digit become ASCII.
fn native_to_ascii(src: &[u16], chars: &Chars) -> Vec<u16> {
    let digit_of = |c: u16| -> Option<u16> {
        NUMBER_CHAR
            .iter()
            .enumerate()
            .find_map(|(k, row)| {
                // The Hebrew row maps 0 to a space: not a digit.
                if k == HE {
                    return None;
                }
                row.iter().position(|&x| x == c)
            })
            .map(|d| d as u16 + 0x30)
    };
    let n = src.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let c = src[i];
        if let Some(d) = digit_of(c) {
            out.push(d);
            continue;
        }
        let before_digit = i + 1 < n && digit_of(src[i + 1]).is_some();
        let o = if before_digit && (2..31).any(|k| SEPARATOR_CHAR[k] == c && c != 0) {
            chars.separator[0]
        } else if before_digit && (2..31).any(|k| DECIMAL_CHAR[k] == c && c != 0) {
            chars.decimal[0]
        } else if before_digit && (1..31).any(|k| MINUS_CHAR[k] == c && c != 0) {
            0x2D
        } else {
            c
        };
        out.push(o);
    }
    out
}

// --- Hebrew numbering -------------------------------------------------------

const HEBREW: [(char, u32); 23] = [
    ('\u{05ea}', 400),
    ('\u{05ea}', 400),
    ('\u{05e9}', 300),
    ('\u{05e8}', 200),
    ('\u{05e7}', 100),
    ('\u{05e6}', 90),
    ('\u{05e4}', 80),
    ('\u{05e2}', 70),
    ('\u{05e1}', 60),
    ('\u{05e0}', 50),
    ('\u{05de}', 40),
    ('\u{05dc}', 30),
    ('\u{05db}', 20),
    ('\u{05d9}', 10),
    ('\u{05d8}', 9),
    ('\u{05d7}', 8),
    ('\u{05d6}', 7),
    ('\u{05d5}', 6),
    ('\u{05d4}', 5),
    ('\u{05d3}', 4),
    ('\u{05d2}', 3),
    ('\u{05d1}', 2),
    ('\u{05d0}', 1),
];

fn make_hebrew(value: u64, out: &mut Vec<char>, is_last: bool, geresh: bool) {
    let mut num = (value % 1000) as u32;
    if value > 1000 {
        make_hebrew(value / 1000, out, num != 0, geresh);
        out.push(' ');
    }
    if num == 0 {
        let word = if value == 1000 {
            "\u{05d0}\u{05dc}\u{05e3}"
        } else if is_last {
            "\u{05d0}\u{05dc}\u{05e4}\u{05d9}\u{05dd}"
        } else {
            "\u{05d0}\u{05dc}\u{05e4}\u{05d9}"
        };
        out.extend(word.chars());
    } else {
        let mut n_chars = 0;
        let mut j = 0;
        while num > 0 && j < HEBREW.len() {
            if num >= HEBREW[j].1 {
                n_chars += 1;
                if num == 15 || num == 16 {
                    j += 1;
                }
                num -= HEBREW[j].1;
                out.push(HEBREW[j].0);
            }
            j += 1;
        }
        if geresh {
            if n_chars > 1 {
                let at = out.len() - 1;
                out.insert(at, '\u{05f4}');
            } else if n_chars == 1 {
                out.push('\u{05f3}');
            }
        }
    }
}

/// `getHebrewNativeNumberString`: the leading number (a `-` and grouping
/// separators are skipped) in Hebrew letters, then the rest of the text.
fn hebrew(s: &str, geresh: bool, chars: &Chars) -> String {
    let mut value: u64 = 0;
    let mut count = 0;
    let mut rest = s.len();
    for (i, ch) in s.char_indices() {
        let c = ch as u32;
        if ch.is_ascii_digit() {
            count += 1;
            if count >= 20 {
                return s.to_string();
            }
            value = value * 10 + u64::from(c - 0x30);
        } else if (c <= 0xFFFF && chars.is_separator(c as u16) && count > 0)
            || (ch == '-' && count == 0)
        {
            continue;
        } else {
            rest = i;
            break;
        }
    }
    if value == 0 {
        return s.to_string();
    }
    let mut out = Vec::new();
    make_hebrew(value, &mut out, true, geresh);
    let mut text: String = out.into_iter().collect();
    text.push_str(&s[rest..]);
    text
}

// --- ODF number:transliteration-* -------------------------------------------

/// ODF `number:transliteration-format` / `-style` for a NatNum mode in a
/// language (`NativeNumberSupplier::convertToXmlAttributes`): the digit one
/// of the number set and `short` / `medium` / `long`.
pub fn to_xml_attributes(lang: LangId, mode: u8) -> (char, &'static str) {
    let mut number = HALF_WIDTH;
    let mut style = "short";
    if is_valid_natnum(lang, mode) {
        let n = lang_number(lang).unwrap_or(0);
        (number, style) = match mode {
            1 => (NATNUM1[n], "short"),
            2 => {
                let k = NATNUM2[n.min(4)];
                (k, if k == HE { "medium" } else { "short" })
            }
            3 => (FULL_WIDTH, "short"),
            4 => (NATNUM1[n], "long"),
            5 => (NATNUM2[n.min(4)], "long"),
            6 => (FULL_WIDTH, "long"),
            7 => (NATNUM1[n], "medium"),
            8 => (NATNUM2[n.min(4)], "medium"),
            9 => (HANGUL_KO, "short"),
            10 => (HANGUL_KO, "long"),
            11 => (HANGUL_KO, "medium"),
            _ => (HALF_WIDTH, "short"),
        };
    }
    let one = char::from_u32(u32::from(NUMBER_CHAR[number][1])).unwrap_or('1');
    (one, style)
}

/// The NatNum mode of ODF transliteration attributes
/// (`NativeNumberSupplier::convertFromXmlAttributes`); `None` for an
/// unknown style.
pub fn from_xml_attributes(format: &str, style: &str) -> Option<u8> {
    let first = format.chars().next()?;
    let num = NUMBER_CHAR
        .iter()
        .position(|row| u32::from(row[1]) == first as u32);
    let in1 = |k: usize| NATNUM1.contains(&k);
    let in2 = |k: usize| NATNUM2.contains(&k);
    Some(match style {
        "short" => match num {
            Some(FULL_WIDTH) => 3,
            Some(HANGUL_KO) => 9,
            Some(k) if in1(k) => 1,
            Some(k) if in2(k) => 2,
            _ => 0,
        },
        "medium" => match num {
            Some(HANGUL_KO) => 11,
            Some(HE) => 2,
            Some(k) if in1(k) => 7,
            Some(k) if in2(k) => 8,
            _ => 0,
        },
        "long" => match num {
            Some(FULL_WIDTH) => 6,
            Some(HANGUL_KO) => 10,
            Some(k) if in1(k) => 4,
            Some(k) if in2(k) => 5,
            _ => 0,
        },
        _ => return None,
    })
}

// --- Excel numeral shapes -----------------------------------------------------

/// Excel's numeral-shape byte of an extended locale id (`[$-D07041E]`: `0D`)
/// as the language and NatNum it means (LibreOffice
/// `SvNumberformat::ImpObtainCalendarAndNumerals`): the locale's own
/// language when it uses those numerals, else the reference language.
pub fn excel_numeral(shape: u8, lang: LangId) -> Option<(LangId, u8)> {
    let shape = shape & 0x7F;
    let reference = match shape {
        0x02 => LangId::AR_SA,
        0x03 => LangId(0x0429),
        0x04 => LangId(0x0439),
        0x05 => LangId(0x0445),
        0x06 => LangId(0x0446),
        0x07 => LangId(0x0447),
        0x08 => LangId(0x0448),
        0x09 => LangId(0x0449),
        0x0A => LangId(0x044A),
        0x0B => LangId(0x044B),
        0x0C => LangId(0x044C),
        0x0D => LangId::TH,
        0x0E => LangId(0x0454),
        0x0F => LangId(0x0451),
        0x10 => LangId(0x0455),
        0x12 => LangId(0x0453),
        0x13 => LangId(0x0450),
        0x1B..=0x1D => LangId::JA,
        0x1E..=0x20 => LangId::ZH_CN,
        0x21..=0x23 => LangId::ZH_TW,
        0x24..=0x27 => LangId::KO,
        _ => return None,
    };
    let natnum = match shape {
        0x27 => 9,
        0x1B..=0x26 => (shape - 0x1B) % 3 + 1,
        _ => 1,
    };
    // Devanagari is shared by Hindi, Marathi and Nepali; Chinese needs the
    // same script (simplified / traditional).
    let same = match shape {
        0x04 => matches!(lang.primary(), 0x39 | 0x4E | 0x61),
        0x1E..=0x20 => lang.primary() == P_ZH && lang_number(lang) == Some(0),
        0x21..=0x23 => lang.primary() == P_ZH && lang_number(lang) == Some(1),
        _ => lang.primary() == reference.primary(),
    };
    Some((if same { lang } else { reference }, natnum))
}

/// The Excel numeral-shape byte for a NatNum in a language (the inverse of
/// [`excel_numeral`]); `None` when Excel has no equivalent.
pub fn excel_numeral_shape(lang: LangId, mode: u8) -> Option<u8> {
    let n = lang_number(lang)?;
    match (n, mode) {
        (0, 1..=3) => Some(0x1E + mode - 1),
        (1, 1..=3) => Some(0x21 + mode - 1),
        (2, 1..=3) => Some(0x1B + mode - 1),
        (3, 1..=3) => Some(0x24 + mode - 1),
        (3, 9) => Some(0x27),
        (_, 1) => Some(match n {
            5 => 0x02,
            24 => 0x03,
            7 | 9 | 22 => 0x04,
            10 => 0x05,
            11 => 0x06,
            12 => 0x07,
            8 => 0x08,
            13 => 0x09,
            14 => 0x0A,
            15 => 0x0B,
            16 => 0x0C,
            6 => 0x0D,
            17 => 0x0E,
            18 | 23 => 0x0F,
            19 => 0x10,
            20 => 0x12,
            21 => 0x13,
            _ => return None,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str, lang: LangId, mode: u8) -> String {
        transliterate(s, lang, mode, Separators::EN).into_owned()
    }

    #[test]
    fn char_modes() {
        // NatNum1: lower-case native digits; separators before a digit map.
        assert_eq!(t("1234", LangId::JA, 1), "一二三四");
        assert_eq!(t("-1,234.5", LangId::ZH_CN, 1), "－一、二三四．五");
        assert_eq!(t("2026", LangId::KO, 1), "二〇二六");
        assert_eq!(t("123.45", LangId::AR_SA, 1), "١٢٣٫٤٥");
        assert_eq!(t("-12", LangId::AR_SA, 1), "-١٢");
        assert_eq!(t("1,234", LangId::TH, 1), "๑,๒๓๔");
        assert_eq!(t("90", LangId(0x0439), 1), "९०");
        assert_eq!(t("45", LangId(0x0429), 1), "۴۵");
        // NatNum2: financial digits.
        assert_eq!(t("1234", LangId::ZH_CN, 2), "壹贰叁肆");
        assert_eq!(t("1234", LangId::ZH_TW, 2), "壹貳參肆");
        assert_eq!(t("1234", LangId::JA, 2), "壱弐参四");
        assert_eq!(t("1230", LangId::KO, 2), "壹貳參\u{F9B2}");
        // NatNum3: full width in every language, locale separators.
        assert_eq!(t("-1,234.5", LangId::EN_US, 3), "－１，２３４．５");
        let de = Separators {
            decimal: ',',
            group: '.',
        };
        assert_eq!(
            transliterate("1.234,5", LangId(0x0407), 3, de),
            "１．２３４，５"
        );
        // NatNum9: Hangul digits.
        assert_eq!(t("2026", LangId::KO, 9), "이영이육");
        // Invalid combinations keep the text.
        assert_eq!(t("123", LangId::EN_US, 1), "123");
        assert_eq!(t("123", LangId::TH, 2), "123");
        assert_eq!(t("123", LangId::JA, 9), "123");
        assert_eq!(t("123", LangId::JA, 12), "123");
        assert!(matches!(
            transliterate("abc", LangId::JA, 1, Separators::EN),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn spelled_modes() {
        // NatNum4 (long, lower).
        assert_eq!(t("1234", LangId::ZH_CN, 4), "一千二百三十四");
        assert_eq!(t("1001", LangId::ZH_CN, 4), "一千〇一");
        assert_eq!(t("10", LangId::ZH_CN, 4), "一十");
        assert_eq!(t("120000", LangId::ZH_CN, 4), "一十二万");
        assert_eq!(t("100000000", LangId::ZH_TW, 4), "一億");
        assert_eq!(t("0", LangId::ZH_CN, 4), "〇");
        assert_eq!(t("12.5", LangId::ZH_CN, 4), "一十二．五");
        // Japanese omits 1 before 10 and 100 and never writes 0.
        assert_eq!(t("1234", LangId::JA, 4), "一千二百三十四");
        assert_eq!(t("1001", LangId::JA, 4), "一千一");
        assert_eq!(t("110", LangId::JA, 4), "百十");
        assert_eq!(t("10", LangId::JA, 4), "十");
        assert_eq!(t("11000", LangId::JA, 4), "一万一千");
        assert_eq!(t("2026", LangId::JA, 4), "二千二十六");
        assert_eq!(t("-5", LangId::JA, 4), "−五");
        assert_eq!(t("1,234", LangId::JA, 4), "一千二百三十四");
        // Korean omits 0.
        assert_eq!(t("1001", LangId::KO, 4), "一千一");
        // NatNum5 / 6.
        assert_eq!(t("1234", LangId::ZH_CN, 5), "壹仟贰佰叁拾肆");
        assert_eq!(t("1234", LangId::ZH_TW, 5), "壹仟貳佰參拾肆");
        assert_eq!(t("1234", LangId::JA, 5), "壱阡弐百参拾四");
        assert_eq!(t("1234", LangId::ZH_CN, 6), "１千２百３十４");
        // NatNum7 / 8: short forms.
        assert_eq!(t("1234", LangId::ZH_CN, 7), "千二百三十四");
        assert_eq!(t("12345", LangId::JA, 7), "一万二三四五");
        assert_eq!(t("1234", LangId::ZH_CN, 8), "仟贰佰叁拾肆");
        // NatNum10 / 11: Hangul.
        assert_eq!(t("1234", LangId::KO, 10), "일천이백삼십사");
        assert_eq!(t("1234", LangId::KO, 11), "천이백삼십사");
        assert_eq!(t("1111", LangId::KO, 11), "천백십일");
    }

    #[test]
    fn hebrew_numbers() {
        assert_eq!(t("15", LangId::HE, 1), "טו");
        assert_eq!(t("16", LangId::HE, 1), "טז");
        assert_eq!(t("5786", LangId::HE, 1), "ה תשפו");
        assert_eq!(t("5786", LangId::HE, 2), "ה׳ תשפ״ו");
        assert_eq!(t("1", LangId::HE, 2), "א׳");
        assert_eq!(t("1000", LangId::HE, 1), "אלף");
        assert_eq!(t("2000", LangId::HE, 1), "ב אלפים");
        assert_eq!(t("12 kg", LangId::HE, 1), "יב kg");
        assert_eq!(t("x12", LangId::HE, 1), "x12");
    }

    #[test]
    fn natnum0_and_dbnum() {
        assert_eq!(t("一二三 ١٢", LangId::JA, 0), "123 12");
        assert_eq!(map_dbnum_to_natnum(1, LangId::ZH_CN, false), 4);
        assert_eq!(map_dbnum_to_natnum(1, LangId::JA, false), 1);
        assert_eq!(map_dbnum_to_natnum(2, LangId::JA, false), 4);
        assert_eq!(map_dbnum_to_natnum(3, LangId::JA, false), 5);
        assert_eq!(map_dbnum_to_natnum(4, LangId::JA, false), 7);
        assert_eq!(map_dbnum_to_natnum(2, LangId::KO, false), 2);
        assert_eq!(map_dbnum_to_natnum(4, LangId::KO, false), 9);
        assert_eq!(map_dbnum_to_natnum(1, LangId::EN_US, false), 0);
        assert_eq!(map_dbnum_to_natnum(2, LangId::ZH_CN, true), 2);
        assert_eq!(map_dbnum_to_natnum(4, LangId::KO, true), 9);
        assert_eq!(map_natnum_to_dbnum(4, LangId::ZH_CN, false), 1);
        assert_eq!(map_natnum_to_dbnum(4, LangId::JA, false), 2);
        assert_eq!(map_natnum_to_dbnum(1, LangId::ZH_CN, false), 0);
        for lang in [LangId::JA, LangId::ZH_CN, LangId::ZH_TW, LangId::KO] {
            for db in 1..=4 {
                for date in [false, true] {
                    let nat = map_dbnum_to_natnum(db, lang, date);
                    if nat != 0 {
                        assert_eq!(map_natnum_to_dbnum(nat, lang, date), db, "{lang:?} {db}");
                    }
                }
            }
        }
    }

    #[test]
    fn xml_attributes() {
        assert_eq!(to_xml_attributes(LangId::JA, 1), ('一', "short"));
        assert_eq!(to_xml_attributes(LangId::JA, 4), ('一', "long"));
        assert_eq!(to_xml_attributes(LangId::JA, 7), ('一', "medium"));
        assert_eq!(to_xml_attributes(LangId::ZH_CN, 2), ('壹', "short"));
        assert_eq!(to_xml_attributes(LangId::HE, 2), ('א', "medium"));
        assert_eq!(to_xml_attributes(LangId::EN_US, 3), ('１', "short"));
        assert_eq!(to_xml_attributes(LangId::AR_SA, 1), ('١', "short"));
        assert_eq!(to_xml_attributes(LangId::KO, 11), ('일', "medium"));
        for lang in [
            LangId::JA,
            LangId::ZH_CN,
            LangId::ZH_TW,
            LangId::KO,
            LangId::TH,
        ] {
            for mode in 1..=11 {
                if !is_valid_natnum(lang, mode) {
                    continue;
                }
                let (f, s) = to_xml_attributes(lang, mode);
                let back = from_xml_attributes(&f.to_string(), s).unwrap();
                // NatNum1 and 4/7 of ja and ko share the lower-case digits.
                let (bf, bs) = to_xml_attributes(lang, back);
                assert_eq!((bf, bs), (f, s), "{lang:?} {mode}");
            }
        }
        assert_eq!(from_xml_attributes("一", "long"), Some(4));
        assert_eq!(from_xml_attributes("๑", "short"), Some(1));
        assert_eq!(from_xml_attributes("1", "short"), Some(0));
        assert_eq!(from_xml_attributes("1", "odd"), None);
    }

    #[test]
    fn excel_numerals() {
        assert_eq!(excel_numeral(0x0D, LangId::TH), Some((LangId::TH, 1)));
        assert_eq!(excel_numeral(0x0D, LangId::EN_US), Some((LangId::TH, 1)));
        assert_eq!(
            excel_numeral(0x02, LangId(0x0C01)),
            Some((LangId(0x0C01), 1))
        );
        assert_eq!(excel_numeral(0x1B, LangId::JA), Some((LangId::JA, 1)));
        assert_eq!(excel_numeral(0x1C, LangId::EN_US), Some((LangId::JA, 2)));
        assert_eq!(excel_numeral(0x23, LangId::ZH_CN), Some((LangId::ZH_TW, 3)));
        assert_eq!(excel_numeral(0x27, LangId::KO), Some((LangId::KO, 9)));
        assert_eq!(excel_numeral(0x01, LangId::KO), None);
        assert_eq!(excel_numeral_shape(LangId::TH, 1), Some(0x0D));
        assert_eq!(excel_numeral_shape(LangId::JA, 2), Some(0x1C));
        assert_eq!(excel_numeral_shape(LangId::JA, 4), None);
        assert_eq!(excel_numeral_shape(LangId::EN_US, 1), None);
    }
}
