//! Calendars for date formats: `[~gengou]`, `[~ROC]`, `[~buddhist]`,
//! `[~hanja]`, `[~hijri]`, `[~jewish]` and the implicit "other" calendar of
//! the era codes `E`, `EE`, `R`, `RR`, `AAA`, `AAAA`.
//!
//! Ported from AOO 4.1 `i18npool/source/calendar/` (`calendar_gregorian.cxx`
//! eras table, `mapFromGregorian`, `getDisplayString`, `NatNumForCalendar`;
//! `calendar_hijri.cxx`; `calendar_jewish.cxx`) and the calendars of
//! `i18npool/source/localedata/data/*.xml`. See `docs/numfmt-natnum.md`.

use crate::natnum::{transliterate, LangId, Separators};

/// A calendar.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum CalendarId {
    /// The Gregorian calendar (proleptic, as everywhere in Tally).
    Gregorian,
    /// Japanese era calendar (Meiji .. Reiwa).
    Gengou,
    /// Republic of China (Minguo) calendar.
    Roc,
    /// Thai Buddhist era.
    Buddhist,
    /// Korean Hanja: Gregorian years with Hanja era and weekday names.
    Hanja,
    /// Hijri (AOO's astronomical new-moon computation).
    Hijri,
    /// Hebrew calendar.
    Jewish,
}

impl CalendarId {
    /// The calendar of a `[~name]` modifier / ODF `number:calendar`
    /// (case-insensitive).
    pub fn from_name(name: &str) -> Option<CalendarId> {
        const ALL: [(&str, CalendarId); 7] = [
            ("gregorian", CalendarId::Gregorian),
            ("gengou", CalendarId::Gengou),
            ("ROC", CalendarId::Roc),
            ("buddhist", CalendarId::Buddhist),
            ("hanja", CalendarId::Hanja),
            ("hijri", CalendarId::Hijri),
            ("jewish", CalendarId::Jewish),
        ];
        ALL.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
            .map(|(_, c)| *c)
    }

    /// The AOO calendar id (`gengou`, `ROC`, …).
    pub fn name(self) -> &'static str {
        match self {
            CalendarId::Gregorian => "gregorian",
            CalendarId::Gengou => "gengou",
            CalendarId::Roc => "ROC",
            CalendarId::Buddhist => "buddhist",
            CalendarId::Hanja => "hanja",
            CalendarId::Hijri => "hijri",
            CalendarId::Jewish => "jewish",
        }
    }

    /// The calendar of Excel's calendar byte in an extended locale id
    /// (`[$-30411]`: 3), as LibreOffice reads it, and the language that has
    /// it when the locale's own does not.
    pub fn from_excel(byte: u8) -> Option<(CalendarId, LangId)> {
        Some(match byte & 0x7F {
            0x03 => (CalendarId::Gengou, LangId::JA),
            0x04 => (CalendarId::Roc, LangId::ZH_TW),
            0x06 | 0x17 => (CalendarId::Hijri, LangId::AR_SA),
            0x07 => (CalendarId::Buddhist, LangId::TH),
            0x08 => (CalendarId::Jewish, LangId::HE),
            _ => return None,
        })
    }

    /// Excel's calendar byte for this calendar (0: none).
    pub fn excel_byte(self) -> u8 {
        match self {
            CalendarId::Gengou => 0x03,
            CalendarId::Roc => 0x04,
            CalendarId::Hijri => 0x06,
            CalendarId::Buddhist => 0x07,
            CalendarId::Jewish => 0x08,
            CalendarId::Gregorian | CalendarId::Hanja => 0,
        }
    }

    /// The era table (`eraArray`) of the calendars that only renumber
    /// Gregorian years.
    fn eras(self) -> Option<&'static [(i64, u32, u32)]> {
        match self {
            CalendarId::Gengou => Some(&GENGOU_ERAS),
            CalendarId::Roc => Some(&[(1912, 1, 1)]),
            CalendarId::Buddhist => Some(&[(-542, 1, 1)]),
            _ => None,
        }
    }
}

/// `gengou_eraArray`: Meiji, Taisho, Showa, Heisei, Reiwa.
const GENGOU_ERAS: [(i64, u32, u32); 5] = [
    (1868, 1, 1),
    (1912, 7, 30),
    (1926, 12, 25),
    (1989, 1, 8),
    (2019, 5, 1),
];

/// The non-Gregorian calendar of a language in AOO's locale data (the one
/// `E`, `EE`, `R`, `RR`, `AAA`, `AAAA` switch to).
pub fn other_calendar(lang: LangId) -> Option<CalendarId> {
    match lang.primary() {
        0x11 => Some(CalendarId::Gengou),
        0x04 if lang.0 == 0x0404 => Some(CalendarId::Roc),
        0x12 => Some(CalendarId::Hanja),
        0x1E => Some(CalendarId::Buddhist),
        // ar_OM has only the Gregorian calendar.
        0x01 if lang.0 != 0x2001 => Some(CalendarId::Hijri),
        0x0D => Some(CalendarId::Jewish),
        _ => None,
    }
}

/// Whether a language's locale data has the calendar (`[~x]` for any
/// other calendar is ignored, as AOO's `loadCalendar` fails).
pub fn has_calendar(lang: LangId, cal: CalendarId) -> bool {
    cal == CalendarId::Gregorian || other_calendar(lang) == Some(cal)
}

/// The calendar fields of a date (`CalendarFieldIndex` values).
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Fields {
    /// Era index into the calendar's era names.
    pub era: usize,
    /// Year in the era (always positive).
    pub year: i64,
    /// Month, 1-based (1..=13 in the Jewish calendar).
    pub month: u32,
    /// Day of month.
    pub day: u32,
    /// Weekday, 0 = Sunday.
    pub weekday: u32,
}

/// The fields of the Gregorian date `y-m-d` (`unix_days` after 1970-01-01,
/// weekday `wd`) in calendar `cal`.
pub fn fields(cal: CalendarId, y: i64, m: u32, d: u32, unix_days: i64, wd: u32) -> Fields {
    let greg = Fields {
        era: usize::from(y > 0),
        year: if y > 0 { y } else { 1 - y },
        month: m,
        day: d,
        weekday: wd,
    };
    if let Some(eras) = cal.eras() {
        // mapFromGregorian: the first era starting after the date.
        let e = eras
            .iter()
            .position(|&(ey, em, ed)| (y, m, d) < (ey, em, ed))
            .unwrap_or(eras.len());
        let year = if e == 0 {
            eras[0].0 - y
        } else {
            y - eras[e - 1].0 + 1
        };
        return Fields {
            era: e,
            year,
            ..greg
        };
    }
    match cal {
        CalendarId::Hijri => {
            let (hd, hm, hy) = hijri_from_julian_day(unix_days + 2_440_588);
            Fields {
                era: usize::from(hy >= 1),
                year: hy.abs(),
                month: hm as u32,
                day: hd as u32,
                weekday: wd,
            }
        }
        CalendarId::Jewish => {
            let (hy, hm, hd) = hebrew_from_absolute(unix_days + 719_163);
            Fields {
                era: usize::from(hy > 0),
                year: if hy <= 0 { 1 - hy } else { hy },
                month: hm as u32,
                day: hd as u32,
                weekday: wd,
            }
        }
        _ => greg,
    }
}

/// Whether `ImpFallBackToGregorianCalendar` applies: a gengou date before
/// Meiji (the "Dummy" era) is shown in the Gregorian calendar.
pub fn falls_back(cal: CalendarId, f: &Fields) -> bool {
    cal == CalendarId::Gengou && f.era == 0
}

// --- Hijri (calendar_hijri.cxx) -------------------------------------------

const RAD_PER_DEG: f64 = 0.017_453_292_519_943_295;
const SYN_PERIOD: f64 = 29.530_588_68;
const JD1900: f64 = 2_415_020.759_33;
const SYN_REF: i64 = 1252;
const GREG_REF: i64 = 1422;

/// `Calendar_hijri::NewMoon`: Julian date of the n-th new moon since 1900.
fn new_moon(n: i64) -> f64 {
    let k = n as f64;
    let t = k / 1236.85;
    let t2 = t * t;
    let t3 = t2 * t;
    let mut jd = JD1900 + SYN_PERIOD * k - 0.000_117_8 * t2 - 0.000_000_155 * t3
        + 0.000_33 * (RAD_PER_DEG * (166.56 + 132.87 * t - 0.009_173 * t2)).sin();
    let sa = RAD_PER_DEG * (359.2242 + 29.105_356_08 * k - 0.000_033_3 * t2 - 0.000_003_47 * t3);
    let ma = RAD_PER_DEG * (306.0253 + 385.816_918_06 * k + 0.010_730_6 * t2 + 0.000_012_36 * t3);
    let tf =
        RAD_PER_DEG * 2.0 * (21.2964 + 390.670_506_46 * k - 0.001_652_8 * t2 - 0.000_002_39 * t3);
    let xtra = (0.1734 - 0.000_393 * t) * sa.sin() + 0.0021 * (sa * 2.0).sin() - 0.4068 * ma.sin()
        + 0.0161 * (2.0 * ma).sin()
        - 0.0004 * (3.0 * ma).sin()
        + 0.0104 * tf.sin()
        - 0.0051 * (sa + ma).sin()
        - 0.0074 * (sa - ma).sin()
        + 0.0004 * (tf + sa).sin()
        - 0.0004 * (tf - sa).sin()
        - 0.0006 * (tf + ma).sin()
        + 0.0010 * (tf - ma).sin()
        + 0.0005 * (sa + 2.0 * ma).sin();
    jd += xtra - (0.41 + 1.2053 * t + 0.4992 * t2) / 1440.0;
    jd
}

/// `Calendar_hijri::getHijri` from a Julian day number: `(day, month, year)`
/// with negative years before the Hijra.
fn hijri_from_julian_day(julday: i64) -> (i64, i64, i64) {
    let jd = julday as f64;
    let synmonth = (0.5 + (jd - JD1900) / SYN_PERIOD) as i64;
    let mut newsyn = synmonth;
    let prevday = julday as f64 - 0.5;
    let mut newjd;
    // Bounded: the estimate is within a month or two of the answer.
    let mut guard = 0;
    loop {
        newjd = new_moon(newsyn);
        newsyn -= 1;
        guard += 1;
        if newjd <= prevday || guard > 64 {
            break;
        }
    }
    newsyn += 1;
    let syndiff = newsyn - SYN_REF;
    let day = (julday as f64 - newjd + 0.5) as i64;
    let mut month = (syndiff % 12) + 1;
    let mut year = GREG_REF + syndiff / 12;
    if syndiff != 0 && month <= 0 {
        month += 12;
        year -= 1;
    }
    if year <= 0 {
        year -= 1;
    }
    (day, month, year)
}

// --- Hebrew (calendar_jewish.cxx, after Dershowitz & Reingold) --------------

const HEBREW_EPOCH: i64 = -1_373_429;

fn hebrew_leap_year(year: i64) -> bool {
    ((7 * year) + 1).rem_euclid(19) < 7
}

fn last_month_of_hebrew_year(year: i64) -> i64 {
    if hebrew_leap_year(year) {
        13
    } else {
        12
    }
}

fn hebrew_elapsed_days(year: i64) -> i64 {
    let months =
        235 * ((year - 1) / 19) + 12 * ((year - 1) % 19) + (7 * ((year - 1) % 19) + 1) / 19;
    let parts = 204 + 793 * (months % 1080);
    let hours = 5 + 12 * months + 793 * (months / 1080) + parts / 1080;
    let conj_day = 1 + 29 * months + hours / 24;
    let conj_parts = 1080 * (hours % 24) + parts % 1080;
    let alt = if conj_parts >= 19440
        || (conj_day % 7 == 2 && conj_parts >= 9924 && !hebrew_leap_year(year))
        || (conj_day % 7 == 1 && conj_parts >= 16789 && hebrew_leap_year(year - 1))
    {
        conj_day + 1
    } else {
        conj_day
    };
    if matches!(alt % 7, 0 | 3 | 5) {
        alt + 1
    } else {
        alt
    }
}

fn days_in_hebrew_year(year: i64) -> i64 {
    hebrew_elapsed_days(year + 1) - hebrew_elapsed_days(year)
}

fn last_day_of_hebrew_month(month: i64, year: i64) -> i64 {
    let short = matches!(month, 2 | 4 | 6 | 10 | 13)
        || (month == 8 && days_in_hebrew_year(year) % 10 != 5)
        || (month == 9 && days_in_hebrew_year(year) % 10 == 3)
        || (month == 12 && !hebrew_leap_year(year));
    if short {
        29
    } else {
        30
    }
}

fn hebrew_to_absolute(month: i64, day: i64, year: i64) -> i64 {
    let mut doy = day;
    if month < 7 {
        for m in 7..=last_month_of_hebrew_year(year) {
            doy += last_day_of_hebrew_month(m, year);
        }
        for m in 1..month {
            doy += last_day_of_hebrew_month(m, year);
        }
    } else {
        for m in 7..month {
            doy += last_day_of_hebrew_month(m, year);
        }
    }
    doy + hebrew_elapsed_days(year) + HEBREW_EPOCH
}

/// `HebrewDate(int d)`: `(year, month, day)` of an absolute (R.D.) date.
fn hebrew_from_absolute(d: i64) -> (i64, i64, i64) {
    let mut year = (d + HEBREW_EPOCH) / 366;
    while d >= hebrew_to_absolute(7, 1, year + 1) {
        year += 1;
    }
    let mut month = if d < hebrew_to_absolute(1, 1, year) {
        7
    } else {
        1
    };
    while d > hebrew_to_absolute(month, last_day_of_hebrew_month(month, year), year) {
        month += 1;
    }
    let day = d - hebrew_to_absolute(month, 1, year) + 1;
    (year, month, day)
}

// --- Names (localedata) ----------------------------------------------------

/// `(abbreviated, full)` names.
type Names = &'static [(&'static str, &'static str)];

const JA_DAYS: Names = &[
    ("日", "日曜日"),
    ("月", "月曜日"),
    ("火", "火曜日"),
    ("水", "水曜日"),
    ("木", "木曜日"),
    ("金", "金曜日"),
    ("土", "土曜日"),
];
const GENGOU_ERA_NAMES: Names = &[
    ("?.", "元号のサポート範囲外"),
    ("明", "明治"),
    ("大", "大正"),
    ("昭", "昭和"),
    ("平", "平成"),
    // AOO 4.1 ships "苓" / "苓和" (a typo); ICU and LibreOffice use 令和.
    ("令", "令和"),
];
const ROC_ERA_NAMES: Names = &[("民國前", "中華民國前"), ("民國", "中華民國")];
const BUDDHIST_ERA_NAMES: Names = &[("ปีก่อน พ.ศ. ที่", "ปีก่อนพุทธศักราชที่"), ("พ.ศ.", "พุทธศักราช")];
const HANJA_ERA_NAMES: Names = &[("紀元前", "西曆紀元前"), ("西紀", "西曆紀元")];
const HIJRI_ERA_NAMES: Names = &[("ه\u{200d}", "قبل الهجرة"), ("", "هجري")];
const JEWISH_ERA_NAMES: Names = &[("לפנה\"ס", "לפנה\"ס"), ("לסה\"נ", "לסה\"נ")];
const ZH_DAYS: Names = &[
    ("日", "星期日"),
    ("一", "星期一"),
    ("二", "星期二"),
    ("三", "星期三"),
    ("四", "星期四"),
    ("五", "星期五"),
    ("六", "星期六"),
];
const ZH_MONTHS: Names = &[
    ("一月", "一月"),
    ("二月", "二月"),
    ("三月", "三月"),
    ("四月", "四月"),
    ("五月", "五月"),
    ("六月", "六月"),
    ("七月", "七月"),
    ("八月", "八月"),
    ("九月", "九月"),
    ("十月", "十月"),
    ("十一月", "十一月"),
    ("十二月", "十二月"),
];
const HANJA_MONTHS: Names = &[
    ("1", "1"),
    ("2", "2"),
    ("3", "3"),
    ("4", "4"),
    ("5", "5"),
    ("6", "6"),
    ("7", "7"),
    ("8", "8"),
    ("9", "9"),
    ("10", "10"),
    ("11", "11"),
    ("12", "12"),
];
const TH_DAYS: Names = &[
    ("อา.", "อาทิตย์"),
    ("จ.", "จันทร์"),
    ("อ.", "อังคาร"),
    ("พ.", "พุธ"),
    ("พฤ.", "พฤหัสบดี"),
    ("ศ.", "ศุกร์"),
    ("ส.", "เสาร์"),
];
const TH_MONTHS: Names = &[
    ("ม.ค.", "มกราคม"),
    ("ก.พ.", "กุมภาพันธ์"),
    ("มี.ค.", "มีนาคม"),
    ("เม.ย.", "เมษายน"),
    ("พ.ค.", "พฤษภาคม"),
    ("มิ.ย.", "มิถุนายน"),
    ("ก.ค.", "กรกฎาคม"),
    ("ส.ค.", "สิงหาคม"),
    ("ก.ย.", "กันยายน"),
    ("ต.ค.", "ตุลาคม"),
    ("พ.ย.", "พฤศจิกายน"),
    ("ธ.ค.", "ธันวาคม"),
];
const AR_DAYS: Names = &[
    ("ح", "الأحد"),
    ("ن", "الاثنين"),
    ("ث", "الثلاثاء"),
    ("ر", "الأربعاء"),
    ("خ", "الخميس"),
    ("ج", "الجمعة"),
    ("س", "السبت"),
];
const HIJRI_MONTHS: Names = &[
    ("محرم", "محرم"),
    ("صفر", "صفر"),
    ("ربيع الأول", "ربيع الأول"),
    ("ربيع الآخر", "ربيع الآخر"),
    ("جمادى الأولى", "جمادى الأولى"),
    ("جمادى الآخرة", "جمادى الآخرة"),
    ("رجب", "رجب"),
    ("شعبان", "شعبان"),
    ("رمضان", "رمضان"),
    ("شوال", "شوال"),
    ("ذو القعدة", "ذو القعدة"),
    ("ذو الحجة", "ذو الحجة"),
];
const HE_DAYS: Names = &[
    ("א", "ראשון"),
    ("ב", "שני"),
    ("ג", "שלישי"),
    ("ד", "רביעי"),
    ("ה", "חמישי"),
    ("ו", "שישי"),
    ("ש", "שבת"),
];
const JEWISH_MONTHS: Names = &[
    ("ניסן", "ניסן"),
    ("אייר", "אייר"),
    ("סיון", "סיון"),
    ("תמוז", "תמוז"),
    ("אב", "אב"),
    ("אלול", "אלול"),
    ("תשרי", "תשרי"),
    ("חשון", "חשון"),
    ("כסליו", "כסליו"),
    ("טבת", "טבת"),
    ("שבט", "שבט"),
    ("אדר", "אדר"),
    ("אדר-ב", "אדר-ב"),
];

/// Gregorian era names of languages Tally has no locale for (localedata).
fn gregorian_eras(lang: LangId) -> Option<Names> {
    Some(match lang.primary() {
        0x04 if lang.0 == 0x0404 => &[("西元前", "西元前"), ("", "西元")],
        0x04 => &[("前", "公元前"), ("", "公元")],
        0x12 => &[("기원전", "서력기원전"), ("서기", "서력기원")],
        0x1E => &[("ปีก่อน ค.ศ. ที่", "ปีก่อนคริสต์ศักราชที่"), ("ค.ศ.", "คริสต์ศักราช")],
        0x01 => &[("ق.م", "قبل الميلاد"), ("م", "ميلادي")],
        0x0D => &[("לפנה\"ס", "לפנה\"ס"), ("לסה\"נ", "לסה\"נ")],
        _ => return None,
    })
}

/// Era names of a calendar; `None` means the locale's Gregorian eras.
pub fn era_names(cal: CalendarId, lang: LangId) -> Option<Names> {
    match cal {
        CalendarId::Gregorian => gregorian_eras(lang),
        CalendarId::Gengou => Some(GENGOU_ERA_NAMES),
        CalendarId::Roc => Some(ROC_ERA_NAMES),
        CalendarId::Buddhist => Some(BUDDHIST_ERA_NAMES),
        CalendarId::Hanja => Some(HANJA_ERA_NAMES),
        CalendarId::Hijri => Some(HIJRI_ERA_NAMES),
        CalendarId::Jewish => Some(JEWISH_ERA_NAMES),
    }
}

/// Weekday names of a calendar; `None` means the locale's.
pub fn day_names(cal: CalendarId) -> Option<Names> {
    match cal {
        CalendarId::Gengou | CalendarId::Hanja => Some(JA_DAYS),
        CalendarId::Roc => Some(ZH_DAYS),
        CalendarId::Buddhist => Some(TH_DAYS),
        CalendarId::Hijri => Some(AR_DAYS),
        CalendarId::Jewish => Some(HE_DAYS),
        CalendarId::Gregorian => None,
    }
}

/// Month names of a calendar; `None` means the locale's (also for gengou:
/// AOO's ja_JP gengou calendar has English month names, Tally keeps the
/// ja-JP locale's `1月`).
pub fn month_names(cal: CalendarId) -> Option<Names> {
    match cal {
        CalendarId::Roc => Some(ZH_MONTHS),
        CalendarId::Hanja => Some(HANJA_MONTHS),
        CalendarId::Buddhist => Some(TH_MONTHS),
        CalendarId::Hijri => Some(HIJRI_MONTHS),
        CalendarId::Jewish => Some(JEWISH_MONTHS),
        CalendarId::Gengou | CalendarId::Gregorian => None,
    }
}

// --- Display (getDisplayString) ---------------------------------------------

/// A numeric `CalendarDisplayCode`.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum NumCode {
    /// `SHORT_DAY` (`D`).
    ShortDay,
    /// `LONG_DAY` (`DD`).
    LongDay,
    /// `SHORT_MONTH` (`M`).
    ShortMonth,
    /// `LONG_MONTH` (`MM`).
    LongMonth,
    /// `SHORT_YEAR` (`YY`, `E`).
    ShortYear,
    /// `LONG_YEAR` (`YYYY`, `EE`, `R`).
    LongYear,
    /// `SHORT_QUARTER` / `LONG_QUARTER` (`Q`, `QQ`; text from the locale).
    Quarter,
}

/// `NatNumForCalendar` (calendar_gregorian.cxx): CJK calendars spell
/// numbers (NatNum1 → 4 or 7) except long years.
fn natnum_for_calendar(lang: LangId, code: NumCode, mode: u8, value: i64) -> u8 {
    let short = (matches!(code, NumCode::ShortYear | NumCode::LongYear) && value >= 100)
        || code == NumCode::Quarter;
    let p = lang.primary();
    let (zh, ja, ko) = (p == 0x04, p == 0x11, p == 0x12);
    if zh || ja || ko {
        return match mode {
            1 if !short => {
                if ja {
                    4
                } else {
                    7
                }
            }
            2 if !short => {
                if ja {
                    5
                } else {
                    8
                }
            }
            1..=3 => mode,
            4 if ko => {
                if short {
                    9
                } else {
                    11
                }
            }
            _ => 0,
        };
    }
    mode
}

/// The text of a numeric field (`Calendar_gregorian::getDisplayString`
/// and `Calendar_jewish::getDisplayString`) with NatNum `mode` (0 = none);
/// `lang` is the calendar's locale, `gannen` shows the first gengou year
/// as 元 also without a NatNum (Excel's `[$-ja-JP-x-gannen]`).
pub fn display_number(
    cal: CalendarId,
    lang: LangId,
    f: &Fields,
    code: NumCode,
    mode: u8,
    gannen: bool,
    sep: Separators,
) -> String {
    let year_code = matches!(code, NumCode::ShortYear | NumCode::LongYear);
    if cal == CalendarId::Gengou && year_code && f.year == 1 && (gannen || matches!(mode, 1 | 2)) {
        return "元".to_string();
    }
    let value = match code {
        NumCode::ShortDay | NumCode::LongDay => i64::from(f.day),
        NumCode::ShortMonth | NumCode::LongMonth | NumCode::Quarter => i64::from(f.month),
        NumCode::ShortYear | NumCode::LongYear => f.year,
    };
    let text = match code {
        NumCode::ShortDay | NumCode::ShortMonth | NumCode::Quarter => value.to_string(),
        NumCode::LongDay | NumCode::LongMonth => format!("{value:02}"),
        NumCode::LongYear if cal == CalendarId::Gengou => format!("{value:02}"),
        NumCode::LongYear => value.to_string(),
        NumCode::ShortYear if cal == CalendarId::Jewish => (value % 1000).to_string(),
        NumCode::ShortYear if value < 100 || cal.eras().is_some() => value.to_string(),
        NumCode::ShortYear => format!("{:02}", value % 100),
    };
    calendar_natnum(text, cal, lang, code, value, mode, sep)
}

/// Applies a section's NatNum to a calendar field's text as
/// `getDisplayString` does (the Jewish calendar always uses Hebrew
/// numbers; CJK languages spell short values, see `NatNumForCalendar`).
pub fn calendar_natnum(
    text: String,
    cal: CalendarId,
    lang: LangId,
    code: NumCode,
    value: i64,
    mode: u8,
    sep: Separators,
) -> String {
    let mode = if cal == CalendarId::Jewish && code != NumCode::Quarter {
        2
    } else {
        mode
    };
    if mode == 0 {
        return text;
    }
    let mode = natnum_for_calendar(lang, code, mode, value);
    if mode == 0 {
        return text;
    }
    match transliterate(&text, lang, mode, sep) {
        std::borrow::Cow::Borrowed(_) => text,
        std::borrow::Cow::Owned(t) => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::{civil_from_days, days_from_civil};

    fn at(cal: CalendarId, y: i64, m: u32, d: u32) -> Fields {
        let u = days_from_civil(y, m, d);
        fields(cal, y, m, d, u, crate::date::weekday_sun0(u))
    }

    #[test]
    fn era_calendars() {
        // Gengou boundaries (gengou_eraArray).
        let g = |y, m, d| {
            let f = at(CalendarId::Gengou, y, m, d);
            (f.era, f.year)
        };
        assert_eq!(g(1867, 12, 31), (0, 1));
        assert_eq!(g(1868, 1, 1), (1, 1));
        assert_eq!(g(1912, 7, 29), (1, 45));
        assert_eq!(g(1912, 7, 30), (2, 1));
        assert_eq!(g(1926, 12, 24), (2, 15));
        assert_eq!(g(1926, 12, 25), (3, 1));
        assert_eq!(g(1989, 1, 7), (3, 64));
        assert_eq!(g(1989, 1, 8), (4, 1));
        assert_eq!(g(2019, 4, 30), (4, 31));
        assert_eq!(g(2019, 5, 1), (5, 1));
        assert_eq!(g(2026, 11, 10), (5, 8));
        assert!(falls_back(
            CalendarId::Gengou,
            &at(CalendarId::Gengou, 1800, 1, 1)
        ));
        // ROC: 1912 is Minguo 1, 1911 is 1 before.
        let r = |y| {
            let f = at(CalendarId::Roc, y, 6, 1);
            (f.era, f.year)
        };
        assert_eq!(r(2026), (1, 115));
        assert_eq!(r(1912), (1, 1));
        assert_eq!(r(1911), (0, 1));
        assert_eq!(r(1900), (0, 12));
        // Buddhist: +543.
        assert_eq!(at(CalendarId::Buddhist, 2026, 1, 1).year, 2569);
        assert_eq!(at(CalendarId::Buddhist, 2026, 1, 1).era, 1);
        // Gregorian and Hanja: era 1 = AD.
        assert_eq!(at(CalendarId::Hanja, 2026, 3, 4).year, 2026);
        assert_eq!(at(CalendarId::Gregorian, 2026, 3, 4).era, 1);
    }

    #[test]
    fn hijri() {
        // AOO's reference point: 2001-03-26 is 1 Muharram 1422.
        let f = at(CalendarId::Hijri, 2001, 3, 26);
        assert_eq!((f.year, f.month, f.day, f.era), (1422, 1, 1, 1));
        // AOO counts from the astronomical new moon (2026-11-09 05:02 UT),
        // not the sighted crescent: 2026-11-10 is 1 Jumada al-Akhirah 1448.
        let f = at(CalendarId::Hijri, 2026, 11, 10);
        assert_eq!((f.year, f.month, f.day), (1448, 6, 1), "{f:?}");
        // 2000-01-01: 24 Ramadan 1420 (Umm al-Qura 25; astronomical 24).
        let f = at(CalendarId::Hijri, 2000, 1, 1);
        assert_eq!((f.year, f.month), (1420, 9));
        assert!((24..=25).contains(&f.day), "{f:?}");
    }

    #[test]
    fn jewish() {
        // Rosh Hashanah 5787 = 2026-09-12.
        let f = at(CalendarId::Jewish, 2026, 9, 12);
        assert_eq!((f.year, f.month, f.day, f.era), (5787, 7, 1, 1));
        let f = at(CalendarId::Jewish, 2026, 9, 11);
        assert_eq!((f.year, f.month, f.day), (5786, 6, 29));
        // Passover 5786: 15 Nisan = 2026-04-02.
        let f = at(CalendarId::Jewish, 2026, 4, 2);
        assert_eq!((f.year, f.month, f.day), (5786, 1, 15));
        // A leap year has Adar II (month 13): 5784, 2024-03-11 = 1 Adar II.
        let f = at(CalendarId::Jewish, 2024, 3, 11);
        assert_eq!((f.year, f.month, f.day), (5784, 13, 1));
        // Round trip over a range.
        for u in (days_from_civil(1900, 1, 1)..days_from_civil(2100, 1, 1)).step_by(97) {
            let (y, m, d) = civil_from_days(u);
            let f = fields(CalendarId::Jewish, y, m, d, u, 0);
            assert_eq!(
                hebrew_to_absolute(f.month as i64, f.day as i64, f.year),
                u + 719_163
            );
        }
    }

    #[test]
    fn display() {
        let ja = LangId::JA;
        let sep = Separators::EN;
        let reiwa1 = at(CalendarId::Gengou, 2019, 5, 1);
        let r8 = at(CalendarId::Gengou, 2026, 11, 10);
        let d = |cal, f: &Fields, code, mode| display_number(cal, ja, f, code, mode, false, sep);
        assert_eq!(d(CalendarId::Gengou, &r8, NumCode::ShortYear, 0), "8");
        assert_eq!(d(CalendarId::Gengou, &r8, NumCode::LongYear, 0), "08");
        assert_eq!(d(CalendarId::Gengou, &reiwa1, NumCode::ShortYear, 0), "1");
        assert_eq!(d(CalendarId::Gengou, &reiwa1, NumCode::ShortYear, 1), "元");
        assert_eq!(
            display_number(
                CalendarId::Gengou,
                ja,
                &reiwa1,
                NumCode::LongYear,
                0,
                true,
                sep
            ),
            "元"
        );
        assert_eq!(d(CalendarId::Gengou, &r8, NumCode::ShortYear, 1), "八");
        assert_eq!(d(CalendarId::Gengou, &r8, NumCode::LongDay, 1), "十");
        assert_eq!(d(CalendarId::Gengou, &r8, NumCode::ShortMonth, 1), "十一");
        assert_eq!(d(CalendarId::Gengou, &r8, NumCode::ShortMonth, 2), "拾壱");
        assert_eq!(d(CalendarId::Gengou, &r8, NumCode::ShortMonth, 3), "１１");
        assert_eq!(d(CalendarId::Gengou, &r8, NumCode::ShortMonth, 4), "11");
        // A Gregorian long year stays digit by digit (value >= 100).
        let g = at(CalendarId::Gregorian, 2026, 11, 10);
        assert_eq!(
            d(CalendarId::Gregorian, &g, NumCode::LongYear, 1),
            "二〇二六"
        );
        assert_eq!(d(CalendarId::Gregorian, &g, NumCode::ShortYear, 0), "26");
        // Korean NatNum4 → Hangul digits / short spelled.
        let ko = |code, f: &Fields| {
            display_number(CalendarId::Gregorian, LangId::KO, f, code, 4, false, sep)
        };
        assert_eq!(ko(NumCode::LongYear, &g), "이영이육");
        assert_eq!(ko(NumCode::ShortDay, &g), "십");
        // ROC / Buddhist short years keep all digits.
        let roc = at(CalendarId::Roc, 2026, 1, 2);
        assert_eq!(
            display_number(
                CalendarId::Roc,
                LangId::ZH_TW,
                &roc,
                NumCode::ShortYear,
                0,
                false,
                sep
            ),
            "115"
        );
        let be = at(CalendarId::Buddhist, 2026, 1, 2);
        assert_eq!(
            display_number(
                CalendarId::Buddhist,
                LangId::TH,
                &be,
                NumCode::ShortYear,
                1,
                false,
                sep
            ),
            "๒๕๖๙"
        );
        // Jewish: Hebrew numbers, short year = last three digits.
        let j = at(CalendarId::Jewish, 2026, 9, 12);
        let he = |code| display_number(CalendarId::Jewish, LangId::HE, &j, code, 0, false, sep);
        assert_eq!(he(NumCode::ShortYear), "תשפ״ז");
        assert_eq!(he(NumCode::LongYear), "ה׳ תשפ״ז");
        assert_eq!(he(NumCode::ShortDay), "א׳");
    }

    #[test]
    fn names_and_lookup() {
        assert_eq!(CalendarId::from_name("Gengou"), Some(CalendarId::Gengou));
        assert_eq!(CalendarId::from_name("roc"), Some(CalendarId::Roc));
        assert_eq!(CalendarId::from_name("persian"), None);
        assert_eq!(other_calendar(LangId::JA), Some(CalendarId::Gengou));
        assert_eq!(other_calendar(LangId::ZH_TW), Some(CalendarId::Roc));
        assert_eq!(other_calendar(LangId::ZH_CN), None);
        assert_eq!(other_calendar(LangId(0x0C01)), Some(CalendarId::Hijri));
        assert_eq!(other_calendar(LangId(0x2001)), None);
        assert!(has_calendar(LangId::EN_US, CalendarId::Gregorian));
        assert!(!has_calendar(LangId::EN_US, CalendarId::Buddhist));
        assert_eq!(GENGOU_ERA_NAMES[5].1, "令和");
        assert_eq!(
            CalendarId::from_excel(3),
            Some((CalendarId::Gengou, LangId::JA))
        );
        assert_eq!(CalendarId::Buddhist.excel_byte(), 7);
    }
}
