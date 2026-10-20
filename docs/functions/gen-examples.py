#!/usr/bin/env python3
"""Generate crates/tally-functions/src/categories/examples.rs.

Run from anywhere:  python3 -I docs/functions/gen-examples.py [--check]

One example call per function for the Function Wizard (`FunctionDef.example`,
exposed as `FunctionMeta.example`). The names, arities and parameter types
come from docs/calc-functions.json plus the extensions in
docs/functions/master-list.toml; the calls come from the curated EXAMPLES
table below. A name missing from the table gets a type-driven fallback
(and a warning), so a newly registered function is never without an
example; the Rust test `examples_cover_the_registry` fails on stale or
unknown names, and this script fails on an argument count outside the
function's arity.

--check: exit 1 if the generated file differs from the one on disk.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "docs" / "calc-functions.json"
MASTER = ROOT / "docs" / "functions" / "master-list.toml"
OUT = ROOT / "crates" / "tally-functions" / "src" / "categories" / "examples.rs"

# Conventions: A1:A10 and B1:B10 hold numbers, C1:C10 text, dates are built
# with DATE() or read from A1/B1 where the function is date-only.
# en-US UI dialect, `;` separators, `"` strings.
EXAMPLES = {
    # ---- Database
    "DCOUNT": '=DCOUNT(A1:E10;"Age";G1:H2)',
    "DCOUNTA": '=DCOUNTA(A1:E10;"Name";G1:H2)',
    "DAVERAGE": '=DAVERAGE(A1:E10;"Salary";G1:H2)',
    "DGET": '=DGET(A1:E10;"Name";G1:H2)',
    "DMAX": '=DMAX(A1:E10;"Salary";G1:H2)',
    "DMIN": '=DMIN(A1:E10;"Salary";G1:H2)',
    "DPRODUCT": '=DPRODUCT(A1:E10;"Factor";G1:H2)',
    "DSTDEV": '=DSTDEV(A1:E10;"Salary";G1:H2)',
    "DSTDEVP": '=DSTDEVP(A1:E10;"Salary";G1:H2)',
    "DSUM": '=DSUM(A1:E10;"Salary";G1:H2)',
    "DVAR": '=DVAR(A1:E10;"Salary";G1:H2)',
    "DVARP": '=DVARP(A1:E10;"Salary";G1:H2)',
    # ---- Date & time
    "DATE": "=DATE(2026;10;20)",
    "DATEVALUE": '=DATEVALUE("2026-10-20")',
    "DAY": "=DAY(A1)",
    "DAYS360": "=DAYS360(A1;B1)",
    "HOUR": "=HOUR(A1)",
    "MINUTE": "=MINUTE(A1)",
    "MONTH": "=MONTH(A1)",
    "NOW": "=NOW()",
    "SECOND": "=SECOND(A1)",
    "TIME": "=TIME(14;30;0)",
    "TIMEVALUE": '=TIMEVALUE("14:30")',
    "TODAY": "=TODAY()",
    "WEEKDAY": "=WEEKDAY(A1;2)",
    "YEAR": "=YEAR(A1)",
    "DAYS": "=DAYS(B1;A1)",
    "WEEKNUM": "=WEEKNUM(A1;2)",
    "EASTERSUNDAY": "=EASTERSUNDAY(2026)",
    "ISOWEEKNUM": "=ISOWEEKNUM(A1)",
    # ---- Financial
    "PV": "=PV(6%/12;120;-500)",
    "FV": "=FV(5%/12;60;-200)",
    "NPER": "=NPER(6%/12;-300;10000)",
    "PMT": "=PMT(5%/12;360;200000)",
    "RATE": "=RATE(60;-250;12000)",
    "IPMT": "=IPMT(5%/12;1;360;200000)",
    "PPMT": "=PPMT(5%/12;1;360;200000)",
    "CUMPRINC": "=CUMPRINC(5%/12;360;200000;1;12;0)",
    "CUMIPMT": "=CUMIPMT(5%/12;360;200000;1;12;0)",
    "SYD": "=SYD(30000;7500;10;1)",
    "SLN": "=SLN(30000;7500;10)",
    "DDB": "=DDB(30000;7500;10;1)",
    "DB": "=DB(30000;7500;10;1)",
    "VDB": "=VDB(30000;7500;10;0;1)",
    "EFFECTIVE": "=EFFECTIVE(5%;12)",
    "NOMINAL": "=NOMINAL(5.12%;12)",
    "NPV": "=NPV(8%;A1:A5)",
    "IRR": "=IRR(A1:A5)",
    "MIRR": "=MIRR(A1:A5;8%;10%)",
    "ISPMT": "=ISPMT(5%/12;1;360;200000)",
    "DURATION": "=DURATION(5%;1000;2000)",
    "ZGZ": "=ZGZ(10;1000;2000)",
    # ---- Information
    "ISREF": "=ISREF(A1)",
    "ISERR": "=ISERR(A1)",
    "ISERROR": "=ISERROR(A1)",
    "ISBLANK": "=ISBLANK(A1)",
    "ISLOGICAL": "=ISLOGICAL(A1)",
    "ISNA": "=ISNA(A1)",
    "ISNONTEXT": "=ISNONTEXT(A1)",
    "ISTEXT": "=ISTEXT(A1)",
    "ISNUMBER": "=ISNUMBER(A1)",
    "ISFORMULA": "=ISFORMULA(A1)",
    "FORMULA": "=FORMULA(A1)",
    "N": "=N(A1)",
    "NA": "=NA()",
    "TYPE": "=TYPE(A1)",
    "CELL": '=CELL("ADDRESS";A1)',
    "CURRENT": "=CURRENT()",
    "INFO": '=INFO("system")',
    # ---- Logical
    "FALSE": "=FALSE()",
    "NOT": "=NOT(A1>5)",
    "TRUE": "=TRUE()",
    "IF": '=IF(A1>5;"big";"small")',
    "OR": "=OR(A1>5;B1>5)",
    "AND": "=AND(A1>5;B1>5)",
    "XOR": "=XOR(A1>5;B1>5)",
    "IFERROR": "=IFERROR(A1/B1;0)",
    "IFNA": '=IFNA(VLOOKUP("Pear";A1:B10;2;0);"missing")',
    # ---- Mathematical
    "ABS": "=ABS(-3)",
    "POWER": "=POWER(2;10)",
    "COUNTBLANK": "=COUNTBLANK(A1:A10)",
    "PI": "=PI()",
    "SUM": "=SUM(A1:A10)",
    "SUMSQ": "=SUMSQ(A1:A10)",
    "PRODUCT": "=PRODUCT(A1:A5)",
    "SUMIF": '=SUMIF(A1:A10;">5";B1:B10)',
    "AVERAGEIF": '=AVERAGEIF(A1:A10;">5";B1:B10)',
    "SUMIFS": '=SUMIFS(C1:C10;A1:A10;">5";B1:B10;"<10")',
    "AVERAGEIFS": '=AVERAGEIFS(C1:C10;A1:A10;">5";B1:B10;"<10")',
    "COUNTIFS": '=COUNTIFS(A1:A10;">5";B1:B10;"<10")',
    "COUNTIF": '=COUNTIF(A1:A10;">5")',
    "SQRT": "=SQRT(16)",
    "RAND": "=RAND()",
    "ISEVEN": "=ISEVEN(A1)",
    "ISODD": "=ISODD(A1)",
    "COMBIN": "=COMBIN(5;2)",
    "COMBINA": "=COMBINA(5;2)",
    "ACOS": "=ACOS(0.5)",
    "ASIN": "=ASIN(0.5)",
    "ACOSH": "=ACOSH(2)",
    "ASINH": "=ASINH(1)",
    "ACOT": "=ACOT(1)",
    "ATAN": "=ATAN(1)",
    "ACOTH": "=ACOTH(2)",
    "ATANH": "=ATANH(0.5)",
    "COS": "=COS(PI())",
    "SIN": "=SIN(PI()/2)",
    "COT": "=COT(PI()/4)",
    "TAN": "=TAN(PI()/4)",
    "COSH": "=COSH(1)",
    "SINH": "=SINH(1)",
    "COTH": "=COTH(1)",
    "TANH": "=TANH(1)",
    "ATAN2": "=ATAN2(1;1)",
    "CSC": "=CSC(PI()/2)",
    "SEC": "=SEC(0)",
    "CSCH": "=CSCH(1)",
    "SECH": "=SECH(1)",
    "DEGREES": "=DEGREES(PI())",
    "RADIANS": "=RADIANS(180)",
    "EXP": "=EXP(1)",
    "LOG": "=LOG(100;10)",
    "LN": "=LN(EXP(1))",
    "LOG10": "=LOG10(1000)",
    "FACT": "=FACT(5)",
    "MOD": "=MOD(10;3)",
    "SIGN": "=SIGN(-5)",
    "SUBTOTAL": "=SUBTOTAL(9;A1:A10)",
    "INT": "=INT(3.7)",
    "TRUNC": "=TRUNC(3.789;2)",
    "ROUND": "=ROUND(3.14159;2)",
    "ROUNDUP": "=ROUNDUP(3.14159;2)",
    "ROUNDDOWN": "=ROUNDDOWN(3.14159;2)",
    "EVEN": "=EVEN(3)",
    "ODD": "=ODD(4)",
    "CEILING": "=CEILING(4.3;0.5)",
    "FLOOR": "=FLOOR(4.7;0.5)",
    "GCD": "=GCD(12;18)",
    "LCM": "=LCM(4;6)",
    # ---- Array
    "TRANSPOSE": "=TRANSPOSE(A1:C3)",
    "MMULT": "=MMULT(A1:B2;D1:E2)",
    "MDETERM": "=MDETERM(A1:C3)",
    "MINVERSE": "=MINVERSE(A1:C3)",
    "MUNIT": "=MUNIT(3)",
    "SUMPRODUCT": "=SUMPRODUCT(A1:A10;B1:B10)",
    "SUMX2MY2": "=SUMX2MY2(A1:A10;B1:B10)",
    "SUMX2PY2": "=SUMX2PY2(A1:A10;B1:B10)",
    "SUMXMY2": "=SUMXMY2(A1:A10;B1:B10)",
    "FREQUENCY": "=FREQUENCY(A1:A20;C1:C5)",
    "LINEST": "=LINEST(B1:B10;A1:A10)",
    "LOGEST": "=LOGEST(B1:B10;A1:A10)",
    "TREND": "=TREND(B1:B10;A1:A10;A11:A12)",
    "GROWTH": "=GROWTH(B1:B10;A1:A10;A11:A12)",
    # ---- Statistical
    "COUNT": "=COUNT(A1:A10)",
    "COUNTA": "=COUNTA(A1:A10)",
    "MAX": "=MAX(A1:A10)",
    "MAXA": "=MAXA(A1:A10)",
    "MIN": "=MIN(A1:A10)",
    "MINA": "=MINA(A1:A10)",
    "VAR": "=VAR(A1:A10)",
    "VARA": "=VARA(A1:A10)",
    "VARP": "=VARP(A1:A10)",
    "VARPA": "=VARPA(A1:A10)",
    "STDEV": "=STDEV(A1:A10)",
    "STDEVA": "=STDEVA(A1:A10)",
    "STDEVP": "=STDEVP(A1:A10)",
    "STDEVPA": "=STDEVPA(A1:A10)",
    "AVERAGE": "=AVERAGE(A1:A10)",
    "AVERAGEA": "=AVERAGEA(A1:A10)",
    "DEVSQ": "=DEVSQ(A1:A10)",
    "AVEDEV": "=AVEDEV(A1:A10)",
    "SKEW": "=SKEW(A1:A10)",
    "KURT": "=KURT(A1:A10)",
    "GEOMEAN": "=GEOMEAN(A1:A10)",
    "HARMEAN": "=HARMEAN(A1:A10)",
    "MODE": "=MODE(A1:A10)",
    "MEDIAN": "=MEDIAN(A1:A10)",
    "PERCENTILE": "=PERCENTILE(A1:A10;0.9)",
    "QUARTILE": "=QUARTILE(A1:A10;3)",
    "LARGE": "=LARGE(A1:A10;2)",
    "SMALL": "=SMALL(A1:A10;2)",
    "PERCENTRANK": "=PERCENTRANK(A1:A10;A2)",
    "RANK": "=RANK(A2;A1:A10)",
    "TRIMMEAN": "=TRIMMEAN(A1:A10;0.2)",
    "PROB": "=PROB(A1:A10;B1:B10;5;8)",
    "B": "=B(10;0.5;3)",
    "PHI": "=PHI(1)",
    "GAUSS": "=GAUSS(1)",
    "FISHER": "=FISHER(0.5)",
    "FISHERINV": "=FISHERINV(0.5)",
    "BINOMDIST": "=BINOMDIST(3;10;0.5;0)",
    "NEGBINOMDIST": "=NEGBINOMDIST(3;5;0.5)",
    "CRITBINOM": "=CRITBINOM(10;0.5;0.9)",
    "POISSON": "=POISSON(2;3;1)",
    "NORMDIST": "=NORMDIST(42;40;1.5;1)",
    "NORMINV": "=NORMINV(0.9;40;1.5)",
    "NORMSDIST": "=NORMSDIST(1)",
    "NORMSINV": "=NORMSINV(0.95)",
    "LOGNORMDIST": "=LOGNORMDIST(10;2;0.5)",
    "LOGINV": "=LOGINV(0.5;2;0.5)",
    "EXPONDIST": "=EXPONDIST(1;0.5;1)",
    "GAMMADIST": "=GAMMADIST(2;1;2;1)",
    "GAMMAINV": "=GAMMAINV(0.5;1;2)",
    "GAMMALN": "=GAMMALN(5)",
    "GAMMA": "=GAMMA(5)",
    "BETADIST": "=BETADIST(0.5;2;3)",
    "BETAINV": "=BETAINV(0.5;2;3)",
    "WEIBULL": "=WEIBULL(2;1;1;1)",
    "HYPGEOMDIST": "=HYPGEOMDIST(1;2;4;10)",
    "TDIST": "=TDIST(2;5;2)",
    "TINV": "=TINV(0.05;5)",
    "FDIST": "=FDIST(2;5;10)",
    "FINV": "=FINV(0.05;5;10)",
    "CHIDIST": "=CHIDIST(3;2)",
    "CHISQDIST": "=CHISQDIST(3;2;1)",
    "CHIINV": "=CHIINV(0.05;2)",
    "CHISQINV": "=CHISQINV(0.95;2)",
    "STANDARDIZE": "=STANDARDIZE(42;40;1.5)",
    "PERMUT": "=PERMUT(5;2)",
    "PERMUTATIONA": "=PERMUTATIONA(5;2)",
    "CONFIDENCE": "=CONFIDENCE(0.05;1.5;100)",
    "ZTEST": "=ZTEST(A1:A10;40)",
    "CHITEST": "=CHITEST(A1:B3;D1:E3)",
    "FTEST": "=FTEST(A1:A10;B1:B10)",
    "TTEST": "=TTEST(A1:A10;B1:B10;2;1)",
    "RSQ": "=RSQ(B1:B10;A1:A10)",
    "INTERCEPT": "=INTERCEPT(B1:B10;A1:A10)",
    "SLOPE": "=SLOPE(B1:B10;A1:A10)",
    "STEYX": "=STEYX(B1:B10;A1:A10)",
    "PEARSON": "=PEARSON(A1:A10;B1:B10)",
    "CORREL": "=CORREL(A1:A10;B1:B10)",
    "COVAR": "=COVAR(A1:A10;B1:B10)",
    "FORECAST": "=FORECAST(11;B1:B10;A1:A10)",
    # ---- Spreadsheet
    "ADDRESS": "=ADDRESS(1;1)",
    "AREAS": "=AREAS(A1:B2)",
    "CHOOSE": '=CHOOSE(2;"red";"green";"blue")',
    "COLUMN": "=COLUMN(B1)",
    "ROW": "=ROW(A5)",
    "SHEET": "=SHEET(A1)",
    "COLUMNS": "=COLUMNS(A1:C3)",
    "ROWS": "=ROWS(A1:C3)",
    "SHEETS": "=SHEETS()",
    "HLOOKUP": '=HLOOKUP("Total";A1:E5;3;0)',
    "VLOOKUP": '=VLOOKUP("Apple";A1:C10;2;0)',
    "INDEX": "=INDEX(A1:C10;2;3)",
    "INDIRECT": '=INDIRECT("A"&ROW())',
    "LOOKUP": "=LOOKUP(5;A1:A10;B1:B10)",
    "MATCH": "=MATCH(42;A1:A10;0)",
    "OFFSET": "=OFFSET(A1;2;1;3;1)",
    "ERRORTYPE": "=ERRORTYPE(A1)",
    "STYLE": '=STYLE("Good")',
    "DDE": '=DDE("soffice";"report.ods";"Sheet1.A1")',
    "HYPERLINK": '=HYPERLINK("https://example.org";"Website")',
    "GETPIVOTDATA": '=GETPIVOTDATA("Sales";A1;"Region";"North")',
    "MULTIPLE.OPERATIONS": "=MULTIPLE.OPERATIONS($B$5;$B$1;A8)",
    # ---- Text
    "BAHTTEXT": "=BAHTTEXT(1234.5)",
    "JIS": '=JIS("abc")',
    "ASC": '=ASC("ＡＢＣ")',
    "CODE": '=CODE("A")',
    "DOLLAR": "=DOLLAR(1234.5;2)",
    "CHAR": "=CHAR(65)",
    "CLEAN": "=CLEAN(A1)",
    "CONCATENATE": '=CONCATENATE("Hello";" ";"world")',
    "EXACT": '=EXACT("a";"A")',
    "FIND": '=FIND("b";"abc")',
    "SEARCH": '=SEARCH("B";"abc")',
    "TRIM": '=TRIM("  a  b  ")',
    "PROPER": '=PROPER("hello world")',
    "UPPER": '=UPPER("hello")',
    "LOWER": '=LOWER("HELLO")',
    "VALUE": '=VALUE("42")',
    "TEXT": '=TEXT(1234.5;"0.00")',
    "T": "=T(A1)",
    "REPLACE": '=REPLACE("abcdef";2;3;"X")',
    "FIXED": "=FIXED(1234.567;2)",
    "LEN": '=LEN("abc")',
    "LEFT": '=LEFT("abcdef";3)',
    "RIGHT": '=RIGHT("abcdef";3)',
    "MID": '=MID("abcdef";2;3)',
    "REPT": '=REPT("-";10)',
    "SUBSTITUTE": '=SUBSTITUTE("a-b-c";"-";"+")',
    "BASE": "=BASE(255;16)",
    "DECIMAL": '=DECIMAL("FF";16)',
    "CONVERT": '=CONVERT(100;"EUR";"DEM")',
    "ROMAN": "=ROMAN(1999)",
    "ARABIC": '=ARABIC("MCMXCIX")',
    "UNICODE": '=UNICODE("€")',
    "UNICHAR": "=UNICHAR(8364)",
    "EUROCONVERT": '=EUROCONVERT(100;"DEM";"EUR")',
    "NUMBERVALUE": '=NUMBERVALUE("1.234,5";",";".")',
    "LENB": '=LENB("abc")',
    "RIGHTB": '=RIGHTB("abcdef";3)',
    "LEFTB": '=LEFTB("abcdef";3)',
    "MIDB": '=MIDB("abcdef";2;3)',
    "BITAND": "=BITAND(12;10)",
    "BITOR": "=BITOR(12;10)",
    "BITXOR": "=BITXOR(12;10)",
    "BITLSHIFT": "=BITLSHIFT(1;4)",
    "BITRSHIFT": "=BITRSHIFT(16;2)",
    # ---- Add-in: date
    "WORKDAY": "=WORKDAY(A1;10)",
    "YEARFRAC": "=YEARFRAC(A1;B1;1)",
    "EDATE": "=EDATE(A1;3)",
    "WEEKNUM_ADD": "=WEEKNUM_ADD(A1;2)",
    "EOMONTH": "=EOMONTH(A1;1)",
    "NETWORKDAYS": "=NETWORKDAYS(A1;B1)",
    "WEEKS": "=WEEKS(A1;B1;0)",
    "MONTHS": "=MONTHS(A1;B1;0)",
    "YEARS": "=YEARS(A1;B1;0)",
    "ISLEAPYEAR": "=ISLEAPYEAR(A1)",
    "DAYSINMONTH": "=DAYSINMONTH(A1)",
    "DAYSINYEAR": "=DAYSINYEAR(A1)",
    "WEEKSINYEAR": "=WEEKSINYEAR(A1)",
    "ROT13": '=ROT13("Hello")',
    # ---- Add-in: math / engineering
    "ISEVEN_ADD": "=ISEVEN_ADD(4)",
    "ISODD_ADD": "=ISODD_ADD(3)",
    "MULTINOMIAL": "=MULTINOMIAL(2;3;4)",
    "SERIESSUM": "=SERIESSUM(2;1;1;A1:A5)",
    "QUOTIENT": "=QUOTIENT(17;3)",
    "MROUND": "=MROUND(17;5)",
    "SQRTPI": "=SQRTPI(2)",
    "RANDBETWEEN": "=RANDBETWEEN(1;6)",
    "GCD_ADD": "=GCD_ADD(12;18)",
    "LCM_ADD": "=LCM_ADD(4;6)",
    "BESSELI": "=BESSELI(1;1)",
    "BESSELJ": "=BESSELJ(1;1)",
    "BESSELK": "=BESSELK(1;1)",
    "BESSELY": "=BESSELY(1;1)",
    "BIN2OCT": '=BIN2OCT("1010")',
    "BIN2DEC": '=BIN2DEC("1010")',
    "BIN2HEX": '=BIN2HEX("1010")',
    "OCT2BIN": '=OCT2BIN("17")',
    "OCT2DEC": '=OCT2DEC("17")',
    "OCT2HEX": '=OCT2HEX("17")',
    "DEC2BIN": "=DEC2BIN(10)",
    "DEC2HEX": "=DEC2HEX(255)",
    "DEC2OCT": "=DEC2OCT(8)",
    "HEX2BIN": '=HEX2BIN("A")',
    "HEX2DEC": '=HEX2DEC("FF")',
    "HEX2OCT": '=HEX2OCT("FF")',
    "DELTA": "=DELTA(5;5)",
    "ERF": "=ERF(1)",
    "ERFC": "=ERFC(1)",
    "GESTEP": "=GESTEP(5;3)",
    "FACTDOUBLE": "=FACTDOUBLE(7)",
    "IMABS": '=IMABS("3+4i")',
    "IMAGINARY": '=IMAGINARY("3+4i")',
    "IMPOWER": '=IMPOWER("1+i";2)',
    "IMARGUMENT": '=IMARGUMENT("3+4i")',
    "IMCOS": '=IMCOS("1+i")',
    "IMDIV": '=IMDIV("1+i";"1-i")',
    "IMEXP": '=IMEXP("1+i")',
    "IMCONJUGATE": '=IMCONJUGATE("3+4i")',
    "IMLN": '=IMLN("1+i")',
    "IMLOG10": '=IMLOG10("1+i")',
    "IMLOG2": '=IMLOG2("1+i")',
    "IMPRODUCT": '=IMPRODUCT("1+i";"2-i")',
    "IMREAL": '=IMREAL("3+4i")',
    "IMSIN": '=IMSIN("1+i")',
    "IMSUB": '=IMSUB("3+4i";"1+i")',
    "IMSQRT": '=IMSQRT("3+4i")',
    "IMSUM": '=IMSUM("1+i";"2-i")',
    "IMTAN": '=IMTAN("1+i")',
    "IMSEC": '=IMSEC("1+i")',
    "IMCSC": '=IMCSC("1+i")',
    "IMCOT": '=IMCOT("1+i")',
    "IMSINH": '=IMSINH("1+i")',
    "IMCOSH": '=IMCOSH("1+i")',
    "IMSECH": '=IMSECH("1+i")',
    "IMCSCH": '=IMCSCH("1+i")',
    "COMPLEX": "=COMPLEX(3;4)",
    "CONVERT_ADD": '=CONVERT_ADD(1;"km";"mi")',
    # ---- Add-in: financial
    "AMORDEGRC": "=AMORDEGRC(2400;A1;B1;300;1;0.15;1)",
    "AMORLINC": "=AMORLINC(2400;A1;B1;300;1;0.15;1)",
    "ACCRINT": "=ACCRINT(A1;B1;C1;5%;1000;2;0)",
    "ACCRINTM": "=ACCRINTM(A1;B1;5%;1000;0)",
    "RECEIVED": "=RECEIVED(A1;B1;1000;5%;0)",
    "DISC": "=DISC(A1;B1;97;100;0)",
    "DURATION_ADD": "=DURATION_ADD(A1;B1;5%;6%;2;0)",
    "EFFECT_ADD": "=EFFECT_ADD(5%;12)",
    "CUMPRINC_ADD": "=CUMPRINC_ADD(5%/12;360;200000;1;12;0)",
    "CUMIPMT_ADD": "=CUMIPMT_ADD(5%/12;360;200000;1;12;0)",
    "PRICE": "=PRICE(A1;B1;5%;6%;100;2;0)",
    "PRICEDISC": "=PRICEDISC(A1;B1;3%;100;0)",
    "PRICEMAT": "=PRICEMAT(A1;B1;C1;5%;6%;0)",
    "MDURATION": "=MDURATION(A1;B1;5%;6%;2;0)",
    "NOMINAL_ADD": "=NOMINAL_ADD(5.12%;12)",
    "DOLLARFR": "=DOLLARFR(1.125;16)",
    "DOLLARDE": "=DOLLARDE(1.02;16)",
    "YIELD": "=YIELD(A1;B1;5%;95;100;2;0)",
    "YIELDDISC": "=YIELDDISC(A1;B1;95;100;0)",
    "YIELDMAT": "=YIELDMAT(A1;B1;C1;5%;95;0)",
    "TBILLEQ": "=TBILLEQ(A1;B1;3%)",
    "TBILLPRICE": "=TBILLPRICE(A1;B1;3%)",
    "TBILLYIELD": "=TBILLYIELD(A1;B1;98)",
    "ODDFPRICE": "=ODDFPRICE(A1;B1;C1;D1;5%;6%;100;2;0)",
    "ODDFYIELD": "=ODDFYIELD(A1;B1;C1;D1;5%;95;100;2;0)",
    "ODDLPRICE": "=ODDLPRICE(A1;B1;C1;5%;6%;100;2;0)",
    "ODDLYIELD": "=ODDLYIELD(A1;B1;C1;5%;95;100;2;0)",
    "XIRR": "=XIRR(A1:A5;B1:B5)",
    "XNPV": "=XNPV(8%;A1:A5;B1:B5)",
    "INTRATE": "=INTRATE(A1;B1;1000;1050;0)",
    "COUPNCD": "=COUPNCD(A1;B1;2;0)",
    "COUPDAYS": "=COUPDAYS(A1;B1;2;0)",
    "COUPDAYSNC": "=COUPDAYSNC(A1;B1;2;0)",
    "COUPDAYBS": "=COUPDAYBS(A1;B1;2;0)",
    "COUPPCD": "=COUPPCD(A1;B1;2;0)",
    "COUPNUM": "=COUPNUM(A1;B1;2;0)",
    "FVSCHEDULE": "=FVSCHEDULE(1000;A1:A5)",
}

# Fallback argument per JSON parameter type, for names not in EXAMPLES.
FALLBACK_ARG = {
    "number": "2", "integer": "2", "date": "A1", "time": "A1",
    "logical": "TRUE()", "text": '"text"', "any": "A1", "criteria": '">5"',
    "range": "A1:A10", "reference": "A1", "array": "A1:B2",
    "numberList": "A1:A10", "logicalList": "A1:A10", "textList": "C1:C10",
    "valueList": "A1:A10",
}


def load_functions():
    """name -> (minArgs, maxArgs, params) for the JSON and the extensions."""
    data = json.loads(SRC.read_text(encoding="utf-8"))
    funcs = {e["name"]: (e["minArgs"], e["maxArgs"], e["params"]) for e in data}
    text = MASTER.read_text(encoding="utf-8")
    for block in text.split("[[function]]")[1:]:
        name = re.search(r'^name = "([^"]+)"', block, re.M).group(1)
        lo = int(re.search(r"^min_args = (\d+)", block, re.M).group(1))
        hi = int(re.search(r"^max_args = (\d+)", block, re.M).group(1))
        if name in funcs:
            funcs[name] = (lo, hi, funcs[name][2])      # master list arity wins
        else:
            funcs[name] = (lo, hi, [])
    return funcs


def split_args(call, name):
    """Top-level `;`-separated arguments of `=NAME(...)`; None when malformed."""
    head = f"={name}("
    if not call.startswith(head) or not call.endswith(")"):
        return None
    body = call[len(head):-1]
    if body == "":
        return []
    args, depth, quoted, cur = [], 0, False, []
    for ch in body:
        if ch == '"':
            quoted = not quoted
        elif not quoted:
            if ch == "(":
                depth += 1
            elif ch == ")":
                depth -= 1
                if depth < 0:
                    return None
            elif ch == ";" and depth == 0:
                args.append("".join(cur))
                cur = []
                continue
        cur.append(ch)
    if quoted or depth != 0:
        return None
    args.append("".join(cur))
    return args


def fallback(name, lo, params):
    args = []
    for p in params:
        if len(args) >= lo and (p.get("optional") or p.get("variadic")):
            break
        args.append(FALLBACK_ARG.get(p.get("type", "any"), "A1"))
    while len(args) < lo:
        args.append("A1")
    return f"={name}({';'.join(args)})"


def rust_str(s):
    return 'r#"' + s + '"#' if '"' in s else '"' + s + '"'


def main(argv):
    funcs = load_functions()
    problems = []
    for name in EXAMPLES:
        if name not in funcs:
            problems.append(f"{name}: in EXAMPLES but not in the JSON / master list")
    rows = []
    for name, (lo, hi, params) in funcs.items():
        call = EXAMPLES.get(name)
        if call is None:
            call = fallback(name, lo, params)
            print(f"warning: {name}: no curated example, using {call}", file=sys.stderr)
        if '"#' in call:
            problems.append(f"{name}: example contains '\"#'")
        args = split_args(call, name)
        if args is None:
            problems.append(f"{name}: malformed example {call!r}")
            continue
        n = len(args)
        if n < lo or (hi != 255 and n > hi):
            problems.append(f"{name}: {n} argument(s) in {call!r}, arity is {lo}..{hi}")
        rows.append((name, call))
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1

    out = [
        "//! GENERATED by `python3 -I docs/functions/gen-examples.py` from\n"
        "//! `docs/calc-functions.json`, `docs/functions/master-list.toml` and the\n"
        "//! curated table in that script. Do not edit by hand.\n"
        "//!\n"
        "//! One example call per function for the Function Wizard\n"
        "//! (`FunctionDef::example`), applied after every category has registered.\n"
        "\n"
        "use crate::registry::Registry;\n"
        "\n"
        "/// `(canonical name, example call in the en-US UI dialect)`.\n"
        "pub(crate) const EXAMPLES: &[(&str, &str)] = &[\n"
    ]
    for name, call in rows:
        out.append(f"    ({rust_str(name)}, {rust_str(call)}),\n")
    out.append(
        "];\n"
        "\n"
        "/// Sets every function's example; an unknown name is recorded as a\n"
        "/// registry conflict (a stale table), which the registry tests report.\n"
        "pub(crate) fn apply(r: &mut Registry) {\n"
        "    for (name, example) in EXAMPLES {\n"
        "        r.set_example(name, example);\n"
        "    }\n"
        "}\n"
    )
    text = "".join(out)
    if "--check" in argv:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if current != text:
            print(f"{OUT} is stale; rerun {pathlib.Path(__file__).name}", file=sys.stderr)
            return 1
        print(f"{OUT.name}: up to date ({len(rows)} examples)")
        return 0
    OUT.write_text(text, encoding="utf-8")
    print(f"wrote {OUT} ({len(rows)} examples)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
