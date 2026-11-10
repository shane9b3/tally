//! Table-driven formatting cases.
//!
//! Expectations follow LibreOffice/OOo `TEXT()` semantics; where OOo and
//! Excel differ the comment says so and OOo wins.

use tally_numfmt::{format_value, format_value_with, parse_format, AlignHint, Locale};
use tally_types::{DateEpoch, ErrorKind, Value};

const RED: u32 = 0xFF0000;
const BLUE: u32 = 0x0000FF;
const GREEN: u32 = 0x00FF00;

/// Serial of 2026-10-19 in the 1899 epoch.
const D: f64 = 46314.0;
/// 14:30 as a fraction of a day.
const T1430: f64 = 14.5 / 24.0;

/// `(value, code, expected)`, default epoch, no width.
const NUM: &[(f64, &str, &str)] = &[
    // --- General / Standard ---
    (0.0, "General", "0"),
    (1.0, "General", "1"),
    (-1.0, "General", "-1"),
    (1.5, "General", "1.5"),
    (0.1 + 0.2, "General", "0.3"),
    (1.0 / 3.0, "General", "0.333333333333333"),
    (2.0 / 3.0, "General", "0.666666666666667"),
    (123_456_789.0, "General", "123456789"),
    (12345.678, "General", "12345.678"),
    (100.0, "General", "100"),
    (1e14, "General", "100000000000000"),
    (999_999_999_999_999.0, "General", "999999999999999"),
    (1e15, "General", "1E+15"),
    (1_234_567_890_123_450.0, "General", "1.23456789012345E+15"),
    (1e100, "General", "1E+100"),
    (1e-4, "General", "0.0001"),
    (0.000123, "General", "0.000123"),
    (1.5e-5, "General", "1.5E-05"),
    (-1e-10, "General", "-1E-10"),
    (1e-300, "General", "1E-300"),
    (-0.0, "General", "0"),
    (1.5, "Standard", "1.5"),
    (1.5, "general", "1.5"),
    (1.5, "", "1.5"),
    (5.0, "General\" kg\"", "5 kg"),
    (-5.0, "General;(General)", "(5)"),
    (f64::NAN, "General", "#NUM!"),
    (f64::INFINITY, "0.00", "#NUM!"),
    // --- 0 ---
    (0.0, "0", "0"),
    (1.4, "0", "1"),
    (1.5, "0", "2"),
    (2.5, "0", "3"),
    (-1.5, "0", "-2"),
    (0.5, "0", "1"),
    (1_234_567.0, "0", "1234567"),
    // OOo/LibreOffice show no sign when the value rounds to zero (Excel: -0).
    (-0.4, "0", "0"),
    (-0.0, "0", "0"),
    (1e20, "0", "100000000000000000000"),
    (1.234_567_890_123_456_7e18, "0", "1234567890123460000"),
    // --- 0.00 ---
    (1.0, "0.00", "1.00"),
    (0.0, "0.00", "0.00"),
    (2.675, "0.00", "2.68"),
    (0.125, "0.00", "0.13"),
    (1.005, "0.00", "1.01"),
    (0.995, "0.00", "1.00"),
    (-1.234, "0.00", "-1.23"),
    (1e-10, "0.00", "0.00"),
    (-1e-10, "0.00", "0.00"),
    (-0.004, "0.00", "0.00"),
    (-0.005, "0.00", "-0.01"),
    (123_456.789, "0.00", "123456.79"),
    (1e-20, "0.00", "0.00"),
    (0.1 + 0.2, "0.00000000000000000", "0.30000000000000000"),
    // --- #,##0 ---
    (0.0, "#,##0", "0"),
    (5.0, "#,##0", "5"),
    (999.0, "#,##0", "999"),
    (1000.0, "#,##0", "1,000"),
    (1_234_567.0, "#,##0", "1,234,567"),
    (-1_234_567.0, "#,##0", "-1,234,567"),
    (1234.5, "#,##0", "1,235"),
    (999_999.5, "#,##0", "1,000,000"),
    (1e12, "#,##0", "1,000,000,000,000"),
    (1e20, "#,##0", "100,000,000,000,000,000,000"),
    // --- #,##0.00 ---
    (1234.567, "#,##0.00", "1,234.57"),
    (-1234.567, "#,##0.00", "-1,234.57"),
    (0.5, "#,##0.00", "0.50"),
    (0.0, "#,##0.00", "0.00"),
    (1_000_000.0, "#,##0.00", "1,000,000.00"),
    (999.995, "#,##0.00", "1,000.00"),
    (12.0, "#,##0.00", "12.00"),
    (123_456_789.123, "#,##0.00", "123,456,789.12"),
    // --- scaling commas ---
    (1_234_567.0, "#,##0,", "1,235"),
    (999.0, "#,##0,", "1"),
    (499.0, "#,##0,", "0"),
    (12_345_678.0, "0,,\"M\"", "12M"),
    (12_345_678.0, "0.0,,\"M\"", "12.3M"),
    (1_234_567.0, "#,##0.0,", "1,234.6"),
    (1_500_000.0, "0.0,,", "1.5"),
    (2500.0, "0,\"K\"", "3K"),
    // --- optional digits ---
    (5.0, "#.##", "5."),
    (1.5, "#.##", "1.5"),
    (0.25, "#.##", ".25"),
    (1.255, "#.##", "1.26"),
    (0.0, "#", ""),
    (1.0, "0.##", "1."),
    (1.1, "0.##", "1.1"),
    (1.0, "0.0#", "1.0"),
    (1.25, "0.0#", "1.25"),
    (1.256, "0.0#", "1.26"),
    (1.5, "?.??", "1.5 "),
    (0.25, "?.??", " .25"),
    (12.0, "???", " 12"),
    (5.0, "#,###", "5"),
    // --- leading zeros and literals ---
    (123.0, "00000", "00123"),
    (5.0, "0,000", "0,005"),
    (123_456_789.0, "000-00-0000", "123-45-6789"),
    (5_551_234_567.0, "(000) 000-0000", "(555) 123-4567"),
    (5.0, "0\" units\"", "5 units"),
    (-5.0, "0\" units\"", "-5 units"),
    (-5.0, "\"$\"0.00", "-$5.00"),
    (1234.5, "$#,##0.00", "$1,234.50"),
    (-1234.5, "$#,##0.00", "-$1,234.50"),
    (5.0, "\\$0", "$5"),
    (5.0, "0_)", "5 "),
    (5.0, "_(0_)", " 5 "),
    (5.0, "\"x\"\\ 0", "x 5"),
    (7.0, "0\"/\"0", "0/7"),
    (3.0, "+0", "+3"),
    (5.0, "0 \"pcs\";-0 \"pcs\"", "5 pcs"),
    (5.0, "[NatNum1]0", "5"),
    (5.0, "[DBNum1]0", "5"),
    (5.0, "[~buddhist]0", "5"),
    // --- percent ---
    (0.15, "0%", "15%"),
    (0.155, "0%", "16%"),
    (1.0, "0%", "100%"),
    (-0.5, "0%", "-50%"),
    (0.0, "0%", "0%"),
    (0.12345, "0.00%", "12.35%"),
    (0.001, "0.00%", "0.10%"),
    (1.5, "0.00%", "150.00%"),
    (0.07, "0.0%", "7.0%"),
    (0.0001, "0%%", "1%%"),
    // --- currency ---
    (1234.5, "[$$-409]#,##0.00", "$1,234.50"),
    (-5.0, "[$$-409]#,##0.00", "-$5.00"),
    (5.0, "[$€-407]#,##0.00", "€5.00"),
    (5.0, "#,##0.00 [$€-407]", "5.00 €"),
    (5.0, "[$USD] #,##0.00", "USD 5.00"),
    (5.0, "[$-409]0.00", "5.00"),
    (0.0, "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00", "$0.00"),
    // --- scientific ---
    (12345.0, "0.00E+00", "1.23E+04"),
    (0.000123, "0.00E+00", "1.23E-04"),
    (0.0, "0.00E+00", "0.00E+00"),
    (-12345.0, "0.00E+00", "-1.23E+04"),
    (1.0, "0.00E+00", "1.00E+00"),
    (9.999, "0.00E+00", "1.00E+01"),
    (1e100, "0.00E+00", "1.00E+100"),
    (123_456_789.0, "0.00E+00", "1.23E+08"),
    (0.5, "0.00E+00", "5.00E-01"),
    (12345.0, "0.00E-00", "1.23E04"),
    (0.000123, "0.00E-00", "1.23E-04"),
    (12345.0, "0E+0", "1E+4"),
    (12345.0, "##0.0E+0", "12.3E+3"),
    (1_234_567.0, "##0.0E+0", "1.2E+6"),
    (123_456.0, "##0.0E+0", "123.5E+3"),
    (0.001_234_5, "##0.0E+0", "1.2E-3"),
    (12345.0, "00.00E+00", "12.35E+03"),
    (12345.0, "0.00e+00", "1.23e+04"),
    (1e-300, "0.0E+0", "1.0E-300"),
    // --- fractions ---
    (1.5, "# ?/?", "1 1/2"),
    (0.5, "# ?/?", " 1/2"),
    (3.0, "# ?/?", "3    "),
    (0.0, "# ?/?", "0    "),
    (0.3, "# ?/?", " 2/7"),
    (2.75, "# ?/?", "2 3/4"),
    (-1.5, "# ?/?", "-1 1/2"),
    (1.99, "# ?/?", "2    "),
    (0.333, "# ?/?", " 1/3"),
    (3.14158, "# ??/??", "3 14/99"),
    (0.5, "# ??/??", "  1/2 "),
    (1.25, "# ??/??", "1  1/4 "),
    (0.333, "# ???/???", " 332/997"),
    (1.5, "# ??/16", "1  8/16"),
    (0.0625, "# ??/16", "  1/16"),
    (2.0, "# ??/16", "2      "),
    (1.3, "# ?/8", "1 2/8"),
    (1.5, "?/?", "3/2"),
    (0.25, "?/?", "1/4"),
    (2.0, "?/?", "2/1"),
    (0.5, "0 ?/?", "0 1/2"),
    (5.5, "# ?/?\" in\"", "5 1/2 in"),
    (1.75, "# ?/4", "1 3/4"),
    (0.1, "# ?/10", " 1/10"),
    // --- dates (1899 epoch) ---
    (D, "MM/DD/YYYY", "10/19/2026"),
    (D, "YYYY-MM-DD", "2026-10-19"),
    (D, "DD MMM YYYY", "19 Oct 2026"),
    (D, "D/M/YY", "19/10/26"),
    (D, "M/D/YYYY", "10/19/2026"),
    (46023.0, "M/D/YYYY", "1/1/2026"),
    (46023.0, "MM/DD/YY", "01/01/26"),
    (D, "DDDD, MMMM D, YYYY", "Monday, October 19, 2026"),
    (D, "DDD", "Mon"),
    (D, "NN", "Mon"),
    (D, "NNN", "Monday"),
    (D, "NNNND", "Monday, 19"),
    (D, "MMMMM", "O"),
    (D, "MMM", "Oct"),
    (D, "MMMM", "October"),
    (D, "YY", "26"),
    (D, "YYYY", "2026"),
    (D, "Q", "Q4"),
    (D, "QQ", "4th quarter"),
    (D, "WW", "43"),
    // AOO ImpAppendEraG: G is the short era name outside gengou; E is
    // SHORT_YEAR (two digits from 100 on), EE / R LONG_YEAR, RR GGGEE
    // (calendar_gregorian.cxx getDisplayString). en-US's other calendar is
    // the Jewish one (en_US.xml): E / EE / R / RR / AAA switch to it, with
    // ASCII digits (NatNum2 is not valid for English) and SHORT_YEAR = year
    // % 1000. 2026-10-19 is 8 Heshvan 5787.
    (D, "G", "AD"),
    (D, "GG", "AD"),
    (D, "GGG", "AD"),
    (D, "E", "787"),
    (D, "EE", "5787"),
    (D, "R", "5787"),
    (D, "RR", "CE5787"),
    (D, "G E", "CE 787"),
    (D, "D MMMM EE", "8 Heshvan 5787"),
    (D, "AAAA", "Monday"),
    (D + 5.0, "AAAA", "Shabbat"),
    (D + 5.0, "AAA", "Sat"),
    (D, "yyyy-mm-dd", "2026-10-19"),
    (D, "dd.mm.yyyy", "19.10.2026"),
    (D + 0.5, "MM/DD/YYYY HH:MM", "10/19/2026 12:00"),
    (D + T1430, "MM/DD/YYYY HH:MM", "10/19/2026 14:30"),
    (D + 0.5, "YYYY-MM-DD HH:MM:SS", "2026-10-19 12:00:00"),
    (D + 0.75, "D MMM YYYY H:MM AM/PM", "19 Oct 2026 6:00 PM"),
    (0.0, "YYYY-MM-DD", "1899-12-30"),
    (1.0, "YYYY-MM-DD", "1899-12-31"),
    (2.0, "YYYY-MM-DD", "1900-01-01"),
    (60.0, "YYYY-MM-DD", "1900-02-28"),
    (61.0, "YYYY-MM-DD", "1900-03-01"),
    (-1.0, "YYYY-MM-DD", "1899-12-29"),
    (-693_593.0, "YYYY-MM-DD", "0001-01-01"),
    (-693_594.0, "YYYY-MM-DD", "###"),
    (2_958_465.0, "YYYY-MM-DD", "9999-12-31"),
    (2_958_466.0, "YYYY-MM-DD", "###"),
    (45351.0, "MM/DD/YYYY", "02/29/2024"),
    (36526.0, "YYYY-MM-DD", "2000-01-01"),
    (36526.9, "YYYY-MM-DD", "2000-01-01"),
    (-0.25, "YYYY-MM-DD HH:MM", "1899-12-29 18:00"),
    // --- times ---
    (0.5, "HH:MM", "12:00"),
    (0.75, "HH:MM", "18:00"),
    (0.0, "HH:MM", "00:00"),
    (T1430, "HH:MM", "14:30"),
    (0.1, "H:MM", "2:24"),
    (7509.0 / 86400.0, "HH:MM:SS", "02:05:09"),
    (7509.0 / 86400.0, "H:M:S", "2:5:9"),
    (0.0, "HH:MM AM/PM", "12:00 AM"),
    (0.5, "HH:MM AM/PM", "12:00 PM"),
    (13.5 / 24.0, "HH:MM AM/PM", "01:30 PM"),
    (13.5 / 24.0, "H:MM AM/PM", "1:30 PM"),
    (13.5 / 24.0, "h:mm am/pm", "1:30 PM"),
    (13.5 / 24.0, "H:MM A/P", "1:30 P"),
    (13.5 / 24.0, "h:mm a/p", "1:30 p"),
    (11.5 / 24.0, "H:MM AM/PM", "11:30 AM"),
    (23.99 / 24.0, "H AM/PM", "11 PM"),
    (86399.999 / 86400.0, "HH:MM:SS", "00:00:00"),
    (86399.999 / 86400.0, "HH:MM:SS.000", "23:59:59.999"),
    (86399.999 / 86400.0, "HH:MM:SS.00", "00:00:00.00"),
    (86399.999 / 86400.0, "HH:MM", "00:00"),
    (
        D + 86399.999 / 86400.0,
        "YYYY-MM-DD HH:MM:SS",
        "2026-10-20 00:00:00",
    ),
    // OOo rounds to the second before showing minutes (Excel truncates).
    (52199.6 / 86400.0, "HH:MM", "14:30"),
    (52199.4 / 86400.0, "HH:MM:SS", "14:29:59"),
    (0.5 + 0.2 / 86400.0, "HH:MM:SS.0", "12:00:00.2"),
    (5.0 / 86400.0, "S", "5"),
    (5.0 / 86400.0, "SS", "05"),
    // --- elapsed time ---
    (1.5, "[HH]:MM", "36:00"),
    (25.5 / 24.0, "[HH]:MM", "25:30"),
    (2.0 + 1.0 / 86400.0, "[H]:MM:SS", "48:00:01"),
    (1.0 / 24.0, "[MM]:SS", "60:00"),
    (1.0 / 1440.0, "[SS]", "60"),
    (100.0 / 24.0, "[HH]:MM:SS", "100:00:00"),
    (0.25, "[H]", "6"),
    (-0.5, "[HH]:MM", "-12:00"),
    (-0.25, "HH:MM", "-06:00"),
    // --- minutes vs months ---
    (60.0 / 86400.0, "MM:SS", "01:00"),
    (90.0 / 86400.0, "MM:SS", "01:30"),
    (D + T1430, "HH:MM MM/DD", "14:30 10/19"),
    (D + T1430, "MM", "10"),
    (D + T1430, "H\"h\"MM", "14h30"),
    // --- sections ---
    (5.0, "0;(0)", "5"),
    (-5.0, "0;(0)", "(5)"),
    (0.0, "0;(0)", "0"),
    (0.0, "0;-0;\"zero\"", "zero"),
    (-3.0, "0;-0;\"zero\"", "-3"),
    (3.0, "0;-0;\"zero\"", "3"),
    (-0.0001, "0.0;-0.0;\"-\"", "-0.0"),
    (0.0, "0.0;-0.0;\"-\"", "-"),
    (-5.0, "0;;", ""),
    (0.0, "0;;", ""),
    (5.0, "0;;", "5"),
    (5.0, ";;;", ""),
    (1.5, "0.00;-0.00;0.00;@", "1.50"),
    (-1.5, "0.00;-0.00;0.00;@", "-1.50"),
    (-1.5, "0.00;0.00", "1.50"),
    // --- conditions ---
    (150.0, "[>=100]\"big\";[<0]\"neg\";\"small\"", "big"),
    (100.0, "[>=100]\"big\";[<0]\"neg\";\"small\"", "big"),
    (-5.0, "[>=100]\"big\";[<0]\"neg\";\"small\"", "neg"),
    (50.0, "[>=100]\"big\";[<0]\"neg\";\"small\"", "small"),
    (-1.0, "[<0]\"neg\";[>0]\"pos\";\"zero\"", "neg"),
    (1.0, "[<0]\"neg\";[>0]\"pos\";\"zero\"", "pos"),
    (0.0, "[<0]\"neg\";[>0]\"pos\";\"zero\"", "zero"),
    // A first section with [<0] is "real negative": no automatic sign.
    (-5.0, "[<0]0;0", "5"),
    (5.0, "[<0]0;0", "5"),
    (1.0, "[=1]\"one\";0", "one"),
    (2.0, "[=1]\"one\";0", "2"),
    (-2.0, "[=1]\"one\";0", "2"),
    (150.0, "[>100]0.0", "150.0"),
    // No section matches: General.
    (50.0, "[>100]0.0", "50"),
    (-50.0, "[>100]0.0", "-50"),
    (0.0, "[<=0]\"np\";0", "np"),
    (-1.0, "[<=0]\"np\";0", "np"),
    (1.0, "[<=0]\"np\";0", "1"),
    (0.0, "[<>0]0;\"z\"", "z"),
    (-4.0, "[<>0]0;\"z\"", "-4"),
    (1.5, "[<-1.5]\"low\";0.0", "1.5"),
    (-2.0, "[<-1.5]\"low\";0.0", "low"),
    (5.0, "[>5]\"a\";[>2]\"b\";\"c\"", "b"),
    (1.0, "[>5]\"a\";[>2]\"b\";\"c\"", "c"),
    // --- text placeholder with numbers ---
    (5.0, "@", "5"),
    (1.5, "@", "1.5"),
    (5.0, "0;@", "5"),
    (-5.0, "0;@", "-5"),
];

/// `(value, code, expected text, expected colour)`.
const COLORED: &[(f64, &str, &str, Option<u32>)] = &[
    (-1.5, "0.00;[RED]-0.00", "-1.50", Some(RED)),
    (1.5, "0.00;[RED]-0.00", "1.50", None),
    (
        -1234.5,
        "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00",
        "-$1,234.50",
        Some(RED),
    ),
    (
        1234.5,
        "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00",
        "$1,234.50",
        None,
    ),
    (
        -1234.5,
        "$#,##0.00;[RED]-$#,##0.00",
        "-$1,234.50",
        Some(RED),
    ),
    (5.0, "[RED][>0]0;[BLUE][<0]0;[GREEN]0", "5", Some(RED)),
    (-5.0, "[RED][>0]0;[BLUE][<0]0;[GREEN]0", "5", Some(BLUE)),
    (0.0, "[RED][>0]0;[BLUE][<0]0;[GREEN]0", "0", Some(GREEN)),
    (5.0, "[>0][RED]0", "5", Some(RED)),
    (5.0, "[BLACK]0", "5", Some(0x000000)),
    (5.0, "[BLUE]0", "5", Some(BLUE)),
    (5.0, "[GREEN]0", "5", Some(GREEN)),
    (5.0, "[CYAN]0", "5", Some(0x00FFFF)),
    (5.0, "[MAGENTA]0", "5", Some(0xFF00FF)),
    (5.0, "[YELLOW]0", "5", Some(0xFFFF00)),
    (5.0, "[WHITE]0", "5", Some(0xFFFFFF)),
    (5.0, "[BROWN]0", "5", Some(0x808000)),
    (5.0, "[GREY]0", "5", Some(0x808080)),
    (5.0, "[GRAY]0", "5", Some(0x808080)),
    (5.0, "[red]0", "5", Some(RED)),
    (5.0, "[Red]0", "5", Some(RED)),
    (5.0, "[COLOR1]0", "5", Some(0x000000)),
    (5.0, "[COLOR3]0", "5", Some(0xFF0000)),
    (5.0, "[COLOR10]0", "5", Some(0x008000)),
    (5.0, "[COLOR56]0", "5", Some(0x333333)),
    (5.0, "[COLOR57]0", "5", Some(0x000000)),
    (5.0, "[COLOR64]0", "5", Some(0x00FFFF)),
    (5.0, "0", "5", None),
    (D, "[BLUE]YYYY-MM-DD", "2026-10-19", Some(BLUE)),
    (50.0, "[RED][>100]0", "50", None),
];

/// `(value, code, width, expected)`.
const WIDTH: &[(f64, &str, u16, &str)] = &[
    (1.0 / 3.0, "General", 5, "0.333"),
    (1.0 / 3.0, "General", 20, "0.333333333333333"),
    (1.0 / 3.0, "General", 1, "0"),
    (-1.0 / 3.0, "General", 5, "-0.33"),
    (123_456_789_012.0, "General", 6, "1E+11"),
    (123_456_789_012.0, "General", 4, "###"),
    (123_456_789_012.0, "General", 12, "123456789012"),
    (123_456_789_012.0, "General", 11, "1.23457E+11"),
    (3.14158, "General", 4, "3.14"),
    (3.14158, "General", 1, "3"),
    (9.99, "General", 3, "10"),
    (1e15, "General", 3, "###"),
    (1e15, "General", 5, "1E+15"),
    (1.5e15, "General", 6, "2E+15"),
    (1.5e15, "General", 7, "1.5E+15"),
    // Rounds to 0.00 and scientific needs 5 chars: shows 0.
    (0.001, "General", 4, "0"),
    (0.001, "General", 5, "0.001"),
    (0.000_012_345, "General", 7, "1.2E-05"),
    (123.456, "General", 5, "123.5"),
    (123.456, "General", 3, "123"),
    (999.9, "General", 3, "###"),
    (1234.5, "General", 4, "1235"),
    (1.5, "General", 0, "###"),
    (0.0, "General", 1, "0"),
    (-5.0, "General", 1, "###"),
    (-5.0, "General", 2, "-5"),
    (1.0 / 3.0, "General\" kg\"", 8, "0.333 kg"),
    (1234.5, "#,##0.00", 8, "1,234.50"),
    (1234.5, "#,##0.00", 7, "###"),
    (D, "MM/DD/YYYY", 9, "###"),
    (D, "MM/DD/YYYY", 10, "10/19/2026"),
    (12345.0, "0.00E+00", 7, "###"),
];

/// `(value, code, epoch, expected)`.
const EPOCH: &[(f64, &str, DateEpoch, &str)] = &[
    (46312.0, "YYYY-MM-DD", DateEpoch::D1900, "2026-10-19"),
    (0.0, "YYYY-MM-DD", DateEpoch::D1900, "1900-01-01"),
    (59.0, "YYYY-MM-DD", DateEpoch::D1900, "1900-03-01"),
    (44852.0, "YYYY-MM-DD", DateEpoch::D1904, "2026-10-19"),
    (0.0, "YYYY-MM-DD", DateEpoch::D1904, "1904-01-01"),
    (-1.0, "YYYY-MM-DD", DateEpoch::D1904, "1903-12-31"),
    (44852.5, "DDDD HH:MM", DateEpoch::D1904, "Monday 12:00"),
    (D, "YYYY-MM-DD", DateEpoch::D1899, "2026-10-19"),
    (1.5, "[HH]:MM", DateEpoch::D1904, "36:00"),
];

/// `(text value, code, expected, colour)`.
const TEXT: &[(&str, &str, &str, Option<u32>)] = &[
    ("abc", "@", "abc", None),
    ("abc", "\"Name: \"@", "Name: abc", None),
    ("abc", "0;-0;0;\"[\"@\"]\"", "[abc]", None),
    ("abc", "0.00", "abc", None),
    ("abc", "0;-0;0;[RED]@", "abc", Some(RED)),
    ("", "@", "", None),
    ("abc", "General", "abc", None),
    ("abc", "0;@", "abc", None),
    ("abc", "@\" !\"", "abc !", None),
    ("12", "@", "12", None),
    ("abc", ";;;", "", None),
    ("abc", "MM/DD/YYYY", "abc", None),
    ("abc", "0;0;0;\"fixed\"", "fixed", None),
    ("a\"b", "@@", "a\"ba\"b", None),
];

fn check(
    failures: &mut Vec<String>,
    v: &Value,
    code: &str,
    width: Option<u16>,
    epoch: DateEpoch,
    want: &str,
    want_color: Option<Option<u32>>,
) {
    let l = Locale::en_us();
    let ast = match parse_format(code) {
        Ok(a) => a,
        Err(e) => {
            failures.push(format!("{code:?}: parse error {e}"));
            return;
        }
    };
    let f = format_value_with(v, &ast, &l, width, epoch);
    if f.text != want {
        failures.push(format!(
            "{v:?} {code:?} width {width:?}: got {:?}, want {want:?}",
            f.text
        ));
    }
    if let Some(c) = want_color {
        if f.color != c {
            failures.push(format!("{v:?} {code:?}: colour {:?}, want {c:?}", f.color));
        }
    }
}

#[test]
fn format_table() {
    let mut failures = Vec::new();
    let mut count = 0;
    for &(x, code, want) in NUM {
        check(
            &mut failures,
            &Value::Number(x),
            code,
            None,
            DateEpoch::D1899,
            want,
            None,
        );
        count += 1;
    }
    for &(x, code, want, color) in COLORED {
        check(
            &mut failures,
            &Value::Number(x),
            code,
            None,
            DateEpoch::D1899,
            want,
            Some(color),
        );
        count += 1;
    }
    for &(x, code, w, want) in WIDTH {
        check(
            &mut failures,
            &Value::Number(x),
            code,
            Some(w),
            DateEpoch::D1899,
            want,
            None,
        );
        count += 1;
    }
    for &(x, code, epoch, want) in EPOCH {
        check(
            &mut failures,
            &Value::Number(x),
            code,
            None,
            epoch,
            want,
            None,
        );
        count += 1;
    }
    for &(s, code, want, color) in TEXT {
        check(
            &mut failures,
            &Value::text(s),
            code,
            None,
            DateEpoch::D1899,
            want,
            Some(color),
        );
        count += 1;
    }
    assert!(
        failures.is_empty(),
        "{} of {count} cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(count >= 250, "only {count} cases");
}

#[test]
fn non_numbers() {
    let l = Locale::en_us();
    let ast = parse_format("0.00").unwrap();
    let f = format_value(&Value::Bool(true), &ast, &l, None);
    assert_eq!((f.text.as_str(), f.align), ("TRUE", AlignHint::Right));
    let f = format_value(&Value::Bool(false), &ast, &l, Some(2));
    assert_eq!(f.text, "FALSE");
    let f = format_value(&Value::Error(ErrorKind::Div0), &ast, &l, None);
    assert_eq!((f.text.as_str(), f.align), ("#DIV/0!", AlignHint::Center));
    let f = format_value(&Value::Error(ErrorKind::NA), &ast, &l, None);
    assert_eq!(f.text, "#N/A");
    let f = format_value(&Value::Error(ErrorKind::InvalidArgument), &ast, &l, None);
    assert_eq!(f.text, "Err:502");
    let f = format_value(&Value::Empty, &ast, &l, None);
    assert_eq!((f.text.as_str(), f.align), ("", AlignHint::Left));
    let f = format_value(&Value::text("x"), &ast, &l, Some(0));
    assert_eq!((f.text.as_str(), f.align), ("x", AlignHint::Left));
    let f = format_value(&Value::Number(1.0), &ast, &l, None);
    assert_eq!(f.align, AlignHint::Right);
}

#[test]
fn fill_and_skip() {
    let l = Locale::en_us();
    let f = format_value(&Value::Number(5.0), &parse_format("0*-").unwrap(), &l, None);
    assert_eq!((f.text.as_str(), f.fill, f.fill_pos), ("5", Some('-'), 1));
    let f = format_value(&Value::Number(5.0), &parse_format("**0").unwrap(), &l, None);
    assert_eq!((f.text.as_str(), f.fill, f.fill_pos), ("5", Some('*'), 0));
    // The sign goes before the fill.
    let f = format_value(
        &Value::Number(-5.0),
        &parse_format("* 0").unwrap(),
        &l,
        None,
    );
    assert_eq!((f.text.as_str(), f.fill, f.fill_pos), ("-5", Some(' '), 1));
    let f = format_value(
        &Value::Number(5.0),
        &parse_format("_($* #,##0.00_)").unwrap(),
        &l,
        None,
    );
    assert_eq!(
        (f.text.as_str(), f.fill, f.fill_pos),
        (" $5.00 ", Some(' '), 2)
    );
    let f = format_value(&Value::text("ab"), &parse_format("@*.").unwrap(), &l, None);
    assert_eq!((f.text.as_str(), f.fill, f.fill_pos), ("ab", Some('.'), 2));
}

#[test]
fn performance_smoke() {
    // 1M numbers with #,##0.00 should take well under a second in release;
    // here we only run 100k so debug builds stay quick.
    let l = Locale::en_us();
    let ast = parse_format("#,##0.00").unwrap();
    let start = std::time::Instant::now();
    let mut total = 0usize;
    for i in 0..100_000 {
        let f = format_value(&Value::Number(i as f64 * 1.37), &ast, &l, None);
        total += f.text.len();
    }
    assert!(total > 0);
    eprintln!("100k #,##0.00 in {:?}", start.elapsed());
}
