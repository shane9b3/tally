//! The built-in worksheet functions of BIFF8 by their index in a
//! `ptgFunc` / `ptgFuncVar` token (the "Ftab" of [MS-XLS] 2.5.198.17).
//!
//! The facts (index, Excel name, fewest and most arguments) are taken
//! from the function tables of Apache OpenOffice 4.1
//! (`main/oox/source/xls/formulabase.cxx`, branch AOO41X, Apache License
//! 2.0: `saFuncTableBiff2` to `saFuncTableBiff8`, a later table replacing
//! an earlier entry with the same index). Left out are the macro-sheet
//! functions and commands (`GOTO`, `HALT`, `ADD.MENU`...: not functions of
//! a worksheet) and index 255, which calls a function by name and is
//! decoded by the reader itself. "30" is "as many as a BIFF8 call can
//! have".
//!
//! Two rows differ from that source:
//! - 204 `USDOLLAR` is given as `DOLLAR`: OpenOffice reads it as its
//!   DOLLAR function (`formulabase.cxx`: `{ "DOLLAR", "USDOLLAR", 204,
//!   204, 1, 2, … FUNCFLAG_IMPORTONLY }`; the binary filter,
//!   `main/sc/source/filter/excel/xlformula.cxx`: `{ ocCurrency, 204, 1,
//!   2, … EXC_FUNCFLAG_IMPORTONLY }`), and an `.xlsx` has no `USDOLLAR`.
//! - 379 `RTD` takes 3 to 30 arguments, not exactly 3: its topics repeat
//!   (Microsoft's description of the function: `RTD(ProgID, server,
//!   topic1, [topic2], …)`), and [MS-XLS] stores it with a count
//!   (`ptgFuncVar`).
//!
//! 261 `ERROR.TYPE` keeps Excel's name although OpenOffice reads it as
//! its ERRORTYPE (`xlformula.cxx`: `{ ocErrorType, 261, 1, 1, … }`): that
//! function returns OpenOffice's error numbers (532 for `#DIV/0!`, where
//! Excel's returns 2), the `.xlsx` reader does not map it either, and a
//! name that calls it is left out rather than calculated differently.
//!
//! The names are Excel's (English), the ones an `.xlsx` formula uses, so
//! the XLSX reader's conversion maps them to Tally's functions; whether
//! Tally has a function is decided there, not here.
//!
//! [`ADDINS`] and [`FUTURE`] are the functions a BIFF8 formula calls by
//! name (index 255), from the same source.

/// (index, Excel name, fewest arguments, most arguments), by index.
#[rustfmt::skip]
const FTAB: &[(u16, &str, u8, u8)] = &[
    (0, "COUNT", 0, 30), (1, "IF", 2, 3), (2, "ISNA", 1, 1), (3, "ISERROR", 1, 1),
    (4, "SUM", 0, 30), (5, "AVERAGE", 1, 30), (6, "MIN", 1, 30), (7, "MAX", 1, 30),
    (8, "ROW", 0, 1), (9, "COLUMN", 0, 1), (10, "NA", 0, 0), (11, "NPV", 2, 30),
    (12, "STDEV", 1, 30), (13, "DOLLAR", 1, 2), (14, "FIXED", 1, 3), (15, "SIN", 1, 1),
    (16, "COS", 1, 1), (17, "TAN", 1, 1), (18, "ATAN", 1, 1), (19, "PI", 0, 0),
    (20, "SQRT", 1, 1), (21, "EXP", 1, 1), (22, "LN", 1, 1), (23, "LOG10", 1, 1),
    (24, "ABS", 1, 1), (25, "INT", 1, 1), (26, "SIGN", 1, 1), (27, "ROUND", 2, 2),
    (28, "LOOKUP", 2, 3), (29, "INDEX", 2, 4), (30, "REPT", 2, 2), (31, "MID", 3, 3),
    (32, "LEN", 1, 1), (33, "VALUE", 1, 1), (34, "TRUE", 0, 0), (35, "FALSE", 0, 0),
    (36, "AND", 1, 30), (37, "OR", 1, 30), (38, "NOT", 1, 1), (39, "MOD", 2, 2),
    (40, "DCOUNT", 3, 3), (41, "DSUM", 3, 3), (42, "DAVERAGE", 3, 3), (43, "DMIN", 3, 3),
    (44, "DMAX", 3, 3), (45, "DSTDEV", 3, 3), (46, "VAR", 1, 30), (47, "DVAR", 3, 3),
    (48, "TEXT", 2, 2), (49, "LINEST", 1, 4), (50, "TREND", 1, 4), (51, "LOGEST", 1, 4),
    (52, "GROWTH", 1, 4), (56, "PV", 3, 5), (57, "FV", 3, 5), (58, "NPER", 3, 5),
    (59, "PMT", 3, 5), (60, "RATE", 3, 6), (61, "MIRR", 3, 3), (62, "IRR", 1, 2),
    (63, "RAND", 0, 0), (64, "MATCH", 2, 3), (65, "DATE", 3, 3), (66, "TIME", 3, 3),
    (67, "DAY", 1, 1), (68, "MONTH", 1, 1), (69, "YEAR", 1, 1), (70, "WEEKDAY", 1, 2),
    (71, "HOUR", 1, 1), (72, "MINUTE", 1, 1), (73, "SECOND", 1, 1), (74, "NOW", 0, 0),
    (75, "AREAS", 1, 1), (76, "ROWS", 1, 1), (77, "COLUMNS", 1, 1), (78, "OFFSET", 3, 5),
    (82, "SEARCH", 2, 3), (83, "TRANSPOSE", 1, 1), (86, "TYPE", 1, 1), (97, "ATAN2", 2, 2),
    (98, "ASIN", 1, 1), (99, "ACOS", 1, 1), (100, "CHOOSE", 2, 30), (101, "HLOOKUP", 3, 4),
    (102, "VLOOKUP", 3, 4), (105, "ISREF", 1, 1), (109, "LOG", 1, 2), (111, "CHAR", 1, 1),
    (112, "LOWER", 1, 1), (113, "UPPER", 1, 1), (114, "PROPER", 1, 1), (115, "LEFT", 1, 2),
    (116, "RIGHT", 1, 2), (117, "EXACT", 2, 2), (118, "TRIM", 1, 1), (119, "REPLACE", 4, 4),
    (120, "SUBSTITUTE", 3, 4), (121, "CODE", 1, 1), (124, "FIND", 2, 3), (125, "CELL", 1, 2),
    (126, "ISERR", 1, 1), (127, "ISTEXT", 1, 1), (128, "ISNUMBER", 1, 1),
    (129, "ISBLANK", 1, 1), (130, "T", 1, 1), (131, "N", 1, 1), (140, "DATEVALUE", 1, 1),
    (141, "TIMEVALUE", 1, 1), (142, "SLN", 3, 3), (143, "SYD", 4, 4), (144, "DDB", 4, 5),
    (148, "INDIRECT", 1, 2), (162, "CLEAN", 1, 1), (163, "MDETERM", 1, 1),
    (164, "MINVERSE", 1, 1), (165, "MMULT", 2, 2), (167, "IPMT", 4, 6), (168, "PPMT", 4, 6),
    (169, "COUNTA", 0, 30), (183, "PRODUCT", 0, 30), (184, "FACT", 1, 1),
    (189, "DPRODUCT", 3, 3), (190, "ISNONTEXT", 1, 1), (193, "STDEVP", 1, 30),
    (194, "VARP", 1, 30), (195, "DSTDEVP", 3, 3), (196, "DVARP", 3, 3), (197, "TRUNC", 1, 2),
    (198, "ISLOGICAL", 1, 1), (199, "DCOUNTA", 3, 3), (204, "DOLLAR", 1, 2),
    (205, "FINDB", 2, 3), (206, "SEARCHB", 2, 3), (207, "REPLACEB", 4, 4), (208, "LEFTB", 1, 2),
    (209, "RIGHTB", 1, 2), (210, "MIDB", 3, 3), (211, "LENB", 1, 1), (212, "ROUNDUP", 2, 2),
    (213, "ROUNDDOWN", 2, 2), (214, "ASC", 1, 1), (215, "DBCS", 1, 1), (216, "RANK", 2, 3),
    (219, "ADDRESS", 2, 5), (220, "DAYS360", 2, 3), (221, "TODAY", 0, 0), (222, "VDB", 5, 7),
    (227, "MEDIAN", 1, 30), (228, "SUMPRODUCT", 1, 30), (229, "SINH", 1, 1),
    (230, "COSH", 1, 1), (231, "TANH", 1, 1), (232, "ASINH", 1, 1), (233, "ACOSH", 1, 1),
    (234, "ATANH", 1, 1), (235, "DGET", 3, 3), (244, "INFO", 1, 1), (247, "DB", 4, 5),
    (252, "FREQUENCY", 2, 2), (261, "ERROR.TYPE", 1, 1),
    (269, "AVEDEV", 1, 30), (270, "BETADIST", 3, 5), (271, "GAMMALN", 1, 1),
    (272, "BETAINV", 3, 5), (273, "BINOMDIST", 4, 4), (274, "CHIDIST", 2, 2),
    (275, "CHIINV", 2, 2), (276, "COMBIN", 2, 2), (277, "CONFIDENCE", 3, 3),
    (278, "CRITBINOM", 3, 3), (279, "EVEN", 1, 1), (280, "EXPONDIST", 3, 3),
    (281, "FDIST", 3, 3), (282, "FINV", 3, 3), (283, "FISHER", 1, 1), (284, "FISHERINV", 1, 1),
    (285, "FLOOR", 2, 2), (286, "GAMMADIST", 4, 4), (287, "GAMMAINV", 3, 3),
    (288, "CEILING", 2, 2), (289, "HYPGEOMDIST", 4, 4), (290, "LOGNORMDIST", 3, 3),
    (291, "LOGINV", 3, 3), (292, "NEGBINOMDIST", 3, 3), (293, "NORMDIST", 4, 4),
    (294, "NORMSDIST", 1, 1), (295, "NORMINV", 3, 3), (296, "NORMSINV", 1, 1),
    (297, "STANDARDIZE", 3, 3), (298, "ODD", 1, 1), (299, "PERMUT", 2, 2),
    (300, "POISSON", 3, 3), (301, "TDIST", 3, 3), (302, "WEIBULL", 4, 4),
    (303, "SUMXMY2", 2, 2), (304, "SUMX2MY2", 2, 2), (305, "SUMX2PY2", 2, 2),
    (306, "CHITEST", 2, 2), (307, "CORREL", 2, 2), (308, "COVAR", 2, 2),
    (309, "FORECAST", 3, 3), (310, "FTEST", 2, 2), (311, "INTERCEPT", 2, 2),
    (312, "PEARSON", 2, 2), (313, "RSQ", 2, 2), (314, "STEYX", 2, 2), (315, "SLOPE", 2, 2),
    (316, "TTEST", 4, 4), (317, "PROB", 3, 4), (318, "DEVSQ", 1, 30), (319, "GEOMEAN", 1, 30),
    (320, "HARMEAN", 1, 30), (321, "SUMSQ", 0, 30), (322, "KURT", 1, 30), (323, "SKEW", 1, 30),
    (324, "ZTEST", 2, 3), (325, "LARGE", 2, 2), (326, "SMALL", 2, 2), (327, "QUARTILE", 2, 2),
    (328, "PERCENTILE", 2, 2), (329, "PERCENTRANK", 2, 3), (330, "MODE", 1, 30),
    (331, "TRIMMEAN", 2, 2), (332, "TINV", 2, 2), (336, "CONCATENATE", 0, 30),
    (337, "POWER", 2, 2), (342, "RADIANS", 1, 1), (343, "DEGREES", 1, 1),
    (344, "SUBTOTAL", 2, 30), (345, "SUMIF", 2, 3), (346, "COUNTIF", 2, 2),
    (347, "COUNTBLANK", 1, 1), (350, "ISPMT", 4, 4), (351, "DATEDIF", 3, 3),
    (352, "DATESTRING", 1, 1), (353, "NUMBERSTRING", 2, 2), (354, "ROMAN", 1, 2),
    (358, "GETPIVOTDATA", 2, 30), (359, "HYPERLINK", 1, 2), (360, "PHONETIC", 1, 1),
    (361, "AVERAGEA", 1, 30), (362, "MAXA", 1, 30), (363, "MINA", 1, 30),
    (364, "STDEVPA", 1, 30), (365, "VARPA", 1, 30), (366, "STDEVA", 1, 30),
    (367, "VARA", 1, 30), (368, "BAHTTEXT", 1, 1), (369, "THAIDAYOFWEEK", 1, 1),
    (370, "THAIDIGIT", 1, 1), (371, "THAIMONTHOFYEAR", 1, 1), (372, "THAINUMSOUND", 1, 1),
    (373, "THAINUMSTRING", 1, 1), (374, "THAISTRINGLENGTH", 1, 1), (375, "ISTHAIDIGIT", 1, 1),
    (376, "ROUNDBAHTDOWN", 1, 1), (377, "ROUNDBAHTUP", 1, 1), (378, "THAIYEAR", 1, 1),
    (379, "RTD", 3, 30),
];

/// The functions of Excel 97-2003's add-ins (the Analysis ToolPak and the
/// Euro tool), which a formula calls by name through an `EXTERNNAME` of
/// the add-in `SUPBOOK`: the rows marked `FUNCFLAG_EXTERNAL` (and
/// `FUNCLIB_EUROTOOL`) of `formulabase.cxx`. Sorted.
#[rustfmt::skip]
const ADDINS: &[&str] = &[
    "ACCRINT", "ACCRINTM", "AMORDEGRC", "AMORLINC", "BESSELI", "BESSELJ", "BESSELK", "BESSELY",
    "BIN2DEC", "BIN2HEX", "BIN2OCT", "COMPLEX", "CONVERT", "COUPDAYBS", "COUPDAYS",
    "COUPDAYSNC", "COUPNCD", "COUPNUM", "COUPPCD", "CUMIPMT", "CUMPRINC", "DEC2BIN", "DEC2HEX",
    "DEC2OCT", "DELTA", "DISC", "DOLLARDE", "DOLLARFR", "DURATION", "EDATE", "EFFECT",
    "EOMONTH", "ERF", "ERFC", "EUROCONVERT", "FACTDOUBLE", "FVSCHEDULE", "GCD", "GESTEP",
    "HEX2BIN", "HEX2DEC", "HEX2OCT", "IMABS", "IMAGINARY", "IMARGUMENT", "IMCONJUGATE", "IMCOS",
    "IMDIV", "IMEXP", "IMLN", "IMLOG10", "IMLOG2", "IMPOWER", "IMPRODUCT", "IMREAL", "IMSIN",
    "IMSQRT", "IMSUB", "IMSUM", "INTRATE", "ISEVEN", "ISODD", "LCM", "MDURATION", "MROUND",
    "MULTINOMIAL", "NETWORKDAYS", "NOMINAL", "OCT2BIN", "OCT2DEC", "OCT2HEX", "ODDFPRICE",
    "ODDFYIELD", "ODDLPRICE", "ODDLYIELD", "PRICE", "PRICEDISC", "PRICEMAT", "QUOTIENT",
    "RANDBETWEEN", "RECEIVED", "SERIESSUM", "SQRTPI", "TBILLEQ", "TBILLPRICE", "TBILLYIELD",
    "WEEKNUM", "WORKDAY", "XIRR", "XNPV", "YEARFRAC", "YIELD", "YIELDDISC", "YIELDMAT",
];

/// The functions Excel 2007 added that an `.xlsx` formula spells without
/// a prefix and an `.xls` file calls by name as `_xlfn.…`
/// (`saFuncTableOox` of `formulabase.cxx`). Later functions have the
/// prefix in an `.xlsx` too, and the XLSX reader's own table says which
/// those are. Sorted.
const FUTURE: &[&str] = &[
    "AVERAGEIF",
    "AVERAGEIFS",
    "COUNTIFS",
    "CUBEKPIMEMBER",
    "CUBEMEMBER",
    "CUBEMEMBERPROPERTY",
    "CUBERANKEDMEMBER",
    "CUBESET",
    "CUBESETCOUNT",
    "CUBEVALUE",
    "IFERROR",
    "SUMIFS",
];

/// Whether `name` (as the file spells it: upper case) is a function of
/// Excel 97-2003's add-ins.
pub(crate) fn is_addin(name: &str) -> bool {
    ADDINS.binary_search(&name).is_ok()
}

/// Whether `name` (upper case, without `_xlfn.`) is a function Excel
/// 2007 added that an `.xlsx` spells without the prefix.
pub(crate) fn is_future(name: &str) -> bool {
    FUTURE.binary_search(&name).is_ok()
}

/// Whether `name` is the Excel name of a built-in function of the table.
#[cfg(test)]
fn is_builtin(name: &str) -> bool {
    FTAB.iter().any(|f| f.1 == name)
}

/// The function with the index `iftab`: its Excel name and its fewest and
/// most arguments.
pub(crate) fn function(iftab: u16) -> Option<(&'static str, u8, u8)> {
    let i = FTAB.binary_search_by_key(&iftab, |f| f.0).ok()?;
    FTAB.get(i).map(|f| (f.1, f.2, f.3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_sane() {
        assert!(FTAB.windows(2).all(|w| w[0].0 < w[1].0));
        for (i, name, min, max) in FTAB {
            assert!(min <= max && *max <= 30, "{i} {name}");
            assert!(
                name.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'.'),
                "{name}"
            );
        }
        assert_eq!(function(0), Some(("COUNT", 0, 30)));
        assert_eq!(function(1), Some(("IF", 2, 3)));
        assert_eq!(function(4), Some(("SUM", 0, 30)));
        assert_eq!(function(24), Some(("ABS", 1, 1)));
        assert_eq!(function(27), Some(("ROUND", 2, 2)));
        assert_eq!(function(74), Some(("NOW", 0, 0)));
        assert_eq!(function(100), Some(("CHOOSE", 2, 30)));
        assert_eq!(function(345), Some(("SUMIF", 2, 3)));
        assert_eq!(function(379), Some(("RTD", 3, 30)));
        assert_eq!(function(204), Some(("DOLLAR", 1, 2)));
        assert_eq!(function(261), Some(("ERROR.TYPE", 1, 1)));
        // Macro-sheet functions, the call by name, indices past the table.
        for i in [53u16, 54, 55, 151, 255, 380, 0x7FFF, 0xFFFF] {
            assert_eq!(function(i), None, "{i}");
        }
    }

    #[test]
    fn the_functions_called_by_name() {
        for list in [ADDINS, FUTURE] {
            assert!(list.windows(2).all(|w| w[0] < w[1]));
            // None of them is a built-in function with an index.
            for name in list {
                assert!(!is_builtin(name), "{name}");
            }
        }
        assert!(is_addin("EDATE") && is_addin("GCD") && is_addin("WEEKNUM"));
        assert!(is_future("IFERROR") && is_future("SUMIFS"));
        for name in ["SUM", "IF", "TRUE", "PI", "N", "edate", "EASTERSUNDAY", ""] {
            assert!(!is_addin(name) && !is_future(name), "{name}");
        }
        assert!(is_builtin("SUM") && is_builtin("DOLLAR") && !is_builtin("EDATE"));
    }
}
